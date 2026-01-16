use std::sync::Arc;

use thiserror::Error;

use crate::config::AppConfig;
use crate::state::AppState;
use inkstone_infra::db::{connect_lazy, DbPoolError};
use inkstone_infra::search::{SearchIndex, SearchIndexError};

#[derive(Debug, Error)]
pub enum WiringError {
    #[error("search index error: {0}")]
    SearchIndex(#[from] SearchIndexError),
    #[error("http client error: {0}")]
    HttpClient(#[from] reqwest::Error),
    #[error("db pool error: {0}")]
    DbPool(#[from] DbPoolError),
}

pub fn build_state(config: AppConfig) -> Result<AppState, WiringError> {
    let search = SearchIndex::open_or_create(&config.index_dir)?;
    build_state_with_search(config, search)
}

pub fn build_state_readonly(config: AppConfig) -> Result<AppState, WiringError> {
    let search = SearchIndex::open_existing(&config.index_dir)?;
    build_state_with_search(config, search)
}

fn build_state_with_search(config: AppConfig, search: SearchIndex) -> Result<AppState, WiringError> {
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
    use crate::config::AppConfig;
    use inkstone_infra::search::{SearchIndex, SearchIndexError};
    use std::path::PathBuf;
    use std::sync::Arc;

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("inkstone-wiring-{name}-{nanos}"))
    }

    fn build_config(index_dir: PathBuf) -> AppConfig {
        AppConfig {
            http_addr: "127.0.0.1:8080".parse().unwrap(),
            index_dir,
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
            public_token_secret: Some("token".to_string()),
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
