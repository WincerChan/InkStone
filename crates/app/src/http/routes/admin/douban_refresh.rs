use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::NaiveDate;
use serde::Serialize;
use thiserror::Error;

use crate::jobs::tasks::douban_crawl;
use crate::jobs::JobError;
use crate::state::AppState;
use inkstone_infra::db::{fetch_douban_overview, DoubanRepoError};

#[derive(Debug, Error)]
pub enum DoubanAdminError {
    #[error("db not configured")]
    DbUnavailable,
    #[error("douban not configured")]
    Disabled,
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
}

#[derive(Debug, Serialize)]
pub struct DoubanTypeEntry {
    #[serde(rename = "type")]
    item_type: String,
    count: i64,
}

pub async fn post_douban_refresh(
    State(state): State<AppState>,
) -> Result<Json<DoubanActionResponse>, DoubanAdminError> {
    ensure_douban_configured(&state)?;
    douban_crawl::run(&state, false).await?;
    let overview = load_overview(&state).await?;
    Ok(Json(DoubanActionResponse {
        action: "refresh",
        overview,
    }))
}

pub async fn post_douban_rebuild(
    State(state): State<AppState>,
) -> Result<Json<DoubanActionResponse>, DoubanAdminError> {
    ensure_douban_configured(&state)?;
    douban_crawl::run(&state, true).await?;
    let overview = load_overview(&state).await?;
    Ok(Json(DoubanActionResponse {
        action: "rebuild",
        overview,
    }))
}

pub async fn get_douban_status(
    State(state): State<AppState>,
) -> Result<Json<DoubanOverviewResponse>, DoubanAdminError> {
    let pool = state.db.as_ref().ok_or(DoubanAdminError::DbUnavailable)?;
    let overview = fetch_douban_overview(pool).await?;
    Ok(Json(map_overview(&state, overview)))
}

impl IntoResponse for DoubanAdminError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            DoubanAdminError::DbUnavailable | DoubanAdminError::Disabled => {
                (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
            }
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

async fn load_overview(state: &AppState) -> Result<DoubanOverviewResponse, DoubanAdminError> {
    let pool = state.db.as_ref().ok_or(DoubanAdminError::DbUnavailable)?;
    let overview = fetch_douban_overview(pool).await?;
    Ok(map_overview(state, overview))
}

fn map_overview(
    state: &AppState,
    overview: inkstone_infra::db::DoubanOverview,
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
    }
}

fn format_date(value: Option<NaiveDate>) -> Option<String> {
    value.map(|date| date.to_string())
}

#[cfg(test)]
mod tests {
    use super::is_douban_uid_configured;

    #[test]
    fn douban_uid_requires_non_empty_value() {
        assert!(!is_douban_uid_configured(""));
        assert!(!is_douban_uid_configured("   "));
        assert!(is_douban_uid_configured("93562087"));
    }
}
