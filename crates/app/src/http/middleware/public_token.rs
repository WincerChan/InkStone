use axum::http::HeaderMap;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use hmac::{Hmac, Mac};
use serde::{Deserialize, Serialize};
use sha2::Sha256;

const TOKEN_HEADER: &str = "x-inkstone-token";
const MAX_PATH_LEN: usize = 512;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PublicTokenError {
    MissingToken,
    InvalidToken,
    InvalidPath,
}

#[derive(Debug, Serialize, Deserialize)]
struct PublicTokenPayload {
    path: String,
}

pub fn extract_path(headers: &HeaderMap, secret: &str) -> Result<String, PublicTokenError> {
    let token = extract_token(headers).ok_or(PublicTokenError::MissingToken)?;
    verify_token(secret, token)
}

#[cfg(test)]
pub fn issue_token(secret: &str, path: &str) -> Result<String, PublicTokenError> {
    let path = normalize_path(path)?;
    let payload = PublicTokenPayload { path };
    let json = serde_json::to_vec(&payload).map_err(|_| PublicTokenError::InvalidToken)?;
    let payload_b64 = URL_SAFE_NO_PAD.encode(json);
    let signature = sign_token(secret, &payload_b64);
    Ok(format!("{payload_b64}.{signature}"))
}

fn verify_token(secret: &str, token: &str) -> Result<String, PublicTokenError> {
    let mut iter = token.splitn(2, '.');
    let payload_b64 = iter.next().filter(|value| !value.is_empty());
    let sig = iter.next().filter(|value| !value.is_empty());
    let (payload_b64, sig) = match (payload_b64, sig) {
        (Some(payload_b64), Some(sig)) => (payload_b64, sig),
        _ => return Err(PublicTokenError::InvalidToken),
    };
    if sig != sign_token(secret, payload_b64) {
        return Err(PublicTokenError::InvalidToken);
    }
    let payload = decode_payload(payload_b64).ok_or(PublicTokenError::InvalidToken)?;
    normalize_path(&payload.path)
}

fn decode_payload(payload_b64: &str) -> Option<PublicTokenPayload> {
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

fn extract_token(headers: &HeaderMap) -> Option<&str> {
    headers
        .get(TOKEN_HEADER)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn normalize_path(value: &str) -> Result<String, PublicTokenError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(PublicTokenError::InvalidPath);
    }
    if trimmed.len() > MAX_PATH_LEN || !trimmed.starts_with('/') {
        return Err(PublicTokenError::InvalidPath);
    }
    if trimmed.chars().any(|ch| ch.is_whitespace()) {
        return Err(PublicTokenError::InvalidPath);
    }
    if trimmed.contains('?') || trimmed.contains('#') {
        return Err(PublicTokenError::InvalidPath);
    }
    Ok(trimmed.to_string())
}

#[cfg(test)]
mod tests {
    use super::{extract_path, issue_token, PublicTokenError};
    use axum::http::{HeaderMap, HeaderValue};

    #[test]
    fn issue_token_round_trip() {
        let token = issue_token("secret", "/posts/hello/").unwrap();
        let mut headers = HeaderMap::new();
        headers.insert("x-inkstone-token", HeaderValue::from_str(&token).unwrap());
        let path = extract_path(&headers, "secret").unwrap();
        assert_eq!(path, "/posts/hello/");
    }

    #[test]
    fn extract_path_rejects_missing_header() {
        let headers = HeaderMap::new();
        assert_eq!(
            extract_path(&headers, "secret").unwrap_err(),
            PublicTokenError::MissingToken
        );
    }

    #[test]
    fn extract_path_rejects_invalid_signature() {
        let token = issue_token("secret", "/posts/hello/").unwrap();
        let mut headers = HeaderMap::new();
        let tampered = format!("{token}x");
        headers.insert("x-inkstone-token", HeaderValue::from_str(&tampered).unwrap());
        assert_eq!(
            extract_path(&headers, "secret").unwrap_err(),
            PublicTokenError::InvalidToken
        );
    }

    #[test]
    fn issue_token_rejects_invalid_path() {
        assert_eq!(
            issue_token("secret", "posts/hello").unwrap_err(),
            PublicTokenError::InvalidPath
        );
    }
}
