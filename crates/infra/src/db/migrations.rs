use sqlx::migrate::Migrator;

use super::DbPool;
use super::DbPoolError;

static MIGRATOR: Migrator = sqlx::migrate!("../../migrations");

pub async fn run_migrations(pool: &DbPool) -> Result<(), DbPoolError> {
    MIGRATOR.run(pool).await?;
    Ok(())
}
