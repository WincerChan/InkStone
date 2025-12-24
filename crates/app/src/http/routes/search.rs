use std::time::Instant;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

use crate::state::AppState;
use inkstone_core::domain::search::{SearchHit, SearchResult};
use inkstone_infra::search::{parse_query, QueryParseError, SearchIndexError};

const MAX_QUERY_LEN: usize = 256;

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub total: usize,
    pub hits: Vec<SearchHit>,
    pub elapsed_ms: u128,
}

#[derive(Debug, Error)]
pub enum SearchApiError {
    #[error("invalid query: {0}")]
    Query(#[from] QueryParseError),
    #[error("query too long (max {0} chars)")]
    QueryTooLong(usize),
    #[error("search failure: {0}")]
    Search(#[from] SearchIndexError),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

pub async fn search(
    State(state): State<AppState>,
    Query(params): Query<SearchParams>,
) -> Result<Json<SearchResponse>, SearchApiError> {
    let started_at = Instant::now();
    let query_text = params.q.unwrap_or_default();
    if let Err(err) = enforce_query_length(&query_text) {
        warn!(
            query_text = %query_text,
            elapsed_ms = started_at.elapsed().as_millis(),
            error = %err,
            "search request rejected"
        );
        return Err(err);
    }
    let limit = params
        .limit
        .unwrap_or(20)
        .min(state.config.max_search_limit);
    let offset = params.offset.unwrap_or(0);

    let query = match parse_query(&query_text) {
        Ok(query) => query,
        Err(err) => {
            warn!(
                query_text = %query_text,
                elapsed_ms = started_at.elapsed().as_millis(),
                error = %err,
                "search query parse failed"
            );
            return Err(err.into());
        }
    };
    let result: SearchResult = match state.search.search(&query, limit, offset) {
        Ok(result) => result,
        Err(err) => {
            warn!(
                query_text = %query_text,
                elapsed_ms = started_at.elapsed().as_millis(),
                error = %err,
                "search execution failed"
            );
            return Err(err.into());
        }
    };

    info!(
        query_text = %query_text,
        limit,
        offset,
        total = result.total,
        elapsed_ms = started_at.elapsed().as_millis(),
        "search request completed"
    );
    Ok(Json(SearchResponse {
        total: result.total,
        hits: result.hits,
        elapsed_ms: started_at.elapsed().as_millis(),
    }))
}

fn enforce_query_length(query_text: &str) -> Result<(), SearchApiError> {
    if query_text.chars().count() > MAX_QUERY_LEN {
        return Err(SearchApiError::QueryTooLong(MAX_QUERY_LEN));
    }
    Ok(())
}

impl IntoResponse for SearchApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            SearchApiError::Query(err) => (StatusCode::BAD_REQUEST, err.to_string()),
            SearchApiError::QueryTooLong(_) => (StatusCode::BAD_REQUEST, self.to_string()),
            SearchApiError::Search(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{enforce_query_length, SearchApiError, MAX_QUERY_LEN};

    #[test]
    fn query_length_rejects_long_text() {
        let query = "a".repeat(MAX_QUERY_LEN + 1);
        let err = enforce_query_length(&query).unwrap_err();
        assert!(matches!(err, SearchApiError::QueryTooLong(_)));
    }

    #[test]
    fn query_length_allows_limit() {
        let query = "a".repeat(MAX_QUERY_LEN);
        enforce_query_length(&query).unwrap();
    }
}
