use std::sync::Arc;

use reqwest::Client;

use crate::config::AppConfig;
use inkstone_infra::db::DbPool;
use inkstone_infra::search::SearchIndex;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub search: Arc<SearchIndex>,
    pub http_client: Client,
    pub db: Option<DbPool>,
}
