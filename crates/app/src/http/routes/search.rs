use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::debug;

use crate::state::AppState;
use inkstone_core::domain::search::{SearchHit, SearchResult};
use inkstone_infra::search::{parse_query, QueryParseError, SearchIndexError};

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
}

#[derive(Debug, Error)]
pub enum SearchApiError {
    #[error("invalid query: {0}")]
    Query(#[from] QueryParseError),
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
    let query_text = params.q.unwrap_or_default();
    let limit = params
        .limit
        .unwrap_or(20)
        .min(state.config.max_search_limit);
    let offset = params.offset.unwrap_or(0);

    let query = parse_query(&query_text)?;
    debug!(query_text = %query_text, ?query, "parsed search query");
    let result: SearchResult = state.search.search(&query, limit, offset)?;

    Ok(Json(SearchResponse {
        total: result.total,
        hits: result.hits,
    }))
}

impl IntoResponse for SearchApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            SearchApiError::Query(err) => (StatusCode::BAD_REQUEST, err.to_string()),
            SearchApiError::Search(err) => (StatusCode::INTERNAL_SERVER_ERROR, err.to_string()),
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}
