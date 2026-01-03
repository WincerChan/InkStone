use chrono::{DateTime, Utc};
use sqlx::{PgPool, Row};
use thiserror::Error;

#[derive(Debug, Error)]
pub enum KudosRepoError {
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

#[derive(Debug)]
pub struct KudosEntry {
    pub path: String,
    pub interaction_id: Vec<u8>,
}

#[derive(Debug, Clone)]
pub struct KudosOverview {
    pub total: i64,
    pub paths: i64,
}

#[derive(Debug, Clone)]
pub struct KudosPathCount {
    pub path: String,
    pub count: i64,
}

#[derive(Debug, Clone)]
pub struct KudosRecentPath {
    pub path: String,
    pub count: i64,
    pub last_at: DateTime<Utc>,
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

pub async fn load_all_kudos(pool: &PgPool) -> Result<Vec<KudosEntry>, KudosRepoError> {
    let rows = sqlx::query(
        r#"
        SELECT path, interaction_id
        FROM kudos
        "#,
    )
    .fetch_all(pool)
    .await?;
    let mut entries = Vec::with_capacity(rows.len());
    for row in rows {
        let path: String = row.try_get("path")?;
        let interaction_id: Vec<u8> = row.try_get("interaction_id")?;
        entries.push(KudosEntry {
            path,
            interaction_id,
        });
    }
    Ok(entries)
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

pub async fn fetch_kudos_overview(pool: &PgPool) -> Result<KudosOverview, KudosRepoError> {
    let total: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM kudos")
        .fetch_one(pool)
        .await?;
    let paths: i64 = sqlx::query_scalar("SELECT COUNT(DISTINCT path) FROM kudos")
        .fetch_one(pool)
        .await?;
    Ok(KudosOverview { total, paths })
}

pub async fn fetch_kudos_top_paths(
    pool: &PgPool,
    limit: i64,
) -> Result<Vec<KudosPathCount>, KudosRepoError> {
    let rows = sqlx::query(
        r#"
        SELECT path, COUNT(*) AS count
        FROM kudos
        GROUP BY path
        ORDER BY count DESC, path ASC
        LIMIT $1
        "#,
    )
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(KudosPathCount {
            path: row.try_get("path")?,
            count: row.try_get("count")?,
        });
    }
    Ok(items)
}

pub async fn count_recent_kudos(
    pool: &PgPool,
    since: DateTime<Utc>,
) -> Result<i64, KudosRepoError> {
    let total: i64 = sqlx::query_scalar(
        r#"
        SELECT COUNT(*)
        FROM kudos
        WHERE created_at >= $1
        "#,
    )
    .bind(since)
    .fetch_one(pool)
    .await?;
    Ok(total)
}

pub async fn fetch_recent_kudos_paths(
    pool: &PgPool,
    since: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<KudosRecentPath>, KudosRepoError> {
    let rows = sqlx::query(
        r#"
        SELECT path,
               COUNT(*) AS count,
               MAX(created_at) AS last_at
        FROM kudos
        WHERE created_at >= $1
        GROUP BY path
        ORDER BY last_at DESC, path ASC
        LIMIT $2
        "#,
    )
    .bind(since)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(KudosRecentPath {
            path: row.try_get("path")?,
            count: row.try_get("count")?,
            last_at: row.try_get("last_at")?,
        });
    }
    Ok(items)
}
