use axum::Router;
use inkstone_admin_core::jobs::JobError;
use inkstone_admin_core::{jobs, state::AdminState, wiring as admin_wiring};
use inkstone_app::state::AppState;
use inkstone_infra::db::run_migrations;
use inkstone_runtime::config::{AdminConfig, ConfigError, PublicConfig, load_dotenv};
use thiserror::Error;
use tokio::net::TcpListener;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("public wiring error: {0}")]
    PublicWiring(#[from] inkstone_app::wiring::WiringError),
    #[error("admin wiring error: {0}")]
    AdminWiring(#[from] inkstone_admin_core::wiring::WiringError),
    #[error("db error: {0}")]
    Db(#[from] inkstone_infra::db::DbPoolError),
    #[error("job error: {0}")]
    Jobs(#[from] JobError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    run().await
}

async fn run() -> Result<(), AppError> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    load_dotenv()?;
    let admin_config = AdminConfig::from_env()?;
    let public_config = PublicConfig::from_env()?;

    let admin_state = build_admin_state(admin_config).await?;
    let public_state = build_public_state(public_config).await?;

    let router = build_router(public_state, admin_state.clone());
    let addr = admin_state.config.http_addr;
    let listener = TcpListener::bind(addr).await?;

    info!(%addr, "all-in-one http server starting");

    let http_task = tokio::spawn(async move {
        axum::serve(listener, router).await.map_err(AppError::from)
    });

    let worker_state = admin_state;
    let worker_task = tokio::spawn(async move {
        info!("worker scheduler starting");
        jobs::start(worker_state, false).await.map_err(AppError::from)
    });

    let shutdown = shutdown_signal();

    tokio::select! {
        _ = shutdown => {
            info!("shutdown signal received");
        }
        res = http_task => {
            res??;
        }
        res = worker_task => {
            res??;
        }
    }

    Ok(())
}

async fn build_admin_state(config: AdminConfig) -> Result<AdminState, AppError> {
    let state = admin_wiring::build_admin_state(config)?;
    if let Some(pool) = state.db.as_ref() {
        run_migrations(pool).await?;
    }
    Ok(state)
}

async fn build_public_state(config: PublicConfig) -> Result<AppState, AppError> {
    let state = inkstone_app::wiring::build_state_readonly(config)?;
    if let Some(pool) = state.db.as_ref() {
        run_migrations(pool).await?;
    }
    Ok(state)
}

fn build_router(public_state: AppState, admin_state: AdminState) -> Router<()> {
    let public_router = inkstone_app::http::router::build(public_state.clone())
        .with_state(public_state);
    let admin_router = inkstone_admin_core::http::router::build(admin_state.clone())
        .with_state(admin_state);
    Router::new().merge(public_router).merge(admin_router)
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        error!(error = %err, "failed to install ctrl-c handler");
    }
}

#[cfg(test)]
mod tests {
    use super::{build_router, build_admin_state, build_public_state};
    use axum::http::StatusCode;
    use axum::Router;
    use inkstone_runtime::config::{AdminConfig, PublicConfig};
    use reqwest::Client;
    use std::path::PathBuf;
    use tokio::net::TcpListener;
    use tokio::time::{sleep, Duration};

    fn temp_dir(name: &str) -> PathBuf {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!("inkstone-allinone-{name}-{nanos}"))
    }

    fn public_config(index_dir: PathBuf) -> PublicConfig {
        PublicConfig {
            http_addr: "127.0.0.1:8080".parse().unwrap(),
            index_dir,
            max_search_limit: 50,
            database_url: None,
            cookie_secret: Some("cookie".to_string()),
            stats_secret: Some("stats".to_string()),
            search_hash_secret: None,
            public_token_secret: Some("token".to_string()),
            cors_allow_origins: Vec::new(),
            pulse_allowed_slds: Vec::new(),
        }
    }

    fn admin_config(index_dir: PathBuf) -> AdminConfig {
        AdminConfig {
            http_addr: "127.0.0.1:8080".parse().unwrap(),
            index_dir,
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
            admin_token_secret: Some("secret".to_string()),
        }
    }

    async fn status_for(router: Router<()>, path: &str) -> StatusCode {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();
        let server = tokio::spawn(async move {
            axum::serve(listener, router).await.unwrap();
        });
        let url = format!("http://{addr}{path}");
        let client = Client::new();
        let status = request_status(&client, &url).await;
        server.abort();
        let _ = server.await;
        status
    }

    async fn request_status(client: &Client, url: &str) -> StatusCode {
        for _ in 0..3 {
            match client.get(url).send().await {
                Ok(response) => return response.status(),
                Err(_) => sleep(Duration::from_millis(10)).await,
            }
        }
        client.get(url).send().await.unwrap().status()
    }

    #[tokio::test]
    async fn allinone_router_serves_public_and_admin_routes() {
        let index_dir = temp_dir("router");
        let admin_state = build_admin_state(admin_config(index_dir.clone()))
            .await
            .unwrap();
        let public_state = build_public_state(public_config(index_dir))
            .await
            .unwrap();
        let router = build_router(public_state, admin_state);

        let public_status = status_for(router.clone(), "/v2/search").await;
        let admin_status = status_for(router, "/v2/admin/health").await;

        assert_ne!(public_status, StatusCode::NOT_FOUND);
        assert_ne!(admin_status, StatusCode::NOT_FOUND);
    }
}
