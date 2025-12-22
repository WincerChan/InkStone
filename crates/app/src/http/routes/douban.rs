use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{Datelike, NaiveDate};
use serde::Serialize;
use thiserror::Error;

use crate::state::AppState;
use inkstone_infra::db::{fetch_douban_marks_by_range, DoubanMarkRecord, DoubanRepoError};

#[derive(Debug, Serialize)]
pub struct DoubanMark {
    pub title: String,
    pub poster: Option<String>,
    #[serde(rename = "type")]
    pub type_: String,
    pub date: String,
    pub url: String,
}

#[derive(Debug, Serialize)]
pub struct DoubanMarksResponse {
    pub total: usize,
    pub items: Vec<DoubanMark>,
}

#[derive(Debug, Error)]
pub enum DoubanApiError {
    #[error("db not configured")]
    DbUnavailable,
    #[error("db error: {0}")]
    Db(#[from] DoubanRepoError),
    #[error("invalid year: {0}")]
    InvalidYear(i32),
    #[error("unknown douban type: {0}")]
    UnknownType(String),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

pub async fn marks_this_year(
    State(state): State<AppState>,
) -> Result<Json<DoubanMarksResponse>, DoubanApiError> {
    let pool = state.db.as_ref().ok_or(DoubanApiError::DbUnavailable)?;
    let year = chrono::Local::now().date_naive().year();
    let (start, end) = year_bounds(year).ok_or(DoubanApiError::InvalidYear(year))?;
    let rows = fetch_douban_marks_by_range(pool, start, end).await?;
    let mut items = Vec::with_capacity(rows.len());
    for row in rows {
        items.push(map_mark(row)?);
    }
    Ok(Json(DoubanMarksResponse {
        total: items.len(),
        items,
    }))
}

fn map_mark(row: DoubanMarkRecord) -> Result<DoubanMark, DoubanApiError> {
    let url = build_douban_url(&row.item_type, &row.id)?;
    Ok(DoubanMark {
        title: row.title,
        poster: row.poster,
        type_: row.item_type,
        date: row.date.to_string(),
        url,
    })
}

fn year_bounds(year: i32) -> Option<(NaiveDate, NaiveDate)> {
    let start = NaiveDate::from_ymd_opt(year, 1, 1)?;
    let end = NaiveDate::from_ymd_opt(year + 1, 1, 1)?;
    Some((start, end))
}

fn build_douban_url(item_type: &str, id: &str) -> Result<String, DoubanApiError> {
    let url = match item_type {
        "movie" => format!("https://movie.douban.com/subject/{}/", id),
        "book" => format!("https://book.douban.com/subject/{}/", id),
        "game" => format!("https://www.douban.com/game/{}/", id),
        other => return Err(DoubanApiError::UnknownType(other.to_string())),
    };
    Ok(url)
}

impl IntoResponse for DoubanApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            DoubanApiError::DbUnavailable => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            DoubanApiError::Db(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            DoubanApiError::InvalidYear(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
            DoubanApiError::UnknownType(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{build_douban_url, year_bounds};

    #[test]
    fn year_bounds_returns_next_year_start() {
        let (start, end) = year_bounds(2025).expect("bounds");
        assert_eq!(start.to_string(), "2025-01-01");
        assert_eq!(end.to_string(), "2026-01-01");
    }

    #[test]
    fn build_douban_url_for_types() {
        assert_eq!(
            build_douban_url("movie", "1").unwrap(),
            "https://movie.douban.com/subject/1/"
        );
        assert_eq!(
            build_douban_url("book", "2").unwrap(),
            "https://book.douban.com/subject/2/"
        );
        assert_eq!(
            build_douban_url("game", "3").unwrap(),
            "https://www.douban.com/game/3/"
        );
    }
}
