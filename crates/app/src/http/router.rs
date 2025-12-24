use axum::middleware;
use axum::routing::{get, post};
use axum::Router;

use crate::http::middleware::{bid_cookie, search_query_limit};
use crate::state::AppState;
use crate::http::routes::{analytics, douban, health, kudos, search, webhook};

pub fn build(state: AppState) -> Router {
    Router::new()
        .route("/health", get(health::health))
        .route(
            "/v2/search",
            get(search::search)
                .layer(middleware::from_fn(search_query_limit::enforce_search_query_length)),
        )
        .route("/v2/douban/marks", get(douban::marks_this_year))
        .route("/v2/kudos", get(kudos::get_kudos).put(kudos::put_kudos))
        .route("/v2/pulse/pv", post(analytics::post_pv))
        .route("/v2/pulse/engage", post(analytics::post_engage))
        .route("/webhook/github", post(webhook::github_webhook))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            bid_cookie::ensure_bid_cookie,
        ))
        .with_state(state)
}
