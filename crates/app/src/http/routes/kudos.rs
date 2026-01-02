use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

use crate::http::middleware::bid_cookie::ClientIds;
use crate::state::AppState;

const MAX_PATH_LEN: usize = 512;

#[derive(Debug, Deserialize)]
pub struct KudosParams {
    pub path: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct KudosResponse {
    pub count: i64,
    pub interacted: bool,
}

#[derive(Debug, Error)]
pub enum KudosApiError {
    #[error("path is required")]
    MissingPath,
    #[error("path is invalid")]
    InvalidPath,
    #[error("path is not allowed")]
    PathNotAllowed,
    #[error("valid paths not loaded")]
    ValidPathsUnavailable,
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
    Query(params): Query<KudosParams>,
) -> Result<Json<KudosResponse>, KudosApiError> {
    ensure_db_configured(&state)?;
    let path = normalize_path(params.path)?;
    ensure_valid_path(&state, &path).await?;
    let cache = state.kudos_cache.read().await;
    let count = cache.count(&path);
    let interacted = cache.has(&path, &ids.interaction_id);
    Ok(Json(KudosResponse { count, interacted }))
}

pub async fn put_kudos(
    State(state): State<AppState>,
    Extension(ids): Extension<ClientIds>,
    Query(params): Query<KudosParams>,
) -> Result<Json<KudosResponse>, KudosApiError> {
    ensure_db_configured(&state)?;
    let path = normalize_path(params.path)?;
    ensure_valid_path(&state, &path).await?;
    let mut cache = state.kudos_cache.write().await;
    cache.insert(&path, &ids.interaction_id);
    let count = cache.count(&path);
    Ok(Json(KudosResponse {
        count,
        interacted: true,
    }))
}

fn normalize_path(value: Option<String>) -> Result<String, KudosApiError> {
    let path = value.unwrap_or_default();
    let trimmed = path.trim();
    if trimmed.is_empty() {
        return Err(KudosApiError::MissingPath);
    }
    if trimmed.len() > MAX_PATH_LEN || !trimmed.starts_with('/') {
        return Err(KudosApiError::InvalidPath);
    }
    if trimmed.chars().any(|ch| ch.is_whitespace()) {
        return Err(KudosApiError::InvalidPath);
    }
    Ok(trimmed.to_string())
}

async fn ensure_valid_path(state: &AppState, path: &str) -> Result<(), KudosApiError> {
    let valid_paths = state.valid_paths.read().await;
    if valid_paths.is_empty() {
        return Err(KudosApiError::ValidPathsUnavailable);
    }
    if !valid_paths.contains(path) {
        return Err(KudosApiError::PathNotAllowed);
    }
    Ok(())
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
            KudosApiError::MissingPath => (StatusCode::BAD_REQUEST, self.to_string()),
            KudosApiError::InvalidPath => (StatusCode::BAD_REQUEST, self.to_string()),
            KudosApiError::PathNotAllowed => (StatusCode::NOT_FOUND, self.to_string()),
            KudosApiError::ValidPathsUnavailable => {
                (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
            }
            KudosApiError::DbUnavailable => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::normalize_path;

    #[test]
    fn normalize_path_rejects_empty() {
        assert!(normalize_path(None).is_err());
        assert!(normalize_path(Some("".to_string())).is_err());
    }

    #[test]
    fn normalize_path_accepts_basic() {
        let path = normalize_path(Some("/posts/hello".to_string())).unwrap();
        assert_eq!(path, "/posts/hello");
    }
}
