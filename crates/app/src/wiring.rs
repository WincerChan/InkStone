use std::sync::Arc;

use reqwest::Client;
use thiserror::Error;

use crate::config::AppConfig;
use crate::state::AppState;
use inkstone_infra::search::{SearchIndex, SearchIndexError};

#[derive(Debug, Error)]
pub enum WiringError {
    #[error("search index error: {0}")]
    SearchIndex(#[from] SearchIndexError),
    #[error("http client error: {0}")]
    HttpClient(#[from] reqwest::Error),
}

pub fn build_state(config: AppConfig) -> Result<AppState, WiringError> {
    let search = SearchIndex::open_or_create(&config.index_dir)?;
    let client = Client::builder().timeout(config.request_timeout).build()?;
    Ok(AppState {
        config: Arc::new(config),
        search: Arc::new(search),
        http_client: client,
    })
}
