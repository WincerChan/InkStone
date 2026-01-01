use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, Method};
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use crate::http::middleware::{bid_cookie, search_query_limit};
use crate::state::AppState;
use crate::http::routes::{admin, analytics, comments, douban, health, kudos, search, webhook};

pub fn build(state: AppState) -> Router {
    let cors = build_cors(&state);
    let mut router = Router::new()
        .route("/health", get(health::health))
        .route(
            "/v2/search",
            get(search::search)
                .layer(middleware::from_fn(search_query_limit::enforce_search_query_length)),
        )
        .route("/v2/douban/marks", get(douban::marks_this_year))
        .route("/v2/comments", get(comments::get_comments))
        .route("/v2/kudos", get(kudos::get_kudos).put(kudos::put_kudos))
        .route("/v2/pulse/pv", post(analytics::post_pv))
        .route("/v2/pulse/engage", post(analytics::post_engage))
        .route("/v2/admin/pulse/sites", get(admin::pulse::list_pulse_sites))
        .route("/v2/admin/pulse/site", get(admin::pulse::get_pulse_site))
        .route(
            "/v2/admin/search/stats",
            get(admin::search_stats::get_search_stats),
        )
        .route(
            "/v2/admin/comments/status",
            get(admin::comments_sync::get_comments_status),
        )
        .route(
            "/v2/admin/comments/sync",
            post(admin::comments_sync::post_comments_sync),
        )
        .route(
            "/v2/admin/comments/rebuild",
            post(admin::comments_sync::post_comments_rebuild),
        )
        .route(
            "/v2/admin/douban/status",
            get(admin::douban_refresh::get_douban_status),
        )
        .route(
            "/v2/admin/douban/refresh",
            post(admin::douban_refresh::post_douban_refresh),
        )
        .route(
            "/v2/admin/douban/rebuild",
            post(admin::douban_refresh::post_douban_rebuild),
        )
        .route(
            "/v2/admin/search/reindex",
            post(admin::search_reindex::post_search_reindex),
        )
        .route(
            "/v2/admin/search/refresh",
            post(admin::search_reindex::post_search_refresh),
        )
        .route(
            "/v2/admin/search/status",
            get(admin::search_reindex::get_search_status),
        )
        .route("/webhook/github/content", post(webhook::github_webhook))
        .route(
            "/webhook/github/discussions",
            post(webhook::github_discussion_webhook),
        )
        .layer(middleware::from_fn_with_state(
            state.clone(),
            bid_cookie::ensure_bid_cookie,
        ))
        .with_state(state);
    if let Some(cors) = cors {
        router = router.layer(cors);
    }
    router
}

fn build_cors(state: &AppState) -> Option<CorsLayer> {
    let mut origins = Vec::new();
    let mut allow_any = false;
    for origin in state.config.cors_allow_origins.iter() {
        if is_wildcard_origin(origin) {
            allow_any = true;
            break;
        }
        match HeaderValue::from_str(origin.trim()) {
            Ok(value) => origins.push(value),
            Err(_) => {
                tracing::warn!(origin = %origin, "invalid CORS origin ignored");
            }
        }
    }

    let cors = CorsLayer::new().allow_methods([Method::GET, Method::POST, Method::PUT, Method::OPTIONS]);

    if !should_enable_cors(allow_any, &origins) {
        return None;
    }

    if allow_any {
        Some(cors.allow_origin(Any).allow_headers(Any))
    } else {
        Some(
            cors.allow_origin(AllowOrigin::list(origins))
                .allow_credentials(true)
                .allow_headers([CONTENT_TYPE]),
        )
    }
}

fn is_wildcard_origin(origin: &str) -> bool {
    origin.trim() == "*"
}

fn should_enable_cors(allow_any: bool, origins: &[HeaderValue]) -> bool {
    allow_any || !origins.is_empty()
}

#[cfg(test)]
mod tests {
    use super::{is_wildcard_origin, should_enable_cors};
    use axum::http::HeaderValue;

    #[test]
    fn wildcard_origin_matches_trimmed_star() {
        assert!(is_wildcard_origin("*"));
        assert!(is_wildcard_origin(" * "));
        assert!(!is_wildcard_origin("https://example.com"));
    }

    #[test]
    fn cors_enablement_requires_origin_or_wildcard() {
        assert!(!should_enable_cors(false, &[]));
        assert!(should_enable_cors(true, &[]));
        assert!(should_enable_cors(false, &[HeaderValue::from_static("https://example.com")]));
    }
}
