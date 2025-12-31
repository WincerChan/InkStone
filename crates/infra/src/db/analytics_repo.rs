use chrono::{DateTime, Utc};
use sqlx::PgPool;
use thiserror::Error;
use uuid::Uuid;

#[derive(Debug, Error)]
pub enum AnalyticsRepoError {
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct PageViewRecord {
    pub page_instance_id: Uuid,
    pub duration_ms: Option<i64>,
    pub user_stats_id: Option<Vec<u8>>,
    pub path: Option<String>,
    pub site: Option<String>,
    pub ts: DateTime<Utc>,
    pub ua_family: Option<String>,
    pub device: Option<String>,
    pub source_type: Option<String>,
    pub ref_host: Option<String>,
    pub country: Option<String>,
}

pub async fn upsert_page_view(
    pool: &PgPool,
    record: &PageViewRecord,
) -> Result<(), AnalyticsRepoError> {
    sqlx::query(
        r#"
        INSERT INTO pulse_events (
            page_instance_id,
            duration_ms,
            user_stats_id,
            path,
            site,
            ts,
            ua_family,
            device,
            source_type,
            ref_host,
            country
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11)
        ON CONFLICT (page_instance_id) DO UPDATE SET
            user_stats_id = EXCLUDED.user_stats_id,
            path = EXCLUDED.path,
            site = EXCLUDED.site,
            ts = EXCLUDED.ts,
            ua_family = EXCLUDED.ua_family,
            device = EXCLUDED.device,
            source_type = EXCLUDED.source_type,
            ref_host = EXCLUDED.ref_host,
            country = EXCLUDED.country,
            duration_ms = pulse_events.duration_ms
        "#,
    )
    .bind(record.page_instance_id)
    .bind(record.duration_ms)
    .bind(record.user_stats_id.as_deref())
    .bind(record.path.as_deref())
    .bind(record.site.as_deref())
    .bind(record.ts)
    .bind(record.ua_family.as_deref())
    .bind(record.device.as_deref())
    .bind(record.source_type.as_deref())
    .bind(record.ref_host.as_deref())
    .bind(record.country.as_deref())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert_engagement(
    pool: &PgPool,
    page_instance_id: Uuid,
    duration_ms: i64,
) -> Result<(), AnalyticsRepoError> {
    sqlx::query(
        r#"
        INSERT INTO pulse_events (page_instance_id, duration_ms)
        VALUES ($1, $2)
        ON CONFLICT (page_instance_id) DO UPDATE SET
            duration_ms = EXCLUDED.duration_ms
        "#,
    )
    .bind(page_instance_id)
    .bind(duration_ms)
    .execute(pool)
    .await?;
    Ok(())
}
