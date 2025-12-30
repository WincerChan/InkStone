use axum::body::Body;
use axum::extract::State;
use axum::http::header::{COOKIE, SET_COOKIE};
use axum::http::{Method, Request};
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use rand::rngs::OsRng;
use rand::RngCore;
use sha2::Sha256;
use thiserror::Error;

use crate::state::AppState;

const COOKIE_NAME: &str = "bid";
const TOKEN_BYTES: usize = 16;
const SIGNATURE_BYTES: usize = 16;
const COOKIE_MAX_AGE_SECS: i64 = 31_536_000;
const EXPECTED_TOKEN_LEN: usize = 16;

#[derive(Debug, Clone)]
pub struct ClientIds {
    pub interaction_id: Vec<u8>,
    pub stats_id: Vec<u8>,
}

#[derive(Debug, Error)]
pub enum BidCookieError {
    #[error("cookie secret not configured")]
    MissingCookieSecret,
    #[error("stats secret not configured")]
    MissingStatsSecret,
    #[error("bid cookie required")]
    CookieRequired,
}

pub async fn ensure_bid_cookie(
    State(state): State<AppState>,
    mut request: Request<Body>,
    next: Next,
) -> Result<Response, BidCookieError> {
    if !should_handle_cookie(&request) {
        return Ok(next.run(request).await);
    }
    let cookie_secret = state
        .config
        .cookie_secret
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(BidCookieError::MissingCookieSecret)?;
    let stats_secret = state
        .config
        .stats_secret
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(BidCookieError::MissingStatsSecret)?;

    let mut set_cookie = None;
    let (token_str, token_bytes) = match extract_cookie(&request, COOKIE_NAME)
        .and_then(|value| parse_cookie_value(&value))
        .and_then(|(token, sig)| verify_cookie(cookie_secret, &token, &sig).then_some(token))
        .and_then(|token| decode_token_bytes(&token).map(|bytes| (token, bytes)))
    {
        Some(value) => value,
        None => {
            if requires_cookie(&request) {
                return Err(BidCookieError::CookieRequired);
            }
            let token_bytes = generate_token_bytes();
            let token_str = encode_token(&token_bytes);
            let sig = sign_token(cookie_secret, &token_str);
            set_cookie = Some(build_cookie_value(&token_str, &sig));
            (token_str, token_bytes)
        }
    };

    let stats_id = build_stats_id(stats_secret, &token_str);
    request.extensions_mut().insert(ClientIds {
        interaction_id: token_bytes,
        stats_id,
    });

    let mut response = next.run(request).await;
    if let Some(cookie_value) = set_cookie {
        if let Ok(value) = cookie_value.parse() {
            response.headers_mut().append(SET_COOKIE, value);
        }
    }
    Ok(response)
}

fn should_handle_cookie(request: &Request<Body>) -> bool {
    let path = request.uri().path();
    path == "/v2/kudos" || path.starts_with("/v2/pulse/")
}

fn requires_cookie(request: &Request<Body>) -> bool {
    request.method() == Method::PUT && request.uri().path() == "/v2/kudos"
}

fn extract_cookie<B>(request: &Request<B>, name: &str) -> Option<String> {
    let header = request.headers().get(COOKIE)?.to_str().ok()?;
    for part in header.split(';') {
        let trimmed = part.trim();
        let mut iter = trimmed.splitn(2, '=');
        let key = iter.next()?.trim();
        let value = iter.next()?.trim();
        if key == name {
            return Some(value.to_string());
        }
    }
    None
}

fn parse_cookie_value(value: &str) -> Option<(String, String)> {
    let mut iter = value.splitn(2, '.');
    let token = iter.next()?.trim();
    let sig = iter.next()?.trim();
    if token.is_empty() || sig.is_empty() {
        return None;
    }
    Some((token.to_string(), sig.to_string()))
}

fn verify_cookie(secret: &str, token: &str, sig: &str) -> bool {
    if sig != sign_token(secret, token) {
        return false;
    }
    true
}

