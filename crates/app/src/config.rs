use std::net::SocketAddr;
use std::path::PathBuf;
use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub http_addr: SocketAddr,
    pub index_dir: PathBuf,
    pub feed_url: String,
    pub poll_interval: Duration,
    pub request_timeout: Duration,
    pub max_search_limit: usize,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid socket address: {0}")]
    InvalidSocket(String),
    #[error("invalid integer for {0}: {1}")]
    InvalidNumber(&'static str, String),
    #[error("invalid value for {0}: {1}")]
    InvalidValue(&'static str, String),
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let http_addr_raw = read_string("INKSTONE_HTTP_ADDR", "127.0.0.1:8080");
        let http_addr = http_addr_raw
            .parse()
            .map_err(|_| ConfigError::InvalidSocket(http_addr_raw.clone()))?;
        let index_dir = PathBuf::from(read_string("INKSTONE_INDEX_DIR", "./data/index"));
        let feed_url = read_string(
            "INKSTONE_FEED_URL",
            "https://velite-refactor.blog-8fo.pages.dev/atom.xml",
        );
        if feed_url.trim().is_empty() {
            return Err(ConfigError::InvalidValue(
                "INKSTONE_FEED_URL",
                feed_url,
            ));
        }
        let poll_interval_secs = read_u64("INKSTONE_POLL_INTERVAL_SECS", 300)?;
        let request_timeout_secs = read_u64("INKSTONE_REQUEST_TIMEOUT_SECS", 15)?;
        let max_search_limit = read_usize("INKSTONE_MAX_SEARCH_LIMIT", 50)?;

        Ok(Self {
            http_addr,
            index_dir,
            feed_url,
            poll_interval: Duration::from_secs(poll_interval_secs),
            request_timeout: Duration::from_secs(request_timeout_secs),
            max_search_limit,
        })
    }
}

fn read_string(key: &'static str, default: &'static str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn read_u64(key: &'static str, default: u64) -> Result<u64, ConfigError> {
    let raw = std::env::var(key).unwrap_or_else(|_| default.to_string());
    raw.parse()
        .map_err(|_| ConfigError::InvalidNumber(key, raw))
}

fn read_usize(key: &'static str, default: usize) -> Result<usize, ConfigError> {
    let raw = std::env::var(key).unwrap_or_else(|_| default.to_string());
    raw.parse()
        .map_err(|_| ConfigError::InvalidNumber(key, raw))
}
