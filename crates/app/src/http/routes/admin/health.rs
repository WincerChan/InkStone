use axum::extract::State;
use axum::Json;
use serde::Serialize;

use crate::http::routes::health::{
    DatabaseStatus, HealthModules, KudosStatus, ModuleStatus, PulseStatus, ValidPathsStatus,
    WebhookStatus,
};
use crate::state::{AdminHealthState, AppState};

#[derive(Debug, Serialize)]
pub struct AdminHealthResponse {
    pub status: &'static str,
    pub modules: HealthModules,
    pub jobs: AdminJobsStatus,
    pub webhooks: AdminWebhooksStatus,
}

#[derive(Debug, Serialize)]
pub struct AdminJobsStatus {
    pub content_refresh: AdminJobStatus,
    pub douban_crawl: AdminJobStatus,
    pub comments_sync: AdminJobStatus,
    pub kudos_flush: AdminJobStatus,
}

#[derive(Debug, Serialize)]
pub struct AdminJobStatus {
    pub last_run_at: Option<String>,
    pub last_success_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct AdminWebhooksStatus {
    pub content: AdminWebhookStatus,
    pub discussions: AdminWebhookStatus,
}

#[derive(Debug, Serialize)]
pub struct AdminWebhookStatus {
    pub last_received_at: Option<String>,
}

pub async fn get_admin_health(State(state): State<AppState>) -> Json<AdminHealthResponse> {
    let modules = build_modules(&state).await;
    let snapshot = {
        let guard = state.admin_health.lock().await;
        guard.clone()
    };
    Json(AdminHealthResponse {
        status: "ok",
        modules,
        jobs: map_jobs(&snapshot),
        webhooks: map_webhooks(&snapshot),
    })
}

async fn build_modules(state: &AppState) -> HealthModules {
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

    HealthModules {
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
    }
}

fn map_jobs(snapshot: &AdminHealthState) -> AdminJobsStatus {
    AdminJobsStatus {
        content_refresh: AdminJobStatus {
            last_run_at: format_timestamp(snapshot.content_refresh_last_run),
            last_success_at: format_timestamp(snapshot.content_refresh_last_success),
        },
        douban_crawl: AdminJobStatus {
            last_run_at: format_timestamp(snapshot.douban_crawl_last_run),
            last_success_at: format_timestamp(snapshot.douban_crawl_last_success),
        },
        comments_sync: AdminJobStatus {
            last_run_at: format_timestamp(snapshot.comments_sync_last_run),
            last_success_at: format_timestamp(snapshot.comments_sync_last_success),
        },
        kudos_flush: AdminJobStatus {
            last_run_at: format_timestamp(snapshot.kudos_flush_last_run),
            last_success_at: format_timestamp(snapshot.kudos_flush_last_success),
        },
    }
}

fn map_webhooks(snapshot: &AdminHealthState) -> AdminWebhooksStatus {
    AdminWebhooksStatus {
        content: AdminWebhookStatus {
            last_received_at: format_timestamp(snapshot.webhook_content_last_received),
        },
        discussions: AdminWebhookStatus {
            last_received_at: format_timestamp(snapshot.webhook_discussions_last_received),
        },
    }
}

fn format_timestamp(value: Option<chrono::DateTime<chrono::Utc>>) -> Option<String> {
    value.map(|timestamp| timestamp.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::format_timestamp;
    use chrono::{TimeZone, Utc};

    #[test]
    fn format_timestamp_renders_rfc3339() {
        let ts = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(
            format_timestamp(Some(ts)),
            Some("2025-01-01T00:00:00+00:00".to_string())
        );
    }
}
