use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::state::AppState;
use inkstone_infra::db::{
    fetch_filter_usage, fetch_keyword_usage, fetch_search_daily, fetch_search_summary,
    fetch_sort_usage, fetch_top_categories, fetch_top_queries, fetch_top_tags, SearchDailyRow,
    SearchDimCountRow, SearchEventsRepoError, SearchFilterUsage, SearchSummaryRow,
    SearchTopQueryRow,
};

const DEFAULT_RANGE_DAYS: i64 = 30;
const DEFAULT_LIMIT: i64 = 20;
const MAX_LIMIT: i64 = 200;

#[derive(Debug, Deserialize)]
pub struct SearchStatsQuery {
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Error)]
pub enum SearchStatsError {
    #[error("invalid date")]
    InvalidDate,
    #[error("invalid date range")]
    InvalidDateRange,
    #[error("db not configured")]
    DbUnavailable,
    #[error("db error: {0}")]
    Db(#[from] SearchEventsRepoError),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Serialize)]
pub struct SearchStatsResponse {
    range: SearchRange,
    summary: SearchSummary,
    daily: Vec<SearchDailyEntry>,
    top_queries: Vec<SearchTopQueryEntry>,
    top_tags: Vec<SearchDimEntry>,
    top_categories: Vec<SearchDimEntry>,
    filter_usage: SearchFilterUsageEntry,
    sort_usage: Vec<SearchDimEntry>,
    keyword_usage: Vec<SearchKeywordEntry>,
}

#[derive(Debug, Serialize)]
pub struct SearchRange {
    from: String,
    to: String,
}

#[derive(Debug, Serialize)]
pub struct SearchSummary {
    total: i64,
    zero_results: i64,
    zero_result_rate: f64,
    avg_elapsed_ms: Option<i64>,
    p95_elapsed_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SearchDailyEntry {
    day: String,
    total: i64,
    zero_results: i64,
    zero_result_rate: f64,
    avg_elapsed_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SearchTopQueryEntry {
    query: String,
    count: i64,
    zero_results: i64,
    zero_result_rate: f64,
    avg_elapsed_ms: Option<i64>,
}

#[derive(Debug, Serialize)]
pub struct SearchDimEntry {
    value: String,
    count: i64,
}

#[derive(Debug, Serialize)]
pub struct SearchFilterUsageEntry {
    with_tags: i64,
    with_category: i64,
    with_range: i64,
}

#[derive(Debug, Serialize)]
pub struct SearchKeywordEntry {
    keyword_count: i32,
    count: i64,
}

pub async fn get_search_stats(
    State(state): State<AppState>,
    Query(query): Query<SearchStatsQuery>,
) -> Result<Json<SearchStatsResponse>, SearchStatsError> {
    let (from, to) = parse_range(query.from.as_deref(), query.to.as_deref())?;
    let limit = clamp_limit(query.limit);
    let pool = state.db.as_ref().ok_or(SearchStatsError::DbUnavailable)?;

    let summary = fetch_search_summary(pool, from, to).await?;
    let daily = fetch_search_daily(pool, from, to).await?;
    let top_queries = fetch_top_queries(pool, from, to, limit).await?;
    let top_tags = fetch_top_tags(pool, from, to, limit).await?;
    let top_categories = fetch_top_categories(pool, from, to, limit).await?;
    let filter_usage = fetch_filter_usage(pool, from, to).await?;
    let sort_usage = fetch_sort_usage(pool, from, to, limit).await?;
    let keyword_usage = fetch_keyword_usage(pool, from, to, limit).await?;

    Ok(Json(SearchStatsResponse {
        range: SearchRange {
            from: from.to_string(),
            to: to.to_string(),
        },
        summary: map_summary(summary),
        daily: daily.into_iter().map(map_daily).collect(),
        top_queries: top_queries.into_iter().map(map_top_query).collect(),
        top_tags: top_tags.into_iter().map(map_dim_entry).collect(),
        top_categories: top_categories.into_iter().map(map_dim_entry).collect(),
        filter_usage: map_filter_usage(filter_usage),
        sort_usage: sort_usage
            .into_iter()
            .map(|entry| SearchDimEntry {
                value: entry.sort,
                count: entry.count,
            })
            .collect(),
        keyword_usage: keyword_usage
            .into_iter()
            .map(|entry| SearchKeywordEntry {
                keyword_count: entry.keyword_count,
                count: entry.count,
            })
            .collect(),
    }))
}

fn map_summary(summary: SearchSummaryRow) -> SearchSummary {
    let rate = if summary.total == 0 {
        0.0
    } else {
        summary.zero_results as f64 / summary.total as f64
    };
    SearchSummary {
        total: summary.total,
        zero_results: summary.zero_results,
        zero_result_rate: rate,
        avg_elapsed_ms: round_ms(summary.avg_elapsed_ms),
        p95_elapsed_ms: round_ms(summary.p95_elapsed_ms),
    }
}

fn map_daily(entry: SearchDailyRow) -> SearchDailyEntry {
    let rate = if entry.total == 0 {
        0.0
    } else {
        entry.zero_results as f64 / entry.total as f64
    };
    SearchDailyEntry {
        day: entry.day.to_string(),
        total: entry.total,
        zero_results: entry.zero_results,
        zero_result_rate: rate,
        avg_elapsed_ms: round_ms(entry.avg_elapsed_ms),
    }
}

fn map_top_query(entry: SearchTopQueryRow) -> SearchTopQueryEntry {
    let rate = if entry.count == 0 {
        0.0
    } else {
        entry.zero_results as f64 / entry.count as f64
    };
    SearchTopQueryEntry {
        query: entry.query_norm,
        count: entry.count,
        zero_results: entry.zero_results,
        zero_result_rate: rate,
        avg_elapsed_ms: round_ms(entry.avg_elapsed_ms),
    }
}

fn map_dim_entry(entry: SearchDimCountRow) -> SearchDimEntry {
    SearchDimEntry {
        value: entry.value,
        count: entry.count,
    }
}

fn map_filter_usage(entry: SearchFilterUsage) -> SearchFilterUsageEntry {
    SearchFilterUsageEntry {
        with_tags: entry.with_tags,
        with_category: entry.with_category,
        with_range: entry.with_range,
    }
}

fn round_ms(value: Option<f64>) -> Option<i64> {
    value.map(|value| value.round() as i64)
}

fn parse_range(from: Option<&str>, to: Option<&str>) -> Result<(NaiveDate, NaiveDate), SearchStatsError> {
    let today = Utc::now().date_naive();
    let default_from = today - Duration::days(DEFAULT_RANGE_DAYS);
    let from = from
        .map(parse_date)
        .transpose()?
        .unwrap_or(default_from);
    let to = to.map(parse_date).transpose()?.unwrap_or(today);
    if from > to {
        return Err(SearchStatsError::InvalidDateRange);
    }
    Ok((from, to))
}

fn parse_date(raw: &str) -> Result<NaiveDate, SearchStatsError> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").map_err(|_| SearchStatsError::InvalidDate)
}

fn clamp_limit(limit: Option<i64>) -> i64 {
    match limit {
        Some(value) if value > 0 => value.min(MAX_LIMIT),
        _ => DEFAULT_LIMIT,
    }
}

impl IntoResponse for SearchStatsError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            SearchStatsError::InvalidDate | SearchStatsError::InvalidDateRange => {
                (StatusCode::BAD_REQUEST, self.to_string())
            }
            SearchStatsError::DbUnavailable => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            SearchStatsError::Db(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{parse_date, parse_range};

    #[test]
    fn parse_date_rejects_invalid() {
        assert!(parse_date("2025-13-01").is_err());
    }

    #[test]
    fn parse_range_defaults() {
        let (from, to) = parse_range(None, None).unwrap();
        assert!(from <= to);
    }
}
