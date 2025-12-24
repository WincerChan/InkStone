use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, Method};
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use crate::http::middleware::{bid_cookie, search_query_limit};
use crate::state::AppState;
use crate::http::routes::{analytics, douban, health, kudos, search, webhook};

pub fn build(state: AppState) -> Router {
    let cors = build_cors(&state);
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
        .layer(cors)
        .with_state(state)
}

fn build_cors(state: &AppState) -> CorsLayer {
    let mut origins = Vec::new();
    for origin in state.config.cors_allow_origins.iter() {
        match HeaderValue::from_str(origin) {
            Ok(value) => origins.push(value),
            Err(_) => {
                tracing::warn!(origin = %origin, "invalid CORS origin ignored");
            }
        }
    }

    let cors = CorsLayer::new().allow_methods([Method::GET, Method::POST, Method::PUT, Method::OPTIONS]);

    if origins.is_empty() {
        cors.allow_origin(Any).allow_headers(Any)
    } else {
        cors.allow_origin(AllowOrigin::list(origins))
            .allow_credentials(true)
            .allow_headers([CONTENT_TYPE])
    }
}
