use std::collections::HashSet;
use std::sync::Arc;
use std::time::Instant;

use reqwest::Client;
use tokio::sync::{Mutex, RwLock};

use crate::config::AppConfig;
use crate::kudos_cache::KudosCache;
use inkstone_infra::db::DbPool;
use inkstone_infra::search::SearchIndex;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub search: Arc<SearchIndex>,
    pub http_client: Client,
    pub db: Option<DbPool>,
    pub valid_paths: Arc<RwLock<HashSet<String>>>,
    pub kudos_cache: Arc<RwLock<KudosCache>>,
    pub content_refresh_backoff: Arc<Mutex<ContentRefreshBackoff>>,
}

#[derive(Debug, Default)]
pub struct ContentRefreshBackoff {
    pub next_feed_at: Option<Instant>,
    pub next_paths_at: Option<Instant>,
}
