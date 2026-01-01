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
    pub session_start_ts: Option<DateTime<Utc>>,
    pub ua_family: Option<String>,
    pub device: Option<String>,
    pub source_type: Option<String>,
    pub ref_host: Option<String>,
    pub country: Option<String>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct VisitorSession {
    pub session_start_ts: DateTime<Utc>,
    pub entry_source_type: Option<String>,
    pub entry_ref_host: Option<String>,
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
            session_start_ts,
            ua_family,
            device,
            source_type,
            ref_host,
            country
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        ON CONFLICT (page_instance_id) DO UPDATE SET
            user_stats_id = EXCLUDED.user_stats_id,
            path = EXCLUDED.path,
            site = EXCLUDED.site,
            ts = EXCLUDED.ts,
            session_start_ts = EXCLUDED.session_start_ts,
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
    .bind(record.session_start_ts)
    .bind(record.ua_family.as_deref())
    .bind(record.device.as_deref())
    .bind(record.source_type.as_deref())
    .bind(record.ref_host.as_deref())
    .bind(record.country.as_deref())
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn upsert_visitor(
    pool: &PgPool,
    site: &str,
    user_stats_id: &[u8],
    last_seen_ts: DateTime<Utc>,
    entry_source_type: Option<&str>,
    entry_ref_host: Option<&str>,
) -> Result<VisitorSession, AnalyticsRepoError> {
    let session = sqlx::query_as::<_, VisitorSession>(
        r#"
        INSERT INTO pulse_visitors (
            site,
            user_stats_id,
            first_seen_ts,
            last_seen_ts,
            session_start_ts,
            entry_source_type,
            entry_ref_host
        )
        VALUES ($1, $2, $3, $3, $3, $4, $5)
        ON CONFLICT (site, user_stats_id) DO UPDATE SET
            last_seen_ts = EXCLUDED.last_seen_ts,
            session_start_ts = CASE
                WHEN pulse_visitors.last_seen_ts < EXCLUDED.last_seen_ts - INTERVAL '30 minutes'
                    THEN EXCLUDED.session_start_ts
                ELSE pulse_visitors.session_start_ts
            END,
            entry_source_type = CASE
                WHEN pulse_visitors.last_seen_ts < EXCLUDED.last_seen_ts - INTERVAL '30 minutes'
                    THEN EXCLUDED.entry_source_type
                ELSE pulse_visitors.entry_source_type
            END,
            entry_ref_host = CASE
                WHEN pulse_visitors.last_seen_ts < EXCLUDED.last_seen_ts - INTERVAL '30 minutes'
                    THEN EXCLUDED.entry_ref_host
                ELSE pulse_visitors.entry_ref_host
            END
        RETURNING session_start_ts, entry_source_type, entry_ref_host
        "#,
    )
    .bind(site)
    .bind(user_stats_id)
    .bind(last_seen_ts)
    .bind(entry_source_type)
    .bind(entry_ref_host)
    .fetch_one(pool)
    .await?;
    Ok(session)
}

pub async fn touch_visitor_last_seen(
    pool: &PgPool,
    site: &str,
    user_stats_id: &[u8],
    last_seen_ts: DateTime<Utc>,
) -> Result<(), AnalyticsRepoError> {
    sqlx::query(
        r#"
        UPDATE pulse_visitors
        SET last_seen_ts = GREATEST(last_seen_ts, $3)
        WHERE site = $1 AND user_stats_id = $2
        "#,
    )
    .bind(site)
    .bind(user_stats_id)
    .bind(last_seen_ts)
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
