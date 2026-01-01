use axum::body::Body;
use axum::extract::State;
use axum::http::header::{AUTHORIZATION, COOKIE, SET_COOKIE};
use axum::http::Request;
use axum::middleware::Next;
use axum::response::{IntoResponse, Response};
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use chrono::Utc;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;
use thiserror::Error;

use crate::state::AppState;

const ADMIN_COOKIE_NAME: &str = "inkstone_admin";

#[derive(Debug, Error)]
pub enum AdminAuthError {
    #[error("admin auth not configured")]
    MissingConfig,
    #[error("admin token required")]
    MissingToken,
    #[error("admin token invalid")]
    InvalidToken,
}

#[derive(Debug, Serialize, Deserialize)]
struct AdminTokenPayload {
    exp: i64,
}

pub async fn require_admin(
    State(state): State<AppState>,
    request: Request<Body>,
    next: Next,
) -> Result<Response, AdminAuthError> {
    let path = request.uri().path();
    if !path.starts_with("/v2/admin") || path == "/v2/admin/login" {
        return Ok(next.run(request).await);
    }

    let secret = state
        .config
        .admin_token_secret
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(AdminAuthError::MissingConfig)?;

    let token = extract_bearer_token(&request)
        .or_else(|| extract_cookie(&request, ADMIN_COOKIE_NAME));
    let token = token.ok_or(AdminAuthError::MissingToken)?;
    if !verify_token(secret, &token) {
        return Err(AdminAuthError::InvalidToken);
    }
    Ok(next.run(request).await)
}

pub fn issue_token(secret: &str, max_age_secs: i64) -> Result<String, AdminAuthError> {
    let exp = Utc::now().timestamp().saturating_add(max_age_secs);
    let payload = AdminTokenPayload { exp };
    let json = serde_json::to_vec(&payload).map_err(|_| AdminAuthError::InvalidToken)?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(json);
    let signature = sign_token(secret, &payload_b64);
    Ok(format!("{payload_b64}.{signature}"))
}

pub fn build_cookie_value(token: &str, max_age_secs: i64, secure: bool) -> String {
    let mut cookie = format!(
        "{name}={value}; Path=/v2/admin; HttpOnly; SameSite=Lax; Max-Age={max_age}",
        name = ADMIN_COOKIE_NAME,
        value = token,
        max_age = max_age_secs
    );
    if secure {
        cookie.push_str("; Secure");
    }
    cookie
}

fn verify_token(secret: &str, token: &str) -> bool {
    let mut iter = token.splitn(2, '.');
    let payload_b64 = match iter.next() {
        Some(value) if !value.is_empty() => value,
        _ => return false,
    };
    let sig = match iter.next() {
        Some(value) if !value.is_empty() => value,
        _ => return false,
    };
    if sig != sign_token(secret, payload_b64) {
        return false;
    }
    let payload = match decode_payload(payload_b64) {
        Some(value) => value,
        None => return false,
    };
    payload.exp > Utc::now().timestamp()
}

fn decode_payload(payload_b64: &str) -> Option<AdminTokenPayload> {
    let bytes = URL_SAFE_NO_PAD.decode(payload_b64.as_bytes()).ok()?;
    serde_json::from_slice(&bytes).ok()
}

fn sign_token(secret: &str, payload_b64: &str) -> String {
    let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes())
        .expect("hmac can take key of any size");
    mac.update(payload_b64.as_bytes());
    let raw = mac.finalize().into_bytes();
    URL_SAFE_NO_PAD.encode(raw)
}

fn extract_bearer_token<B>(request: &Request<B>) -> Option<String> {
    let header = request.headers().get(AUTHORIZATION)?.to_str().ok()?;
    let header = header.trim();
    let value = header.strip_prefix("Bearer ")?;
    let value = value.trim();
    if value.is_empty() {
        None
    } else {
        Some(value.to_string())
    }
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

pub fn is_https(headers: &axum::http::HeaderMap) -> bool {
    if let Some(value) = headers.get("x-forwarded-proto") {
        if let Ok(value) = value.to_str() {
            if value.split(',').any(|part| part.trim().eq_ignore_ascii_case("https")) {
                return true;
            }
        }
    }
    if let Some(value) = headers.get("forwarded") {
        if let Ok(value) = value.to_str() {
            for part in value.split(';') {
                let part = part.trim();
                if let Some(proto) = part.strip_prefix("proto=") {
                    if proto.trim().eq_ignore_ascii_case("https") {
                        return true;
                    }
                }
            }
        }
    }
    false
}

impl IntoResponse for AdminAuthError {
    fn into_response(self) -> Response {
        let status = match self {
            AdminAuthError::MissingConfig => axum::http::StatusCode::SERVICE_UNAVAILABLE,
            AdminAuthError::MissingToken | AdminAuthError::InvalidToken => {
                axum::http::StatusCode::UNAUTHORIZED
            }
        };
        (status, self.to_string()).into_response()
    }
}

pub fn attach_cookie(mut response: Response, cookie_value: String) -> Response {
    if let Ok(value) = cookie_value.parse() {
        response.headers_mut().append(SET_COOKIE, value);
    }
    response
}

#[cfg(test)]
mod tests {
    use super::{build_cookie_value, issue_token, verify_token};

    #[test]
    fn issue_token_round_trip() {
        let secret = "secret";
        let token = issue_token(secret, 60).unwrap();
        assert!(verify_token(secret, &token));
    }

    #[test]
    fn build_cookie_includes_path_and_age() {
        let cookie = build_cookie_value("token", 60, false);
        assert!(cookie.contains("Path=/v2/admin"));
        assert!(cookie.contains("Max-Age=60"));
    }
}
