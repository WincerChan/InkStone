pub mod http;
pub mod state;
pub mod wiring;

use thiserror::Error;
use tracing_subscriber::EnvFilter;

use crate::http::HttpError;
use crate::wiring::WiringError;
use inkstone_infra::db::run_migrations;
use inkstone_runtime::config::{ConfigError, PublicConfig, load_dotenv};

#[derive(Debug, Error)]
pub enum AppError {
    #[error("config error: {0}")]
    Config(#[from] ConfigError),
    #[error("wiring error: {0}")]
    Wiring(#[from] WiringError),
    #[error("db error: {0}")]
    Db(#[from] inkstone_infra::db::DbPoolError),
    #[error("http error: {0}")]
    Http(#[from] HttpError),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub async fn run_public() -> Result<(), AppError> {
    let env_filter = EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(env_filter).init();

    load_dotenv()?;
    let config = PublicConfig::from_env()?;
    let state = wiring::build_state_readonly(config)?;
    if let Some(pool) = state.db.as_ref() {
        run_migrations(pool).await?;
    }
    let addr = state.config.http_addr;
    http::serve(addr, state).await?;
    Ok(())
}
