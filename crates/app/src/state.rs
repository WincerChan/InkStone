use std::sync::Arc;
use std::time::Instant;

use chrono::{DateTime, Utc};
use reqwest::Client;
use tokio::sync::Mutex;

use crate::config::AppConfig;
use inkstone_infra::db::DbPool;
use inkstone_infra::search::SearchIndex;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub search: Arc<SearchIndex>,
    pub http_client: Client,
    pub db: Option<DbPool>,
    pub content_refresh_backoff: Arc<Mutex<ContentRefreshBackoff>>,
    pub admin_health: Arc<Mutex<AdminHealthState>>,
}

#[derive(Debug, Default)]
pub struct ContentRefreshBackoff {
    pub next_feed_at: Option<Instant>,
}

#[derive(Debug, Clone, Default)]
pub struct AdminHealthState {
    pub content_refresh_last_run: Option<DateTime<Utc>>,
    pub content_refresh_last_success: Option<DateTime<Utc>>,
    pub douban_crawl_last_run: Option<DateTime<Utc>>,
    pub douban_crawl_last_success: Option<DateTime<Utc>>,
    pub comments_sync_last_run: Option<DateTime<Utc>>,
    pub comments_sync_last_success: Option<DateTime<Utc>>,
    pub webhook_content_last_received: Option<DateTime<Utc>>,
    pub webhook_discussions_last_received: Option<DateTime<Utc>>,
}
