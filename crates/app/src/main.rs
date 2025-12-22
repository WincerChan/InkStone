mod cli;
mod config;
mod http;
mod jobs;
mod state;
mod wiring;

use clap::Parser;
use thiserror::Error;
use tracing::{error, info};
use tracing_subscriber::EnvFilter;

use crate::cli::Cli;
use crate::config::ConfigError;
use crate::http::HttpError;
use crate::jobs::JobError;
use crate::wiring::WiringError;
use inkstone_infra::db::run_migrations;

#[derive(Debug, Error)]
pub enum AppError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("invalid cli: {0}")]
    InvalidCli(String),
    #[error("wiring error: {0}")]
    Wiring(#[from] WiringError),
    #[error("db error: {0}")]
    Db(#[from] inkstone_infra::db::DbPoolError),
    #[error("http error: {0}")]
    Http(#[from] HttpError),
    #[error("job error: {0}")]
    Jobs(#[from] JobError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
    #[error("task join error: {0}")]
    Join(#[from] tokio::task::JoinError),
}

#[tokio::main]
async fn main() -> Result<(), AppError> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    let cli = Cli::parse();
    config::load_dotenv()?;
    let config = config::AppConfig::from_env()?;
    if cli.rebuild_schema && !cli.mode.run_worker() {
        return Err(AppError::InvalidCli(
            "rebuild-schema requires worker mode".to_string(),
        ));
    }
    if cli.rebuild_schema {
        let index_dir = &config.index_dir;
        if index_dir.exists() {
            info!(
                index_dir = %index_dir.display(),
                "rebuild schema requested, deleting index dir"
            );
            std::fs::remove_dir_all(index_dir)?;
        }
    }
    let state = wiring::build_state(config)?;
    if let Some(pool) = state.db.as_ref() {
        run_migrations(pool).await?;
    }

    let mut api_task = None;
    let mut worker_task = None;

    if cli.mode.run_api() {
        let addr = state.config.http_addr;
        let http_state = state.clone();
        api_task = Some(tokio::spawn(async move {
            info!(%addr, "http server starting");
            http::serve(addr, http_state).await
        }));
    }

    if cli.mode.run_worker() {
        let worker_state = state.clone();
        let rebuild = cli.rebuild || cli.rebuild_schema;
        worker_task = Some(tokio::spawn(async move {
            info!("worker scheduler starting");
            jobs::start(worker_state, rebuild).await
        }));
    }

    if api_task.is_none() && worker_task.is_none() {
        info!("no mode selected; exiting");
        return Ok(());
    }

    let shutdown = shutdown_signal();

    match (api_task, worker_task) {
        (Some(api), Some(worker)) => {
            tokio::select! {
                _ = shutdown => {
                    info!("shutdown signal received");
                }
                res = api => {
                    res??;
                }
                res = worker => {
                    res??;
                }
            }
        }
        (Some(api), None) => {
            tokio::select! {
                _ = shutdown => {
                    info!("shutdown signal received");
                }
                res = api => {
                    res??;
                }
            }
        }
        (None, Some(worker)) => {
            tokio::select! {
                _ = shutdown => {
                    info!("shutdown signal received");
                }
                res = worker => {
                    res??;
                }
            }
        }
        (None, None) => {}
    }

    Ok(())
}

async fn shutdown_signal() {
    if let Err(err) = tokio::signal::ctrl_c().await {
        error!(error = %err, "failed to install ctrl-c handler");
    }
}
