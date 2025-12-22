use axum::middleware;
use axum::routing::get;
use axum::Router;

use crate::http::middleware::search_query_limit;
use crate::state::AppState;
use crate::http::routes::{douban, health, search};

pub fn build(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route(
            "/search",
            get(search::search)
                .layer(middleware::from_fn(search_query_limit::enforce_search_query_length)),
        )
        .route("/douban/marks", get(douban::marks_this_year))
        .with_state(state)
}
