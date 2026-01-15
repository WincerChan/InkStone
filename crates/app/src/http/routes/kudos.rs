use axum::extract::{Extension, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::warn;

use crate::http::middleware::public_token::{self, PublicTokenError};
use crate::http::middleware::bid_cookie::ClientIds;
use crate::state::AppState;

const MAX_PATH_LEN: usize = 512;

#[derive(Debug, Deserialize)]
pub struct KudosParams {
    #[serde(rename = "inkstone_token")]
    pub inkstone_token: Option<String>,
    pub path: Option<String>,
}

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
    Query(params): Query<KudosParams>,
) -> Result<Json<KudosResponse>, KudosApiError> {
    ensure_db_configured(&state)?;
    let path = resolve_path(
        &state,
        params.inkstone_token.as_deref(),
        params.path.as_deref(),
    )?;
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
    let path = resolve_path(
        &state,
        params.inkstone_token.as_deref(),
        params.path.as_deref(),
    )?;
    let mut cache = state.kudos_cache.write().await;
    cache.insert(&path, &ids.interaction_id);
    let count = cache.count(&path);
    Ok(Json(KudosResponse {
        count,
        interacted: true,
    }))
}

fn resolve_path(
    state: &AppState,
    token: Option<&str>,
    legacy_path: Option<&str>,
) -> Result<String, KudosApiError> {
    if let Some(token) = token {
        let secret = token_secret(state)?;
        return public_token::extract_path_from_token(secret, token).map_err(map_token_error);
    }
    // TODO(compat): temporary legacy fallback for `path` query param during rollout.
    normalize_legacy_path(legacy_path)
}

fn token_secret(state: &AppState) -> Result<&str, KudosApiError> {
    state
        .config
        .public_token_secret
        .as_deref()
        .filter(|value| !value.is_empty())
        .ok_or(KudosApiError::TokenNotConfigured)
}

fn normalize_legacy_path(path: Option<&str>) -> Result<String, KudosApiError> {
    let trimmed = path.unwrap_or("").trim();
    if trimmed.is_empty() {
        return Err(KudosApiError::MissingToken);
    }
    if trimmed.len() > MAX_PATH_LEN || !trimmed.starts_with('/') {
        return Err(KudosApiError::InvalidPath);
    }
    if trimmed.chars().any(|ch| ch.is_whitespace()) {
        return Err(KudosApiError::InvalidPath);
    }
    if trimmed.contains('?') || trimmed.contains('#') {
        return Err(KudosApiError::InvalidPath);
    }
    Ok(trimmed.to_string())
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
        PublicTokenError::InvalidToken => KudosApiError::InvalidToken,
        PublicTokenError::InvalidPath => KudosApiError::InvalidPath,
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_legacy_path, resolve_path};
    use crate::http::middleware::public_token;
    use crate::state::AppState;
    use inkstone_infra::search::SearchStrategy;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};

    fn build_state(secret: Option<&str>) -> AppState {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let strategy = SearchStrategy::Jieba;
        let index_dir = std::env::temp_dir()
            .join(format!("inkstone-kudos-{suffix}"))
            .join(strategy.as_dir_name());
        let _ = std::fs::create_dir_all(&index_dir);
        let config = crate::config::AppConfig {
            http_addr: "127.0.0.1:8080".parse().unwrap(),
            index_dir: index_dir.clone(),
            search_strategy: strategy,
            feed_url: "https://example.com/index.json".to_string(),
            poll_interval: std::time::Duration::from_secs(300),
            douban_poll_interval: std::time::Duration::from_secs(300),
            comments_sync_interval: std::time::Duration::from_secs(300),
            request_timeout: std::time::Duration::from_secs(15),
            max_search_limit: 50,
            database_url: None,
            douban_max_pages: 1,
            douban_uid: "1".to_string(),
            douban_cookie: "cookie".to_string(),
            douban_user_agent: "ua".to_string(),
            cookie_secret: Some("cookie".to_string()),
            stats_secret: Some("stats".to_string()),
            search_hash_secret: None,
            public_token_secret: secret.map(|value| value.to_string()),
            kudos_flush_interval: std::time::Duration::from_secs(60),
            github_webhook_secret: None,
            github_discussion_webhook_secret: None,
            github_app_id: None,
            github_app_installation_id: None,
            github_app_private_key: None,
            github_repo_owner: None,
            github_repo_name: None,
            github_discussion_category_id: None,
            cors_allow_origins: Vec::new(),
            pulse_allowed_slds: Vec::new(),
            admin_password_hash: None,
            admin_token_secret: None,
        };
        AppState {
            config: Arc::new(config),
            search: Arc::new(
                inkstone_infra::search::SearchIndex::open_or_create(&index_dir, strategy).unwrap(),
            ),
            http_client: reqwest::Client::new(),
            db: None,
            kudos_cache: Arc::new(RwLock::new(crate::kudos_cache::KudosCache::default())),
            content_refresh_backoff: Arc::new(Mutex::new(
                crate::state::ContentRefreshBackoff::default(),
            )),
            admin_health: Arc::new(Mutex::new(crate::state::AdminHealthState::default())),
        }
    }

    #[test]
    fn normalize_legacy_path_rejects_missing() {
        assert!(matches!(
            normalize_legacy_path(None).unwrap_err(),
            super::KudosApiError::MissingToken
        ));
    }

    #[test]
    fn resolve_path_prefers_token() {
        let state = build_state(Some("secret"));
        let token = public_token::issue_token("secret", "/posts/a/").unwrap();
        let path = resolve_path(&state, Some(&token), Some("/posts/b/")).unwrap();
        assert_eq!(path, "/posts/a/");
    }

    #[test]
    fn resolve_path_uses_legacy_when_missing_token() {
        let state = build_state(None);
        let path = resolve_path(&state, None, Some("/posts/hello/")).unwrap();
        assert_eq!(path, "/posts/hello/");
    }
}
