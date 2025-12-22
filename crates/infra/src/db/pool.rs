use sqlx::postgres::PgPoolOptions;
use sqlx::PgPool;
use thiserror::Error;

pub type DbPool = PgPool;

#[derive(Debug, Error)]
pub enum DbPoolError {
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

pub fn connect_lazy(database_url: &str) -> Result<DbPool, DbPoolError> {
    Ok(PgPoolOptions::new()
        .max_connections(5)
        .connect_lazy(database_url)?)
}
