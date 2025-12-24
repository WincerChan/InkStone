use axum::body::Bytes;
use axum::extract::State;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use hmac::{Hmac, Mac};
use serde::Deserialize;
use sha2::Sha256;
use thiserror::Error;
use tracing::{info, warn};

use crate::jobs::tasks;
use crate::state::AppState;

const HEADER_EVENT: &str = "x-github-event";
const HEADER_SIGNATURE: &str = "x-hub-signature-256";

#[derive(Debug, Error)]
pub enum WebhookError {
    #[error("webhook secret not configured")]
    SecretUnavailable,
    #[error("missing event header")]
    MissingEvent,
    #[error("missing signature header")]
    MissingSignature,
    #[error("invalid signature")]
    InvalidSignature,
    #[error("invalid payload")]
    InvalidPayload,
}

#[derive(Debug, Deserialize)]
struct CheckRunPayload {
    action: Option<String>,
    check_run: Option<CheckRun>,
}

#[derive(Debug, Deserialize)]
struct CheckRun {
    status: Option<String>,
    conclusion: Option<String>,
}

pub async fn github_webhook(
    State(state): State<AppState>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, WebhookError> {
    let secret = state
        .config
        .github_webhook_secret
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(WebhookError::SecretUnavailable)?;
    let event = header_value(&headers, HEADER_EVENT).ok_or(WebhookError::MissingEvent)?;
    let signature = header_value(&headers, HEADER_SIGNATURE).ok_or(WebhookError::MissingSignature)?;
    if !verify_signature(secret, &body, signature) {
        return Err(WebhookError::InvalidSignature);
    }

    if event.eq_ignore_ascii_case("ping") {
        return Ok(StatusCode::NO_CONTENT);
    }
    if !event.eq_ignore_ascii_case("check_run") {
        return Ok(StatusCode::ACCEPTED);
    }

    let payload: CheckRunPayload =
        serde_json::from_slice(&body).map_err(|_| WebhookError::InvalidPayload)?;
    if !should_trigger(&payload) {
        return Ok(StatusCode::ACCEPTED);
    }

    let state = state.clone();
    tokio::spawn(async move {
        info!("github webhook triggered refresh");
        if let Err(err) = tasks::valid_paths_refresh::run(&state).await {
            warn!(error = %err, "valid paths refresh failed");
        }
        match tasks::feed_index::run(&state, false).await {
            Ok(stats) => info!(?stats, "feed index run complete"),
            Err(err) => warn!(error = %err, "feed index run failed"),
        }
    });

    Ok(StatusCode::ACCEPTED)
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn verify_signature(secret: &str, body: &[u8], signature: &str) -> bool {
    let signature = match signature.strip_prefix("sha256=") {
        Some(value) => value,
        None => return false,
    };
    let signature = match hex::decode(signature) {
        Ok(value) => value,
        Err(_) => return false,
    };
    let mut mac = match Hmac::<Sha256>::new_from_slice(secret.as_bytes()) {
        Ok(mac) => mac,
        Err(_) => return false,
    };
    mac.update(body);
    mac.verify_slice(&signature).is_ok()
}

fn should_trigger(payload: &CheckRunPayload) -> bool {
    let action = payload.action.as_deref().unwrap_or("");
    let Some(check_run) = payload.check_run.as_ref() else {
        return false;
    };
    let status = check_run.status.as_deref().unwrap_or("");
    let conclusion = check_run.conclusion.as_deref().unwrap_or("");
    action == "completed" && status == "completed" && conclusion == "success"
}

impl IntoResponse for WebhookError {
    fn into_response(self) -> axum::response::Response {
        let status = match self {
            WebhookError::SecretUnavailable => StatusCode::SERVICE_UNAVAILABLE,
            WebhookError::MissingEvent
            | WebhookError::MissingSignature
            | WebhookError::InvalidPayload => StatusCode::BAD_REQUEST,
            WebhookError::InvalidSignature => StatusCode::UNAUTHORIZED,
        };
        (status, self.to_string()).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{should_trigger, verify_signature, CheckRun, CheckRunPayload};
    use hmac::{Hmac, Mac};
    use sha2::Sha256;

    fn sign(secret: &str, body: &[u8]) -> String {
        let mut mac = Hmac::<Sha256>::new_from_slice(secret.as_bytes()).unwrap();
        mac.update(body);
        let raw = mac.finalize().into_bytes();
        format!("sha256={}", hex::encode(raw))
    }

    #[test]
    fn verify_signature_accepts_valid() {
        let secret = "secret";
        let body = b"payload";
        let sig = sign(secret, body);
        assert!(verify_signature(secret, body, &sig));
    }

    #[test]
    fn verify_signature_rejects_invalid() {
        let secret = "secret";
        let body = b"payload";
        assert!(!verify_signature(secret, body, "sha256=deadbeef"));
    }

    #[test]
    fn should_trigger_only_on_success() {
        let payload = CheckRunPayload {
            action: Some("completed".to_string()),
            check_run: Some(CheckRun {
                status: Some("completed".to_string()),
                conclusion: Some("success".to_string()),
            }),
        };
        assert!(should_trigger(&payload));
    }

    #[test]
    fn should_not_trigger_without_check_run() {
        let payload = CheckRunPayload {
            action: Some("completed".to_string()),
            check_run: None,
        };
        assert!(!should_trigger(&payload));
    }
}
