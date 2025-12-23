use sqlx::{PgPool, Row};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum KudosRepoError {
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

pub async fn insert_kudos(
    pool: &PgPool,
    path: &str,
    interaction_id: &[u8],
) -> Result<bool, KudosRepoError> {
    let result = sqlx::query(
        r#"
        INSERT INTO kudos (path, interaction_id)
        VALUES ($1, $2)
        ON CONFLICT (path, interaction_id) DO NOTHING
        "#,
    )
    .bind(path)
    .bind(interaction_id)
    .execute(pool)
    .await?;
    Ok(result.rows_affected() > 0)
}

pub async fn count_kudos(pool: &PgPool, path: &str) -> Result<i64, KudosRepoError> {
    let row = sqlx::query(
        r#"
        SELECT COUNT(*) AS count
        FROM kudos
        WHERE path = $1
        "#,
    )
    .bind(path)
    .fetch_one(pool)
    .await?;
    let count: i64 = row.try_get("count")?;
    Ok(count)
}

pub async fn has_kudos(
    pool: &PgPool,
    path: &str,
    interaction_id: &[u8],
) -> Result<bool, KudosRepoError> {
    let row = sqlx::query(
        r#"
        SELECT 1
        FROM kudos
        WHERE path = $1 AND interaction_id = $2
        LIMIT 1
        "#,
    )
    .bind(path)
    .bind(interaction_id)
    .fetch_optional(pool)
    .await?;
    Ok(row.is_some())
}
