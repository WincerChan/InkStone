use std::sync::Arc;

use crate::config::AppConfig;
use inkstone_infra::db::DbPool;
use inkstone_infra::search::SearchIndex;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<AppConfig>,
    pub search: Arc<SearchIndex>,
    pub db: Option<DbPool>,
}
