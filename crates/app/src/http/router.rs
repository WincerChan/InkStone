use axum::routing::get;
use axum::Router;

use crate::state::AppState;
use crate::http::routes::{health, search};

pub fn build(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route("/search", get(search::search))
        .with_state(state)
}
