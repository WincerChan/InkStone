use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;

use super::AnalyticsRepoError;

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PulseSiteOverview {
    pub site: String,
    pub pv: i64,
    pub uv: i64,
    pub last_seen_at: Option<DateTime<Utc>>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PulseTotals {
    pub pv: i64,
    pub uv: i64,
    pub avg_duration_ms: Option<f64>,
    pub total_duration_ms: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PulseActiveTotals {
    pub pv: i64,
    pub uv: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PulseDailyStat {
    pub day: NaiveDate,
    pub pv: i64,
    pub uv: i64,
    pub avg_duration_ms: Option<f64>,
    pub total_duration_ms: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PulseTopPath {
    pub path: String,
    pub pv: i64,
    pub uv: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PulseDimCount {
    pub value: String,
    pub count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct PulseDimStats {
    pub value: String,
    pub pv: i64,
    pub uv: i64,
}

pub async fn list_sites(pool: &PgPool) -> Result<Vec<PulseSiteOverview>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseSiteOverview>(
        r#"
        SELECT
            site,
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv,
            MAX(ts) AS last_seen_at
        FROM pulse_events
        WHERE site IS NOT NULL
        GROUP BY site
        ORDER BY last_seen_at DESC
        "#,
    )
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_totals(
    pool: &PgPool,
    site: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<PulseTotals, AnalyticsRepoError> {
    let row = sqlx::query_as::<_, PulseTotals>(
        r#"
        SELECT
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv,
            AVG(duration_ms)::double precision AS avg_duration_ms,
            COALESCE(SUM(duration_ms), 0)::bigint AS total_duration_ms
        FROM pulse_events
        WHERE site = $1 AND day BETWEEN $2 AND $3
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn fetch_daily(
    pool: &PgPool,
    site: &str,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<PulseDailyStat>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDailyStat>(
        r#"
        SELECT
            day,
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv,
            AVG(duration_ms)::double precision AS avg_duration_ms,
            COALESCE(SUM(duration_ms), 0)::bigint AS total_duration_ms
        FROM pulse_events
        WHERE site = $1 AND day BETWEEN $2 AND $3
        GROUP BY day
        ORDER BY day
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_top_paths(
    pool: &PgPool,
    site: &str,
    from: NaiveDate,
    to: NaiveDate,
    limit: i64,
) -> Result<Vec<PulseTopPath>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseTopPath>(
        r#"
        SELECT
            path,
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv
        FROM pulse_events
        WHERE site = $1
          AND day BETWEEN $2 AND $3
          AND path IS NOT NULL
          AND path <> ''
        GROUP BY path
        ORDER BY pv DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_device_stats(
    pool: &PgPool,
    site: &str,
    from: NaiveDate,
    to: NaiveDate,
    limit: i64,
) -> Result<Vec<PulseDimStats>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimStats>(
        r#"
        SELECT
            COALESCE(NULLIF(device, ''), 'unknown') AS value,
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv
        FROM pulse_events
        WHERE site = $1
          AND day BETWEEN $2 AND $3
        GROUP BY value
        ORDER BY pv DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_ua_stats(
    pool: &PgPool,
    site: &str,
    from: NaiveDate,
    to: NaiveDate,
    limit: i64,
) -> Result<Vec<PulseDimStats>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimStats>(
        r#"
        SELECT
            COALESCE(NULLIF(ua_family, ''), 'unknown') AS value,
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv
        FROM pulse_events
        WHERE site = $1
          AND day BETWEEN $2 AND $3
        GROUP BY value
        ORDER BY pv DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_source_stats(
    pool: &PgPool,
    site: &str,
    from: NaiveDate,
    to: NaiveDate,
    limit: i64,
) -> Result<Vec<PulseDimStats>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimStats>(
        r#"
        SELECT
            COALESCE(NULLIF(source_type, ''), 'unknown') AS value,
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv
        FROM pulse_events
        WHERE site = $1
          AND day BETWEEN $2 AND $3
        GROUP BY value
        ORDER BY pv DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_ref_host_stats(
    pool: &PgPool,
    site: &str,
    from: NaiveDate,
    to: NaiveDate,
    limit: i64,
) -> Result<Vec<PulseDimStats>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimStats>(
        r#"
        SELECT
            COALESCE(NULLIF(ref_host, ''), 'unknown') AS value,
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv
        FROM pulse_events
        WHERE site = $1
          AND day BETWEEN $2 AND $3
        GROUP BY value
        ORDER BY pv DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_country_stats(
    pool: &PgPool,
    site: &str,
    from: NaiveDate,
    to: NaiveDate,
    limit: i64,
) -> Result<Vec<PulseDimStats>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimStats>(
        r#"
        SELECT
            COALESCE(NULLIF(country, ''), 'unknown') AS value,
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv
        FROM pulse_events
        WHERE site = $1
          AND day BETWEEN $2 AND $3
        GROUP BY value
        ORDER BY pv DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_active_totals(
    pool: &PgPool,
    site: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
) -> Result<PulseActiveTotals, AnalyticsRepoError> {
    let row = sqlx::query_as::<_, PulseActiveTotals>(
        r#"
        SELECT
            COUNT(*)::bigint AS pv,
            COUNT(*)::bigint AS uv
        FROM pulse_visitors
        WHERE site = $1 AND last_seen_ts BETWEEN $2 AND $3
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn fetch_active_top_paths(
    pool: &PgPool,
    site: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<PulseTopPath>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseTopPath>(
        r#"
        SELECT
            path,
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv
        FROM pulse_events
        WHERE site = $1
          AND ts BETWEEN $2 AND $3
          AND path IS NOT NULL
          AND path <> ''
        GROUP BY path
        ORDER BY pv DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_active_device_counts(
    pool: &PgPool,
    site: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<PulseDimCount>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimCount>(
        r#"
        WITH per_user AS (
            SELECT
                user_stats_id,
                COALESCE(NULLIF(device, ''), 'unknown') AS device,
                ROW_NUMBER() OVER (PARTITION BY user_stats_id ORDER BY ts DESC) AS rn
            FROM pulse_events
            WHERE site = $1
              AND ts BETWEEN $2 AND $3
              AND user_stats_id IS NOT NULL
        )
        SELECT
            device AS value,
            COUNT(*)::bigint AS count
        FROM per_user
        WHERE rn = 1
        GROUP BY device
        ORDER BY count DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_active_ua_counts(
    pool: &PgPool,
    site: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<PulseDimCount>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimCount>(
        r#"
        WITH per_user AS (
            SELECT
                user_stats_id,
                COALESCE(NULLIF(ua_family, ''), 'unknown') AS ua_family,
                ROW_NUMBER() OVER (PARTITION BY user_stats_id ORDER BY ts DESC) AS rn
            FROM pulse_events
            WHERE site = $1
              AND ts BETWEEN $2 AND $3
              AND user_stats_id IS NOT NULL
        )
        SELECT
            ua_family AS value,
            COUNT(*)::bigint AS count
        FROM per_user
        WHERE rn = 1
        GROUP BY ua_family
        ORDER BY count DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_active_source_counts(
    pool: &PgPool,
    site: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<PulseDimCount>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimCount>(
        r#"
        SELECT
            entry_source_type AS value,
            COUNT(*)::bigint AS count
        FROM pulse_visitors
        WHERE site = $1
          AND last_seen_ts BETWEEN $2 AND $3
          AND entry_source_type IS NOT NULL
          AND entry_source_type <> ''
        GROUP BY entry_source_type
        ORDER BY count DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_active_ref_host_counts(
    pool: &PgPool,
    site: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<PulseDimCount>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimCount>(
        r#"
        SELECT
            entry_ref_host AS value,
            COUNT(*)::bigint AS count
        FROM pulse_visitors
        WHERE site = $1
          AND last_seen_ts BETWEEN $2 AND $3
          AND entry_ref_host IS NOT NULL
          AND entry_ref_host <> ''
        GROUP BY entry_ref_host
        ORDER BY count DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_active_country_counts(
    pool: &PgPool,
    site: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    limit: i64,
) -> Result<Vec<PulseDimCount>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimCount>(
        r#"
        WITH per_user AS (
            SELECT
                user_stats_id,
                COALESCE(NULLIF(country, ''), 'unknown') AS country,
                ROW_NUMBER() OVER (PARTITION BY user_stats_id ORDER BY ts DESC) AS rn
            FROM pulse_events
            WHERE site = $1
              AND ts BETWEEN $2 AND $3
              AND user_stats_id IS NOT NULL
        )
        SELECT
            country AS value,
            COUNT(*)::bigint AS count
        FROM per_user
        WHERE rn = 1
        GROUP BY country
        ORDER BY count DESC
        LIMIT $4
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
