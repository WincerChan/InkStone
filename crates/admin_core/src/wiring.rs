use std::sync::Arc;

use reqwest::Client;
use thiserror::Error;
use tokio::sync::Mutex;

use crate::state::{AdminHealthState, AdminState, ContentRefreshBackoff};
use inkstone_infra::db::{DbPoolError, connect_lazy};
use inkstone_infra::search::{SearchIndex, SearchIndexError};
use inkstone_runtime::config::AdminConfig;

#[derive(Debug, Error)]
pub enum WiringError {
    #[error("search index error: {0}")]
    SearchIndex(#[from] SearchIndexError),
    #[error("http client error: {0}")]
    HttpClient(#[from] reqwest::Error),
    #[error("db pool error: {0}")]
    DbPool(#[from] DbPoolError),
}

pub fn build_admin_state(config: AdminConfig) -> Result<AdminState, WiringError> {
    let search = SearchIndex::open_or_create(&config.index_dir)?;
    let db = match config.database_url.as_deref() {
        Some(url) => Some(connect_lazy(url)?),
        None => None,
    };
    let client = Client::builder()
        .timeout(config.request_timeout)
        .build()?;
    Ok(AdminState {
        config: Arc::new(config),
        search: Arc::new(search),
        http_client: client,
        db,
        content_refresh_backoff: Arc::new(Mutex::new(ContentRefreshBackoff::default())),
        admin_health: Arc::new(Mutex::new(AdminHealthState::default())),
    })
}
