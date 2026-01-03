use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{Duration, NaiveDate, Utc};
use serde::Serialize;
use thiserror::Error;

use crate::jobs::tasks::douban_crawl::{self, DoubanCategory};
use crate::jobs::JobError;
use crate::state::AppState;
use inkstone_infra::db::{
    count_recent_douban_items, fetch_douban_overview, fetch_recent_douban_items, DoubanRecentItem,
    DoubanRepoError,
};

const DEFAULT_RECENT_LIMIT: i64 = 20;
const MAX_RECENT_LIMIT: i64 = 200;

#[derive(Debug, serde::Deserialize)]
pub struct DoubanAdminQuery {
    #[serde(rename = "type")]
    pub item_type: Option<String>,
}

#[derive(Debug, serde::Deserialize)]
pub struct DoubanStatusQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Error)]
pub enum DoubanAdminError {
    #[error("db not configured")]
    DbUnavailable,
    #[error("douban not configured")]
    Disabled,
    #[error("invalid type")]
    InvalidType,
    #[error("db error: {0}")]
    Db(#[from] DoubanRepoError),
    #[error("job error: {0}")]
    Job(#[from] JobError),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Serialize)]
pub struct DoubanActionResponse {
    action: &'static str,
    overview: DoubanOverviewResponse,
}

#[derive(Debug, Serialize)]
pub struct DoubanOverviewResponse {
    enabled: bool,
    total: i64,
    with_date: i64,
    last_date: Option<String>,
    types: Vec<DoubanTypeEntry>,
    recent_24h: DoubanRecentSummary,
}

#[derive(Debug, Serialize)]
pub struct DoubanTypeEntry {
    #[serde(rename = "type")]
    item_type: String,
    count: i64,
}

#[derive(Debug, Serialize)]
pub struct DoubanRecentSummary {
    total: i64,
    items: Vec<DoubanRecentEntry>,
}

#[derive(Debug, Serialize)]
pub struct DoubanRecentEntry {
    #[serde(rename = "type")]
    item_type: String,
    id: String,
    title: String,
    date: Option<String>,
    updated_at: String,
    url: String,
}

pub async fn post_douban_refresh(
    State(state): State<AppState>,
    Query(query): Query<DoubanAdminQuery>,
) -> Result<Json<DoubanActionResponse>, DoubanAdminError> {
    ensure_douban_configured(&state)?;
    match parse_category(query.item_type.as_deref())? {
        Some(category) => douban_crawl::run_for_category(&state, false, category).await?,
        None => douban_crawl::run(&state, false).await?,
    }
    let overview = load_overview(&state, None).await?;
    Ok(Json(DoubanActionResponse {
        action: "refresh",
        overview,
    }))
}

pub async fn post_douban_rebuild(
    State(state): State<AppState>,
    Query(query): Query<DoubanAdminQuery>,
) -> Result<Json<DoubanActionResponse>, DoubanAdminError> {
    ensure_douban_configured(&state)?;
    match parse_category(query.item_type.as_deref())? {
        Some(category) => douban_crawl::run_for_category(&state, true, category).await?,
        None => douban_crawl::run(&state, true).await?,
    }
    let overview = load_overview(&state, None).await?;
    Ok(Json(DoubanActionResponse {
        action: "rebuild",
        overview,
    }))
}

pub async fn get_douban_status(
    State(state): State<AppState>,
    Query(query): Query<DoubanStatusQuery>,
) -> Result<Json<DoubanOverviewResponse>, DoubanAdminError> {
    let overview = load_overview(&state, query.limit).await?;
    Ok(Json(overview))
}

impl IntoResponse for DoubanAdminError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            DoubanAdminError::DbUnavailable | DoubanAdminError::Disabled => {
                (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
            }
            DoubanAdminError::InvalidType => (StatusCode::BAD_REQUEST, self.to_string()),
            DoubanAdminError::Db(_) | DoubanAdminError::Job(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}

fn ensure_douban_configured(state: &AppState) -> Result<(), DoubanAdminError> {
    if state.db.is_none() {
        return Err(DoubanAdminError::DbUnavailable);
    }
    if !is_douban_uid_configured(&state.config.douban_uid) {
        return Err(DoubanAdminError::Disabled);
    }
    Ok(())
}

fn is_douban_uid_configured(uid: &str) -> bool {
    !uid.trim().is_empty()
}

fn parse_category(value: Option<&str>) -> Result<Option<DoubanCategory>, DoubanAdminError> {
    let Some(raw) = value else {
        return Ok(None);
    };
    let trimmed = raw.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    match trimmed {
        "movie" => Ok(Some(DoubanCategory::Movie)),
        "book" => Ok(Some(DoubanCategory::Book)),
        "game" => Ok(Some(DoubanCategory::Game)),
        _ => Err(DoubanAdminError::InvalidType),
    }
}

async fn load_overview(
    state: &AppState,
    limit: Option<i64>,
) -> Result<DoubanOverviewResponse, DoubanAdminError> {
    let pool = state.db.as_ref().ok_or(DoubanAdminError::DbUnavailable)?;
    let overview = fetch_douban_overview(pool).await?;
    let limit = clamp_limit(limit);
    let since = Utc::now() - Duration::hours(24);
    let total = count_recent_douban_items(pool, since).await?;
    let items = fetch_recent_douban_items(pool, since, limit).await?;
    Ok(map_overview(state, overview, total, items))
}

fn map_overview(
    state: &AppState,
    overview: inkstone_infra::db::DoubanOverview,
    recent_total: i64,
    recent_items: Vec<DoubanRecentItem>,
) -> DoubanOverviewResponse {
    DoubanOverviewResponse {
        enabled: is_douban_uid_configured(&state.config.douban_uid),
        total: overview.total,
        with_date: overview.with_date,
        last_date: format_date(overview.last_date),
        types: overview
            .types
            .into_iter()
            .map(|entry| DoubanTypeEntry {
                item_type: entry.item_type,
                count: entry.count,
            })
            .collect(),
        recent_24h: DoubanRecentSummary {
            total: recent_total,
            items: recent_items
                .into_iter()
                .map(|item| map_recent_entry(item))
                .collect(),
        },
    }
}

fn format_date(value: Option<NaiveDate>) -> Option<String> {
    value.map(|date| date.to_string())
}

fn map_recent_entry(item: DoubanRecentItem) -> DoubanRecentEntry {
    let url = build_douban_url(&item.item_type, &item.id);
    DoubanRecentEntry {
        item_type: item.item_type,
        id: item.id,
        title: item.title,
        date: item.date.map(|value| value.to_string()),
        updated_at: item.updated_at.to_rfc3339(),
        url,
    }
}

fn build_douban_url(item_type: &str, id: &str) -> String {
    match item_type {
        "movie" => format!("https://movie.douban.com/subject/{}/", id),
        "book" => format!("https://book.douban.com/subject/{}/", id),
        "game" => format!("https://www.douban.com/game/{}/", id),
        other => format!("https://www.douban.com/{}/{}", other, id),
    }
}

fn clamp_limit(limit: Option<i64>) -> i64 {
    match limit {
        Some(value) if value > 0 => value.min(MAX_RECENT_LIMIT),
        _ => DEFAULT_RECENT_LIMIT,
    }
}

#[cfg(test)]
mod tests {
    use super::{is_douban_uid_configured, parse_category, DoubanCategory};

    #[test]
    fn douban_uid_requires_non_empty_value() {
        assert!(!is_douban_uid_configured(""));
        assert!(!is_douban_uid_configured("   "));
        assert!(is_douban_uid_configured("93562087"));
    }

    #[test]
    fn parse_category_accepts_known_values() {
        assert!(matches!(
            parse_category(Some("movie")).unwrap(),
            Some(DoubanCategory::Movie)
        ));
        assert!(matches!(
            parse_category(Some("book")).unwrap(),
            Some(DoubanCategory::Book)
        ));
        assert!(matches!(
            parse_category(Some("game")).unwrap(),
            Some(DoubanCategory::Game)
        ));
        assert!(parse_category(Some("")).unwrap().is_none());
        assert!(parse_category(None).unwrap().is_none());
    }
}
