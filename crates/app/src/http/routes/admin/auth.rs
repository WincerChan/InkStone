use axum::extract::State;
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use argon2::{Argon2, PasswordHash, PasswordVerifier};
use chrono::{Duration, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::http::middleware::admin_auth;
use crate::state::AppState;

const DEFAULT_REMEMBER_DAYS: i64 = 7;
const MAX_REMEMBER_DAYS: i64 = 30;

#[derive(Debug, Deserialize)]
pub struct AdminLoginRequest {
    pub password: String,
    pub remember_days: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct AdminLoginResponse {
    pub expires_at: String,
    pub max_age_secs: i64,
}

#[derive(Debug, Error)]
pub enum AdminLoginError {
    #[error("admin auth not configured")]
    MissingConfig,
    #[error("password is required")]
    MissingPassword,
    #[error("remember_days must be 7 or 30")]
    InvalidRememberDays,
    #[error("invalid password")]
    InvalidPassword,
    #[error("invalid admin password hash")]
    InvalidHash,
    #[error("token issuance failed")]
    TokenIssue,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

pub async fn login(
    State(state): State<AppState>,
    headers: HeaderMap,
    Json(payload): Json<AdminLoginRequest>,
) -> Result<Response, AdminLoginError> {
    let password = payload.password.trim();
    if password.is_empty() {
        return Err(AdminLoginError::MissingPassword);
    }

    let hash = state
        .config
        .admin_password_hash
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(AdminLoginError::MissingConfig)?;
    let secret = state
        .config
        .admin_token_secret
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(AdminLoginError::MissingConfig)?;

    let parsed_hash = PasswordHash::new(hash).map_err(|_| AdminLoginError::InvalidHash)?;
    let verifier = Argon2::default();
    if verifier
        .verify_password(password.as_bytes(), &parsed_hash)
        .is_err()
    {
        return Err(AdminLoginError::InvalidPassword);
    }

    let remember_days = resolve_remember_days(payload.remember_days)?;
    let max_age_secs = remember_days.saturating_mul(24 * 60 * 60);
    let token = admin_auth::issue_token(secret, max_age_secs).map_err(|_| AdminLoginError::TokenIssue)?;
    let secure = admin_auth::is_https(&headers);
    let cookie = admin_auth::build_cookie_value(&token, max_age_secs, secure);

    let expires_at = (Utc::now() + Duration::seconds(max_age_secs)).to_rfc3339();
    let response = Json(AdminLoginResponse {
        expires_at,
        max_age_secs,
    })
    .into_response();
    Ok(admin_auth::attach_cookie(response, cookie))
}

fn resolve_remember_days(value: Option<i64>) -> Result<i64, AdminLoginError> {
    let days = value.unwrap_or(DEFAULT_REMEMBER_DAYS);
    match days {
        DEFAULT_REMEMBER_DAYS | MAX_REMEMBER_DAYS => Ok(days),
        _ => Err(AdminLoginError::InvalidRememberDays),
    }
}

impl IntoResponse for AdminLoginError {
    fn into_response(self) -> Response {
        let (status, message) = match &self {
            AdminLoginError::MissingConfig | AdminLoginError::InvalidHash => {
                (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
            }
            AdminLoginError::MissingPassword | AdminLoginError::InvalidRememberDays => {
                (StatusCode::BAD_REQUEST, self.to_string())
            }
            AdminLoginError::InvalidPassword => (StatusCode::UNAUTHORIZED, self.to_string()),
            AdminLoginError::TokenIssue => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{resolve_remember_days, AdminLoginError};

    #[test]
    fn resolve_remember_days_defaults_to_seven() {
        assert_eq!(resolve_remember_days(None).unwrap(), 7);
    }

    #[test]
    fn resolve_remember_days_accepts_thirty() {
        assert_eq!(resolve_remember_days(Some(30)).unwrap(), 30);
    }

    #[test]
    fn resolve_remember_days_rejects_other_values() {
        let err = resolve_remember_days(Some(1)).unwrap_err();
        assert!(matches!(err, AdminLoginError::InvalidRememberDays));
    }
}
