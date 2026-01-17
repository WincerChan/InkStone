use axum::extract::State;
use axum::Json;
use serde::Serialize;
use inkstone_runtime::health::{
    DatabaseStatus, KudosStatus, ModuleStatus, PublicHealthModules, PulseStatus,
};
use crate::state::AppState;

#[derive(Debug, Serialize)]
pub struct HealthResponse {
    pub status: &'static str,
    pub modules: PublicHealthModules,
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
    let token_ready = state
        .config
        .public_token_secret
        .as_ref()
        .is_some_and(|value| !value.is_empty());
    let kudos_enabled = db_configured && cookie_ready && token_ready;
    let pulse_enabled = db_configured && cookie_ready && token_ready;
    let comments_enabled = db_configured;

    Json(HealthResponse {
        status: "ok",
        modules: PublicHealthModules {
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
                token_ready,
            },
            pulse: PulseStatus {
                enabled: pulse_enabled,
                cookie_ready,
                token_ready,
            },
        },
    })
}

#[cfg(test)]
mod tests {
    use super::health;
    use axum::extract::State;
    use std::sync::Arc;
    use serde_json::Value;
    use crate::state::AppState;
    use inkstone_infra::db::connect_lazy;
    use inkstone_infra::search::SearchIndex;
    use inkstone_runtime::config::PublicConfig;

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
        let config = PublicConfig {
            http_addr: "127.0.0.1:8080".parse().unwrap(),
            index_dir,
            max_search_limit: 50,
            database_url: None,
            cookie_secret: Some("cookie".to_string()),
            stats_secret: Some("stats".to_string()),
            search_hash_secret: None,
            public_token_secret: Some("token".to_string()),
            cors_allow_origins: Vec::new(),
            pulse_allowed_slds: Vec::new(),
        };
        AppState {
            config: Arc::new(config),
            search: Arc::new(search),
            db,
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

    #[tokio::test]
    async fn health_excludes_admin_only_modules() {
        let state = build_state(true);
        let response = health(State(state)).await;
        let value = serde_json::to_value(&response.0).unwrap();
        let modules = value
            .get("modules")
            .and_then(Value::as_object)
            .unwrap();
        assert!(!modules.contains_key("webhook"));
        assert!(!modules.contains_key("douban"));
    }
}
