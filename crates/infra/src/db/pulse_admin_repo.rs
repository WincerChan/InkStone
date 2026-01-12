use chrono::{DateTime, NaiveDate, Utc};
use sqlx::PgPool;

use super::AnalyticsRepoError;

#[derive(Debug, Clone, Default)]
pub struct PulseFilters {
    pub device: Option<String>,
    pub ua_family: Option<String>,
    pub source_type: Option<String>,
    pub ref_host: Option<String>,
    pub country: Option<String>,
}

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
pub struct PulseActiveMinuteUv {
    pub minute: DateTime<Utc>,
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
    filters: &PulseFilters,
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
          AND ($4 IS NULL OR device = $4)
          AND ($5 IS NULL OR LOWER(ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(country) = $8)
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn fetch_daily(
    pool: &PgPool,
    site: &str,
    from: NaiveDate,
    to: NaiveDate,
    filters: &PulseFilters,
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
          AND ($4 IS NULL OR device = $4)
          AND ($5 IS NULL OR LOWER(ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(country) = $8)
        GROUP BY day
        ORDER BY day
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_top_paths(
    pool: &PgPool,
    site: &str,
    from: NaiveDate,
    to: NaiveDate,
    filters: &PulseFilters,
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
          AND ($4 IS NULL OR device = $4)
          AND ($5 IS NULL OR LOWER(ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(country) = $8)
        GROUP BY path
        ORDER BY pv DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
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
    filters: &PulseFilters,
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
          AND ($4 IS NULL OR device = $4)
          AND ($5 IS NULL OR LOWER(ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(country) = $8)
        GROUP BY value
        ORDER BY pv DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
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
    filters: &PulseFilters,
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
          AND ($4 IS NULL OR device = $4)
          AND ($5 IS NULL OR LOWER(ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(country) = $8)
        GROUP BY value
        ORDER BY pv DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
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
    filters: &PulseFilters,
    limit: i64,
) -> Result<Vec<PulseDimStats>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimStats>(
        r#"
        SELECT
            COALESCE(NULLIF(entry_source_type, ''), 'unknown') AS value,
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv
        FROM pulse_events
        WHERE site = $1
          AND day BETWEEN $2 AND $3
          AND ($4 IS NULL OR device = $4)
          AND ($5 IS NULL OR LOWER(ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(country) = $8)
        GROUP BY value
        ORDER BY pv DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
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
    filters: &PulseFilters,
    limit: i64,
) -> Result<Vec<PulseDimStats>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimStats>(
        r#"
        SELECT
            COALESCE(NULLIF(entry_ref_host, ''), 'unknown') AS value,
            COUNT(*)::bigint AS pv,
            COUNT(DISTINCT user_stats_id)::bigint AS uv
        FROM pulse_events
        WHERE site = $1
          AND day BETWEEN $2 AND $3
          AND ($4 IS NULL OR device = $4)
          AND ($5 IS NULL OR LOWER(ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(country) = $8)
        GROUP BY value
        ORDER BY pv DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
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
    filters: &PulseFilters,
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
          AND ($4 IS NULL OR device = $4)
          AND ($5 IS NULL OR LOWER(ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(country) = $8)
        GROUP BY value
        ORDER BY pv DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
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
    filters: &PulseFilters,
) -> Result<PulseActiveTotals, AnalyticsRepoError> {
    let row = sqlx::query_as::<_, PulseActiveTotals>(
        r#"
        SELECT
            (
                SELECT COUNT(*)::bigint
                FROM pulse_events
                WHERE site = $1 AND ts BETWEEN $2 AND $3
                  AND ($4 IS NULL OR device = $4)
                  AND ($5 IS NULL OR LOWER(ua_family) = $5)
                  AND ($6 IS NULL OR entry_source_type = $6)
                  AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
                  AND ($8 IS NULL OR LOWER(country) = $8)
            ) AS pv,
            (
                SELECT COUNT(*)::bigint
                FROM pulse_visitors
                WHERE site = $1 AND last_seen_ts BETWEEN $2 AND $3
                  AND ($4 IS NULL OR last_device = $4)
                  AND ($5 IS NULL OR LOWER(last_ua_family) = $5)
                  AND ($6 IS NULL OR entry_source_type = $6)
                  AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
                  AND ($8 IS NULL OR LOWER(last_country) = $8)
            ) AS uv
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn fetch_active_top_paths(
    pool: &PgPool,
    site: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    filters: &PulseFilters,
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
          AND ($4 IS NULL OR device = $4)
          AND ($5 IS NULL OR LOWER(ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(country) = $8)
        GROUP BY path
        ORDER BY pv DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_active_minute_uv(
    pool: &PgPool,
    site: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    filters: &PulseFilters,
) -> Result<Vec<PulseActiveMinuteUv>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseActiveMinuteUv>(
        r#"
        SELECT
            date_trunc('minute', last_seen_ts) AS minute,
            COUNT(*)::bigint AS uv
        FROM pulse_visitors
        WHERE site = $1
          AND last_seen_ts BETWEEN $2 AND $3
          AND ($4 IS NULL OR last_device = $4)
          AND ($5 IS NULL OR LOWER(last_ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(last_country) = $8)
        GROUP BY minute
        ORDER BY minute
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_active_device_counts(
    pool: &PgPool,
    site: &str,
    from: DateTime<Utc>,
    to: DateTime<Utc>,
    filters: &PulseFilters,
    limit: i64,
) -> Result<Vec<PulseDimCount>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimCount>(
        r#"
        SELECT
            COALESCE(NULLIF(last_device, ''), 'unknown') AS value,
            COUNT(*)::bigint AS count
        FROM pulse_visitors
        WHERE site = $1
          AND last_seen_ts BETWEEN $2 AND $3
          AND ($4 IS NULL OR last_device = $4)
          AND ($5 IS NULL OR LOWER(last_ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(last_country) = $8)
        GROUP BY value
        ORDER BY count DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
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
    filters: &PulseFilters,
    limit: i64,
) -> Result<Vec<PulseDimCount>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimCount>(
        r#"
        SELECT
            COALESCE(NULLIF(last_ua_family, ''), 'unknown') AS value,
            COUNT(*)::bigint AS count
        FROM pulse_visitors
        WHERE site = $1
          AND last_seen_ts BETWEEN $2 AND $3
          AND ($4 IS NULL OR last_device = $4)
          AND ($5 IS NULL OR LOWER(last_ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(last_country) = $8)
        GROUP BY value
        ORDER BY count DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
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
    filters: &PulseFilters,
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
          AND ($4 IS NULL OR last_device = $4)
          AND ($5 IS NULL OR LOWER(last_ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(last_country) = $8)
        GROUP BY entry_source_type
        ORDER BY count DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
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
    filters: &PulseFilters,
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
          AND ($4 IS NULL OR last_device = $4)
          AND ($5 IS NULL OR LOWER(last_ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(last_country) = $8)
        GROUP BY entry_ref_host
        ORDER BY count DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
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
    filters: &PulseFilters,
    limit: i64,
) -> Result<Vec<PulseDimCount>, AnalyticsRepoError> {
    let rows = sqlx::query_as::<_, PulseDimCount>(
        r#"
        SELECT
            COALESCE(NULLIF(last_country, ''), 'unknown') AS value,
            COUNT(*)::bigint AS count
        FROM pulse_visitors
        WHERE site = $1
          AND last_seen_ts BETWEEN $2 AND $3
          AND ($4 IS NULL OR last_device = $4)
          AND ($5 IS NULL OR LOWER(last_ua_family) = $5)
          AND ($6 IS NULL OR entry_source_type = $6)
          AND ($7 IS NULL OR LOWER(entry_ref_host) = $7)
          AND ($8 IS NULL OR LOWER(last_country) = $8)
        GROUP BY value
        ORDER BY count DESC
        LIMIT $9
        "#,
    )
    .bind(site)
    .bind(from)
    .bind(to)
    .bind(filters.device.as_deref())
    .bind(filters.ua_family.as_deref())
    .bind(filters.source_type.as_deref())
    .bind(filters.ref_host.as_deref())
    .bind(filters.country.as_deref())
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
