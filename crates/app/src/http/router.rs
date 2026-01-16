use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, Method};
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use crate::http::middleware::{bid_cookie, search_query_limit};
use crate::http::routes::{analytics, comments, douban, health, kudos, search};
use crate::state::AppState;

pub fn build(state: AppState) -> Router<AppState> {
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
        .route(
            "/v2/kudos",
            get(kudos::get_kudos)
                .put(kudos::put_kudos)
                .post(kudos::put_kudos),
        )
        .route("/v2/pulse/pv", post(analytics::post_pv))
        .route("/v2/pulse/engage", post(analytics::post_engage))
        .layer(middleware::from_fn_with_state(
            state.clone(),
            bid_cookie::ensure_bid_cookie,
        ));
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

    let cors =
        CorsLayer::new().allow_methods([Method::GET, Method::POST, Method::PUT, Method::OPTIONS]);

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
    use super::{build, is_wildcard_origin, should_enable_cors};
    use crate::state::AppState;
    use axum::http::{HeaderValue, StatusCode};
    use axum::Router;
    use inkstone_infra::search::SearchIndex;
    use inkstone_runtime::config::PublicConfig;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::time::{sleep, Duration};

    fn build_state() -> AppState {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let index_dir = std::env::temp_dir().join(format!("inkstone-router-{suffix}"));
        let _ = std::fs::create_dir_all(&index_dir);
        let config = PublicConfig {
            http_addr: "127.0.0.1:8080".parse().unwrap(),
            index_dir: index_dir.clone(),
            max_search_limit: 50,
            database_url: None,
            cookie_secret: Some("cookie".to_string()),
            stats_secret: Some("stats".to_string()),
            search_hash_secret: None,
            public_token_secret: Some("token".to_string()),
            cors_allow_origins: Vec::new(),
            pulse_allowed_slds: Vec::new(),
        };
        AppState {
            config: Arc::new(config),
            search: Arc::new(SearchIndex::open_or_create(&index_dir).unwrap()),
            db: None,
        }
    }

    async fn status_for(router: Router<()>, path: &str) -> StatusCode {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let url = format!("http://{addr}{path}");
        let client = reqwest::Client::new();
        let status = request_status(&client, &url).await;
        server.abort();
        let _ = server.await;
        status
    }

    async fn request_status(client: &reqwest::Client, url: &str) -> StatusCode {
        for _ in 0..3 {
            match client.get(url).send().await {
                Ok(response) => return response.status(),
                Err(_) => sleep(Duration::from_millis(10)).await,
            }
        }
        client.get(url).send().await.unwrap().status()
    }

    #[tokio::test]
    async fn public_router_serves_public_routes() {
        let state = build_state();
        let router = build(state.clone()).with_state(state);
        let status = status_for(router, "/v2/search").await;
        assert_ne!(status, StatusCode::NOT_FOUND);
    }

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