fn generate_token_bytes() -> Vec<u8> {
    let mut bytes = [0u8; TOKEN_BYTES];
    OsRng.fill_bytes(&mut bytes);
    bytes.to_vec()
}

fn sign_token(secret: &str, token: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("hmac can take key of any size");
    mac.update(token.as_bytes());
    let raw = mac.finalize().into_bytes();
    URL_SAFE_NO_PAD.encode(&raw[..SIGNATURE_BYTES])
}

fn build_stats_id(secret: &str, token: &str) -> Vec<u8> {
    let day = Utc::now().format("%Y%m%d").to_string();
    let payload = format!("{token}{day}");
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("hmac can take key of any size");
    mac.update(payload.as_bytes());
    let raw = mac.finalize().into_bytes();
    raw[..SIGNATURE_BYTES].to_vec()
}

fn decode_token_bytes(token: &str) -> Option<Vec<u8>> {
    let decoded = URL_SAFE_NO_PAD.decode(token.as_bytes()).ok()?;
    if decoded.len() != EXPECTED_TOKEN_LEN {
        return None;
    }
    Some(decoded)
}

fn encode_token(bytes: &[u8]) -> String {
    URL_SAFE_NO_PAD.encode(bytes)
}

fn build_cookie_value(token: &str, sig: &str) -> String {
    format!(
        "{name}={value}; Path=/; Secure; HttpOnly; SameSite=Lax; Max-Age={max_age}",
        name = COOKIE_NAME,
        value = format!("{token}.{sig}"),
        max_age = COOKIE_MAX_AGE_SECS
    )
}

impl IntoResponse for BidCookieError {
    fn into_response(self) -> Response {
        let status = match self {
            BidCookieError::CookieRequired => axum::http::StatusCode::UNAUTHORIZED,
            BidCookieError::MissingCookieSecret | BidCookieError::MissingStatsSecret => {
                axum::http::StatusCode::SERVICE_UNAVAILABLE
            }
        };
        (status, self.to_string()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use axum::http::Request;

    use super::{
        build_stats_id, decode_token_bytes, parse_cookie_value, requires_cookie,
        should_handle_cookie, sign_token, verify_cookie,
    };

    #[test]
    fn parse_cookie_value_splits_token_and_sig() {
        let (token, sig) = parse_cookie_value("token.sig").unwrap();
        assert_eq!(token, "token");
        assert_eq!(sig, "sig");
    }

    #[test]
    fn verify_cookie_accepts_signed_value() {
        let secret = "secret";
        let token = "token";
        let sig = sign_token(secret, token);
        assert!(verify_cookie(secret, token, &sig));
    }

    #[test]
    fn build_stats_id_varies_by_token() {
        let secret = "secret";
        let id_a = build_stats_id(secret, "token-a");
        let id_b = build_stats_id(secret, "token-b");
        assert_ne!(id_a, id_b);
    }

    #[test]
    fn decode_token_bytes_rejects_wrong_length() {
        let token = "short";
        assert!(decode_token_bytes(token).is_none());
    }

    #[test]
    fn requires_cookie_for_kudos_put() {
        let req = Request::builder()
            .method("PUT")
            .uri("/v2/kudos")
            .body(axum::body::Body::empty())
            .expect("request");
        assert!(requires_cookie(&req));
    }

    #[test]
    fn should_handle_cookie_for_kudos_and_pulse() {
        let req = Request::builder()
            .method("GET")
            .uri("/v2/kudos")
            .body(axum::body::Body::empty())
            .expect("request");
        assert!(should_handle_cookie(&req));
        let req = Request::builder()
            .method("POST")
            .uri("/v2/pulse/pv")
            .body(axum::body::Body::empty())
            .expect("request");
        assert!(should_handle_cookie(&req));
        let req = Request::builder()
            .method("GET")
            .uri("/v2/search")
            .body(axum::body::Body::empty())
            .expect("request");
        assert!(!should_handle_cookie(&req));
    }
}
