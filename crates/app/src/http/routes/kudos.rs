use axum::extract::{Extension, State};
use axum::http::HeaderMap;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;
use thiserror::Error;
use tracing::warn;

use crate::http::middleware::public_token::{self, PublicTokenError};
use crate::http::middleware::bid_cookie::ClientIds;
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct KudosResponse {
    pub count: i64,
    pub interacted: bool,
}

#[derive(Debug, Error)]
pub enum KudosApiError {
    #[error("token is required")]
    MissingToken,
    #[error("token is invalid")]
    InvalidToken,
    #[error("path is invalid")]
    InvalidPath,
    #[error("token not configured")]
    TokenNotConfigured,
    #[error("db not configured")]
    DbUnavailable,
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

pub async fn get_kudos(
    State(state): State<AppState>,
    Extension(ids): Extension<ClientIds>,
    headers: HeaderMap,
) -> Result<Json<KudosResponse>, KudosApiError> {
    ensure_db_configured(&state)?;
    let secret = token_secret(&state)?;
    let path = public_token::extract_path(&headers, secret).map_err(map_token_error)?;
    let cache = state.kudos_cache.read().await;
    let count = cache.count(&path);
    let interacted = cache.has(&path, &ids.interaction_id);
    Ok(Json(KudosResponse { count, interacted }))
}

pub async fn put_kudos(
    State(state): State<AppState>,
    Extension(ids): Extension<ClientIds>,
    headers: HeaderMap,
) -> Result<Json<KudosResponse>, KudosApiError> {
    ensure_db_configured(&state)?;
    let secret = token_secret(&state)?;
    let path = public_token::extract_path(&headers, secret).map_err(map_token_error)?;
    let mut cache = state.kudos_cache.write().await;
    cache.insert(&path, &ids.interaction_id);
    let count = cache.count(&path);
    Ok(Json(KudosResponse {
        count,
        interacted: true,
    }))
}

fn token_secret(state: &AppState) -> Result<&str, KudosApiError> {
    state
        .config
        .public_token_secret
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(KudosApiError::TokenNotConfigured)
}

fn ensure_db_configured(state: &AppState) -> Result<(), KudosApiError> {
    if state.db.is_none() {
        return Err(KudosApiError::DbUnavailable);
    }
    Ok(())
}

impl IntoResponse for KudosApiError {
    fn into_response(self) -> axum::response::Response {
        warn!(error = %self, "kudos api error");
        let (status, message) = match &self {
            KudosApiError::MissingToken => (StatusCode::BAD_REQUEST, self.to_string()),
            KudosApiError::InvalidToken => (StatusCode::UNAUTHORIZED, self.to_string()),
            KudosApiError::InvalidPath => (StatusCode::BAD_REQUEST, self.to_string()),
            KudosApiError::TokenNotConfigured => {
                (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
            }
            KudosApiError::DbUnavailable => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}

fn map_token_error(err: PublicTokenError) -> KudosApiError {
    match err {
        PublicTokenError::MissingToken => KudosApiError::MissingToken,
        PublicTokenError::InvalidToken => KudosApiError::InvalidToken,
        PublicTokenError::InvalidPath => KudosApiError::InvalidPath,
    }
}
