use std::sync::Arc;

use inkstone_infra::db::DbPool;
use inkstone_infra::search::SearchIndex;
use inkstone_runtime::config::PublicConfig;

#[derive(Clone)]
pub struct AppState {
    pub config: Arc<PublicConfig>,
    pub search: Arc<SearchIndex>,
    pub db: Option<DbPool>,
}
