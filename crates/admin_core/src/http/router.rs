use axum::Router;
use axum::http::header::CONTENT_TYPE;
use axum::http::{HeaderValue, Method};
use axum::middleware;
use axum::routing::{get, post};
use tower_http::cors::{AllowOrigin, Any, CorsLayer};

use crate::http::middleware::admin_auth;
use crate::http::routes::{admin, webhook};
use crate::state::AdminState;

pub fn build(state: AdminState) -> Router<AdminState> {
    let cors = build_cors(&state);
    let mut router = Router::new()
        .route("/v2/admin/login", post(admin::auth::login))
        .route("/v2/admin/pulse/sites", get(admin::pulse::list_pulse_sites))
        .route("/v2/admin/pulse/site", get(admin::pulse::get_pulse_site))
        .route(
            "/v2/admin/pulse/active",
            get(admin::pulse::get_pulse_active),
        )
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

fn build_cors(state: &AdminState) -> Option<CorsLayer> {
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
    use crate::state::{AdminHealthState, AdminState, ContentRefreshBackoff};
    use axum::Router;
    use axum::http::{HeaderValue, StatusCode};
    use inkstone_infra::search::SearchIndex;
    use inkstone_runtime::config::AdminConfig;
    use std::sync::Arc;
    use tokio::net::TcpListener;
    use tokio::sync::Mutex;
    use tokio::time::{Duration, sleep};

    fn build_state() -> AdminState {
        let suffix = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let index_dir = std::env::temp_dir().join(format!("inkstone-admin-router-{suffix}"));
        let _ = std::fs::create_dir_all(&index_dir);
        let config = AdminConfig {
            http_addr: "127.0.0.1:8080".parse().unwrap(),
            index_dir: index_dir.clone(),
            feed_url: "https://example.com/index.json".to_string(),
            poll_interval: std::time::Duration::from_secs(300),
            douban_poll_interval: std::time::Duration::from_secs(300),
            comments_sync_interval: std::time::Duration::from_secs(300),
            request_timeout: std::time::Duration::from_secs(15),
            database_url: None,
            douban_max_pages: 1,
            douban_uid: "1".to_string(),
            douban_cookie: "cookie".to_string(),
            douban_user_agent: "ua".to_string(),
            douban_poster_r2_endpoint: None,
            douban_poster_r2_bucket: None,
            douban_poster_r2_access_key_id: None,
            douban_poster_r2_secret_access_key: None,
            douban_poster_r2_public_base_url: None,
            douban_poster_r2_region: "auto".to_string(),
            douban_poster_r2_prefix: "douban".to_string(),
            cookie_secret: Some("cookie".to_string()),
            stats_secret: Some("stats".to_string()),
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
            admin_password_hash: None,
            admin_token_secret: None,
        };
        AdminState {
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
    async fn admin_router_hides_public_routes() {
        let state = build_state();
        let router = build(state.clone()).with_state(state);
        let status = status_for(router, "/v2/search").await;
        assert_eq!(status, StatusCode::NOT_FOUND);
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
        assert!(should_enable_cors(
            false,
            &[HeaderValue::from_static("https://example.com")]
        ));
    }
}
