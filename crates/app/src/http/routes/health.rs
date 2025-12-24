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
