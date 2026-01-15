use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, Method};
use axum::middleware;
use axum::routing::{get, post};
use axum::Router;
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use crate::http::middleware::{admin_auth, bid_cookie, search_query_limit};
use crate::http::HttpMode;
use crate::state::AppState;
use crate::http::routes::{admin, analytics, comments, douban, health, kudos, search, webhook};

pub fn build(state: AppState, mode: HttpMode) -> Router<AppState> {
    match mode {
        HttpMode::All => build_all(state),
        HttpMode::Public => build_public(state, true),
        HttpMode::Admin => build_admin(state, true),
    }
}

fn build_all(state: AppState) -> Router<AppState> {
    let public = build_public(state.clone(), true);
    let admin = build_admin(state, false);
    public.merge(admin)
}

fn build_public(state: AppState, include_health: bool) -> Router<AppState> {
    let cors = build_cors(&state);
    let mut router = Router::new();
    if include_health {
        router = router.route("/health", get(health::health));
    }
    router = router
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

fn build_admin(state: AppState, include_health: bool) -> Router<AppState> {
    let cors = build_cors(&state);
    let mut router = Router::new();
    if include_health {
        router = router.route("/health", get(health::health));
    }
    router = router
        .route("/v2/admin/login", post(admin::auth::login))
        .route("/v2/admin/pulse/sites", get(admin::pulse::list_pulse_sites))
        .route("/v2/admin/pulse/site", get(admin::pulse::get_pulse_site))
        .route("/v2/admin/pulse/active", get(admin::pulse::get_pulse_active))
        .route(
            "/v2/admin/pulse/active/summary",
            get(admin::pulse::get_pulse_active_summary),
        )
        .route("/v2/admin/health", get(admin::health::get_admin_health))
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
            "/v2/admin/kudos/status",
            get(admin::kudos::get_kudos_status),
        )
        .route(
            "/v2/admin/kudos/top_paths",
            get(admin::kudos::get_kudos_top_paths),
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
            admin_auth::require_admin,
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
    use super::{build, is_wildcard_origin, should_enable_cors};
    use crate::config::AppConfig;
    use crate::http::HttpMode;
    use crate::state::{AdminHealthState, AppState, ContentRefreshBackoff};
    use axum::http::{HeaderValue, StatusCode};
    use axum::Router;
    use inkstone_infra::search::SearchIndex;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use tokio::time::{sleep, Duration};

    fn build_state() -> AppState {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let index_dir = std::env::temp_dir().join(format!("inkstone-router-{suffix}"));
        let _ = std::fs::create_dir_all(&index_dir);
        let config = AppConfig {
            http_addr: "127.0.0.1:8080".parse().unwrap(),
            index_dir: index_dir.clone(),
            feed_url: "https://example.com/index.json".to_string(),
            poll_interval: std::time::Duration::from_secs(300),
            douban_poll_interval: std::time::Duration::from_secs(300),
            comments_sync_interval: std::time::Duration::from_secs(300),
            request_timeout: std::time::Duration::from_secs(15),
            max_search_limit: 50,
            database_url: None,
            douban_max_pages: 1,
            douban_uid: "1".to_string(),
            douban_cookie: "cookie".to_string(),
            douban_user_agent: "ua".to_string(),
            cookie_secret: Some("cookie".to_string()),
            stats_secret: Some("stats".to_string()),
            search_hash_secret: None,
            public_token_secret: Some("token".to_string()),
            github_webhook_secret: None,
            github_discussion_webhook_secret: None,
            github_app_id: None,
            github_app_installation_id: None,
            github_app_private_key: None,
            github_repo_owner: None,
            github_repo_name: None,
            github_discussion_category_id: None,
            cors_allow_origins: Vec::new(),
            pulse_allowed_slds: Vec::new(),
            admin_password_hash: None,
            admin_token_secret: None,
        };
        AppState {
            config: Arc::new(config),
            search: Arc::new(SearchIndex::open_or_create(&index_dir).unwrap()),
            http_client: reqwest::Client::new(),
            db: None,
            content_refresh_backoff: Arc::new(Mutex::new(ContentRefreshBackoff::default())),
            admin_health: Arc::new(Mutex::new(AdminHealthState::default())),
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
    async fn public_router_hides_admin_routes() {
        let state = build_state();
        let router = build(state.clone(), HttpMode::Public).with_state(state);
        let status = status_for(router, "/v2/admin/health").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn admin_router_hides_public_routes() {
        let state = build_state();
        let router = build(state.clone(), HttpMode::Admin).with_state(state);
        let status = status_for(router, "/v2/search").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
    }

    #[tokio::test]
    async fn all_router_exposes_public_and_admin_routes() {
        let state = build_state();
        let router = build(state.clone(), HttpMode::All).with_state(state);
        let public_status = status_for(router.clone(), "/v2/search").await;
        let admin_status = status_for(router, "/v2/admin/health").await;
        assert_ne!(public_status, StatusCode::NOT_FOUND);
        assert_ne!(admin_status, StatusCode::NOT_FOUND);
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
