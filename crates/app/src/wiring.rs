use std::sync::Arc;

use thiserror::Error;

use crate::state::AppState;
use inkstone_infra::db::{connect_lazy, DbPoolError};
use inkstone_infra::search::{SearchIndex, SearchIndexError};
use inkstone_runtime::config::PublicConfig;

#[derive(Debug, Error)]
pub enum WiringError {
    #[error("search index error: {0}")]
    SearchIndex(#[from] SearchIndexError),
    #[error("http client error: {0}")]
    HttpClient(#[from] reqwest::Error),
    #[error("db pool error: {0}")]
    DbPool(#[from] DbPoolError),
}

pub fn build_state(config: PublicConfig) -> Result<AppState, WiringError> {
    let search = SearchIndex::open_or_create(&config.index_dir)?;
    build_state_with_search(config, search)
}

pub fn build_state_readonly(config: PublicConfig) -> Result<AppState, WiringError> {
    let search = SearchIndex::open_existing(&config.index_dir)?;
    build_state_with_search(config, search)
}

fn build_state_with_search(
    config: PublicConfig,
    search: SearchIndex,
) -> Result<AppState, WiringError> {
    let db = match config.database_url.as_deref() {
        Some(url) => Some(connect_lazy(url)?),
        None => None,
    };
    Ok(AppState {
        config: Arc::new(config),
        search: Arc::new(search),
        db,
    })
}

#[cfg(test)]
mod tests {
    use super::{build_state_readonly, WiringError};
    use inkstone_infra::search::{SearchIndex, SearchIndexError};
    use inkstone_runtime::config::PublicConfig;
    use std::path::PathBuf;
    use std::sync::Arc;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("inkstone-wiring-{name}-{nanos}"))
    }

    fn build_config(index_dir: PathBuf) -> PublicConfig {
        PublicConfig {
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
        }
    }

    #[test]
    fn build_state_readonly_requires_existing_index() {
        let index_dir = temp_dir("missing");
        let config = build_config(index_dir.clone());
        let result = build_state_readonly(config);
        assert!(matches!(
            result,
            Err(WiringError::SearchIndex(SearchIndexError::MissingIndex(_)))
        ));
        let _ = std::fs::remove_dir_all(index_dir);
    }

    #[test]
    fn build_state_readonly_opens_existing_index() {
        let index_dir = temp_dir("existing");
        std::fs::create_dir_all(&index_dir).unwrap();
        SearchIndex::open_or_create(&index_dir).unwrap();
        let config = build_config(index_dir.clone());
        let state = build_state_readonly(config).unwrap();
        assert!(Arc::strong_count(&state.config) >= 1);
        let _ = std::fs::remove_dir_all(index_dir);
    }
}
