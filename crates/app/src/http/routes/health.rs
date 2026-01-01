use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub modules: HealthModules,
}

#[derive(Debug, Serialize)]
pub struct HealthModules {
    pub search: ModuleStatus,
    pub database: DatabaseStatus,
    pub comments: ModuleStatus,
    pub kudos: KudosStatus,
    pub pulse: PulseStatus,
    pub douban: ModuleStatus,
    pub valid_paths: ValidPathsStatus,
    pub webhook: WebhookStatus,
}

#[derive(Debug, Serialize)]
pub struct ModuleStatus {
    pub enabled: bool,
}

#[derive(Debug, Serialize)]
pub struct DatabaseStatus {
    pub configured: bool,
}

#[derive(Debug, Serialize)]
pub struct KudosStatus {
    pub enabled: bool,
    pub cookie_ready: bool,
    pub valid_paths_loaded: bool,
}

#[derive(Debug, Serialize)]
pub struct PulseStatus {
    pub enabled: bool,
    pub cookie_ready: bool,
}

#[derive(Debug, Serialize)]
pub struct ValidPathsStatus {
    pub loaded: bool,
    pub count: usize,
}

#[derive(Debug, Serialize)]
pub struct WebhookStatus {
    pub configured: bool,
}

pub async fn health(State(state): State<AppState>) -> Json<HealthResponse> {
    let db_configured = state.db.is_some();
    let cookie_ready = state
        .config
        .cookie_secret
        .as_ref()
        .is_some_and(|value| !value.is_empty())
        && state
            .config
            .stats_secret
            .as_ref()
            .is_some_and(|value| !value.is_empty());
    let valid_paths_count = state.valid_paths.read().await.len();
    let valid_paths_loaded = valid_paths_count > 0;
    let kudos_enabled = db_configured && cookie_ready && valid_paths_loaded;
    let pulse_enabled = db_configured && cookie_ready;
    let comments_enabled = db_configured;
    let webhook_configured = state
        .config
        .github_webhook_secret
        .as_ref()
        .is_some_and(|value| !value.is_empty());

    Json(HealthResponse {
        status: "ok",
        modules: HealthModules {
            search: ModuleStatus { enabled: true },
            database: DatabaseStatus {
                configured: db_configured,
            },
            comments: ModuleStatus {
                enabled: comments_enabled,
            },
            kudos: KudosStatus {
                enabled: kudos_enabled,
                cookie_ready,
                valid_paths_loaded,
            },
            pulse: PulseStatus {
                enabled: pulse_enabled,
                cookie_ready,
            },
            douban: ModuleStatus {
                enabled: db_configured,
            },
            valid_paths: ValidPathsStatus {
                loaded: valid_paths_loaded,
                count: valid_paths_count,
            },
            webhook: WebhookStatus {
                configured: webhook_configured,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::health;
    use axum::extract::State;
    use chrono::Duration;
    use std::collections::HashSet;
    use std::sync::Arc;
    use tokio::sync::{Mutex, RwLock};

    use crate::config::AppConfig;
    use crate::kudos_cache::KudosCache;
    use crate::state::{AdminHealthState, AppState, ContentRefreshBackoff};
    use inkstone_infra::db::connect_lazy;
    use inkstone_infra::search::SearchIndex;

    fn build_state(db_configured: bool) -> AppState {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let index_dir = std::env::temp_dir().join(format!("inkstone-health-{suffix}"));
        let _ = std::fs::create_dir_all(&index_dir);
        let search = SearchIndex::open_or_create(&index_dir).unwrap();
        let db = if db_configured {
            Some(connect_lazy("postgres://user:pass@localhost/db").unwrap())
        } else {
            None
        };
        let config = AppConfig {
            http_addr: "127.0.0.1:8080".parse().unwrap(),
            index_dir,
            feed_url: "https://example.com/index.json".to_string(),
            poll_interval: Duration::seconds(300).to_std().unwrap(),
            douban_poll_interval: Duration::seconds(300).to_std().unwrap(),
            comments_sync_interval: Duration::seconds(300).to_std().unwrap(),
            request_timeout: Duration::seconds(15).to_std().unwrap(),
            max_search_limit: 50,
            database_url: None,
            douban_max_pages: 1,
            douban_uid: "93562087".to_string(),
            douban_cookie: "bid=3EHqn8aRvcI".to_string(),
            douban_user_agent: "ua".to_string(),
            cookie_secret: Some("cookie".to_string()),
            stats_secret: Some("stats".to_string()),
            valid_paths_url: "https://example.com/paths.txt".to_string(),
            kudos_flush_interval: Duration::seconds(60).to_std().unwrap(),
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
            search: Arc::new(search),
            http_client: reqwest::Client::new(),
            db,
            valid_paths: Arc::new(RwLock::new(HashSet::new())),
            kudos_cache: Arc::new(RwLock::new(KudosCache::default())),
            content_refresh_backoff: Arc::new(Mutex::new(ContentRefreshBackoff::default())),
            admin_health: Arc::new(Mutex::new(AdminHealthState::default())),
        }
    }

    #[tokio::test]
    async fn health_marks_comments_enabled_when_db_configured() {
        let state = build_state(true);
        let response = health(State(state)).await;
        assert!(response.modules.comments.enabled);
    }

    #[tokio::test]
    async fn health_marks_comments_disabled_without_db() {
        let state = build_state(false);
        let response = health(State(state)).await;
        assert!(!response.modules.comments.enabled);
    }
}
