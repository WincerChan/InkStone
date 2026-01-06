use chrono::NaiveDate;
use sqlx::PgPool;
use thiserror::Error;

#[derive(Debug, Error)]
pub enum SearchEventsRepoError {
    #[error("sqlx error: {0}")]
    Sqlx(#[from] sqlx::Error),
}

#[derive(Debug, Clone)]
pub struct SearchEvent {
    pub query_raw: String,
    pub query_norm: String,
    pub keyword_count: i32,
    pub tags: Vec<String>,
    pub category: Option<String>,
    pub range_start: Option<NaiveDate>,
    pub range_end: Option<NaiveDate>,
    pub sort: String,
    pub kind: String,
    pub search_user_hash: Option<String>,
    pub result_total: i32,
    pub elapsed_ms: i32,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchSummaryRow {
    pub total: i64,
    pub zero_results: i64,
    pub avg_elapsed_ms: Option<f64>,
    pub p95_elapsed_ms: Option<f64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchDailyRow {
    pub day: NaiveDate,
    pub total: i64,
    pub zero_results: i64,
    pub avg_elapsed_ms: Option<f64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchTopQueryRow {
    pub query_norm: String,
    pub requests: i64,
    pub count: i64,
    pub zero_results: i64,
    pub avg_elapsed_ms: Option<f64>,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchDimCount {
    pub value: String,
    pub count: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchFilterUsage {
    pub with_tags: i64,
    pub with_category: i64,
    pub with_range: i64,
}

#[derive(Debug, Clone, sqlx::FromRow)]
pub struct SearchKeywordUsage {
    pub keyword_count: i32,
    pub count: i64,
}

pub async fn insert_search_event(
    pool: &PgPool,
    event: &SearchEvent,
) -> Result<(), SearchEventsRepoError> {
    sqlx::query(
        r#"
        INSERT INTO search_events (
            query_raw,
            query_norm,
            keyword_count,
            tags,
            category,
            range_start,
            range_end,
            sort,
            kind,
            search_user_hash,
            result_total,
            elapsed_ms
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, $10, $11, $12)
        "#,
    )
    .bind(&event.query_raw)
    .bind(&event.query_norm)
    .bind(event.keyword_count)
    .bind(&event.tags)
    .bind(event.category.as_deref())
    .bind(event.range_start)
    .bind(event.range_end)
    .bind(&event.sort)
    .bind(&event.kind)
    .bind(event.search_user_hash.as_deref())
    .bind(event.result_total)
    .bind(event.elapsed_ms)
    .execute(pool)
    .await?;
    Ok(())
}

pub async fn fetch_recent_search_query(
    pool: &PgPool,
    search_user_hash: &str,
    within_secs: i64,
) -> Result<Option<String>, SearchEventsRepoError> {
    let query_norm: Option<String> = sqlx::query_scalar(
        r#"
        SELECT query_norm
        FROM search_events
        WHERE kind = 'search'
          AND search_user_hash = $1
          AND ts >= NOW() - ($2 * interval '1 second')
        ORDER BY ts DESC
        LIMIT 1
        "#,
    )
    .bind(search_user_hash)
    .bind(within_secs)
    .fetch_optional(pool)
    .await?;
    Ok(query_norm)
}

pub async fn fetch_search_summary(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<SearchSummaryRow, SearchEventsRepoError> {
    let row = sqlx::query_as::<_, SearchSummaryRow>(
        r#"
        SELECT
            COUNT(*)::bigint AS total,
            COUNT(*) FILTER (WHERE result_total = 0)::bigint AS zero_results,
            AVG(elapsed_ms)::double precision AS avg_elapsed_ms,
            percentile_cont(0.95) WITHIN GROUP (ORDER BY elapsed_ms)::double precision
                AS p95_elapsed_ms
        FROM search_events
        WHERE day BETWEEN $1 AND $2
          AND kind = 'search'
        "#,
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn fetch_search_daily(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<Vec<SearchDailyRow>, SearchEventsRepoError> {
    let rows = sqlx::query_as::<_, SearchDailyRow>(
        r#"
        SELECT
            day,
            COUNT(*)::bigint AS total,
            COUNT(*) FILTER (WHERE result_total = 0)::bigint AS zero_results,
            AVG(elapsed_ms)::double precision AS avg_elapsed_ms
        FROM search_events
        WHERE day BETWEEN $1 AND $2
          AND kind = 'search'
        GROUP BY day
        ORDER BY day
        "#,
    )
    .bind(from)
    .bind(to)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_top_queries(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
    limit: i64,
) -> Result<Vec<SearchTopQueryRow>, SearchEventsRepoError> {
    let rows = sqlx::query_as::<_, SearchTopQueryRow>(
        r#"
        SELECT
            query_norm,
            COUNT(*)::bigint AS requests,
            (
                COUNT(DISTINCT search_user_hash)
                + COUNT(*) FILTER (WHERE search_user_hash IS NULL)
            )::bigint AS count,
            COUNT(*) FILTER (WHERE result_total = 0)::bigint AS zero_results,
            AVG(elapsed_ms)::double precision AS avg_elapsed_ms
        FROM search_events
        WHERE day BETWEEN $1 AND $2
          AND kind = 'search'
        GROUP BY query_norm
        ORDER BY count DESC
        LIMIT $3
        "#,
    )
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_top_tags(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
    limit: i64,
) -> Result<Vec<SearchDimCount>, SearchEventsRepoError> {
    let rows = sqlx::query_as::<_, SearchDimCount>(
        r#"
        SELECT
            tag AS value,
            COUNT(*)::bigint AS count
        FROM search_events
        JOIN LATERAL unnest(tags) AS tag ON TRUE
        WHERE day BETWEEN $1 AND $2
          AND tag <> ''
          AND kind = 'search'
        GROUP BY tag
        ORDER BY count DESC
        LIMIT $3
        "#,
    )
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_top_categories(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
    limit: i64,
) -> Result<Vec<SearchDimCount>, SearchEventsRepoError> {
    let rows = sqlx::query_as::<_, SearchDimCount>(
        r#"
        SELECT
            category AS value,
            COUNT(*)::bigint AS count
        FROM search_events
        WHERE day BETWEEN $1 AND $2
          AND category IS NOT NULL
          AND category <> ''
          AND kind = 'search'
        GROUP BY category
        ORDER BY count DESC
        LIMIT $3
        "#,
    )
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}

pub async fn fetch_filter_usage(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
) -> Result<SearchFilterUsage, SearchEventsRepoError> {
    let row = sqlx::query_as::<_, SearchFilterUsage>(
        r#"
        SELECT
            COUNT(*) FILTER (WHERE array_length(tags, 1) > 0)::bigint AS with_tags,
            COUNT(*) FILTER (WHERE category IS NOT NULL AND category <> '')::bigint AS with_category,
            COUNT(*) FILTER (WHERE range_start IS NOT NULL OR range_end IS NOT NULL)::bigint
                AS with_range
        FROM search_events
        WHERE day BETWEEN $1 AND $2
          AND kind = 'search'
        "#,
    )
    .bind(from)
    .bind(to)
    .fetch_one(pool)
    .await?;
    Ok(row)
}

pub async fn fetch_keyword_usage(
    pool: &PgPool,
    from: NaiveDate,
    to: NaiveDate,
    limit: i64,
) -> Result<Vec<SearchKeywordUsage>, SearchEventsRepoError> {
    let rows = sqlx::query_as::<_, SearchKeywordUsage>(
        r#"
        SELECT
            keyword_count,
            COUNT(*)::bigint AS count
        FROM search_events
        WHERE day BETWEEN $1 AND $2
          AND kind = 'search'
        GROUP BY keyword_count
        ORDER BY count DESC
        LIMIT $3
        "#,
    )
    .bind(from)
    .bind(to)
    .bind(limit)
    .fetch_all(pool)
    .await?;
    Ok(rows)
}
