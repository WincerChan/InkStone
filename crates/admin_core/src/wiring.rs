use std::sync::Arc;

use reqwest::Client;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::state::{AdminHealthState, AdminState, ContentRefreshBackoff};
use inkstone_app::config::AppConfig;

#[derive(Debug, Error)]
pub enum WiringError {
    #[error("core wiring error: {0}")]
    Core(#[from] inkstone_app::wiring::WiringError),
    #[error("http client error: {0}")]
    HttpClient(#[from] reqwest::Error),
}

pub fn build_admin_state(config: AppConfig) -> Result<AdminState, WiringError> {
    let core = inkstone_app::wiring::build_state(config)?;
    let client = Client::builder()
        .timeout(core.config.request_timeout)
        .build()?;
    Ok(AdminState {
        config: core.config,
        search: core.search,
        http_client: client,
        db: core.db,
        content_refresh_backoff: Arc::new(Mutex::new(ContentRefreshBackoff::default())),
        admin_health: Arc::new(Mutex::new(AdminHealthState::default())),
    })
}
