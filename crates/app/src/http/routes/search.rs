use std::time::Instant;

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use tracing::{info, warn};

use crate::state::AppState;
use inkstone_core::domain::search::{SearchHit, SearchQuery, SearchResult};
use inkstone_infra::search::{parse_query, QueryParseError, SearchIndexError, SearchSort};

const MAX_QUERY_LEN: usize = 256;

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub sort: Option<SearchSortParam>,
}

#[derive(Debug, Deserialize, Clone, Copy)]
#[serde(rename_all = "lowercase")]
pub enum SearchSortParam {
    Relevance,
    Latest,
}

impl Default for SearchSortParam {
    fn default() -> Self {
        Self::Relevance
    }
}

impl SearchSortParam {
    fn as_sort(self) -> SearchSort {
        match self {
            SearchSortParam::Relevance => SearchSort::Relevance,
            SearchSortParam::Latest => SearchSort::Latest,
        }
    }

    fn as_str(self) -> &'static str {
        match self {
            SearchSortParam::Relevance => "relevance",
            SearchSortParam::Latest => "latest",
        }
    }
}

#[derive(Debug, Serialize)]
pub struct SearchResponse {
    pub total: usize,
    pub hits: Vec<SearchHitResponse>,
    pub elapsed_ms: u128,
}

#[derive(Debug, Serialize)]
pub struct SearchHitResponse {
    #[serde(flatten)]
    pub hit: SearchHit,
    pub matched: MatchedFields,
}

#[derive(Debug, Serialize, PartialEq, Eq)]
pub struct MatchedFields {
    pub title: bool,
    pub content: bool,
    pub tags: bool,
    pub category: bool,
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
    let sort = params.sort.unwrap_or_default();

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
    let result: SearchResult = match state.search.search(&query, limit, offset, sort.as_sort()) {
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
        sort = sort.as_str(),
        total = result.total,
        elapsed_ms = started_at.elapsed().as_millis(),
        "search request completed"
    );
    let hits = result
        .hits
        .into_iter()
        .map(|hit| SearchHitResponse {
            matched: build_matched(&hit, &query),
            hit,
        })
        .collect();
    Ok(Json(SearchResponse {
        total: result.total,
        hits,
        elapsed_ms: started_at.elapsed().as_millis(),
    }))
}

fn enforce_query_length(query_text: &str) -> Result<(), SearchApiError> {
    if query_text.chars().count() > MAX_QUERY_LEN {
        return Err(SearchApiError::QueryTooLong(MAX_QUERY_LEN));
    }
    Ok(())
}

fn build_matched(hit: &SearchHit, query: &SearchQuery) -> MatchedFields {
    let title = hit.title.contains("<b>");
    let content = hit
        .content
        .as_deref()
        .map(|value| value.contains("<b>"))
        .unwrap_or(false);

    let mut tags = false;
    if !query.tags.is_empty() {
        tags = query
            .tags
            .iter()
            .any(|tag| hit.tags.iter().any(|hit_tag| hit_tag == tag));
    }
    if !tags && !query.keywords.is_empty() {
        tags = query
            .keywords
            .iter()
            .any(|keyword| hit.tags.iter().any(|tag| tag == keyword));
    }

    let mut category = false;
    if let Some(query_category) = query.category.as_ref() {
        category = hit.category.as_deref() == Some(query_category.as_str());
    }
    if !category {
        if let Some(hit_category) = hit.category.as_deref() {
            category = query.keywords.iter().any(|keyword| keyword == hit_category);
        }
    }

    MatchedFields {
        title,
        content,
        tags,
        category,
    }
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
    use super::{
        build_matched, enforce_query_length, MatchedFields, SearchApiError, SearchSortParam,
        MAX_QUERY_LEN,
    };
    use chrono::{TimeZone, Utc};
    use inkstone_core::domain::search::{SearchHit, SearchQuery};
    use serde_json::json;

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

    #[test]
    fn sort_param_parses_latest() {
        let sort: SearchSortParam = serde_json::from_value(json!("latest")).unwrap();
        assert!(matches!(sort, SearchSortParam::Latest));
    }

    #[test]
    fn matched_fields_detects_highlight_and_keyword_tags() {
        let hit = SearchHit {
            id: "id".to_string(),
            title: "<b>实验室</b> 笔记".to_string(),
            content: Some("正文内容".to_string()),
            url: "https://example.com".to_string(),
            tags: vec!["实验室".to_string()],
            category: None,
            published_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        };
        let query = SearchQuery {
            keywords: vec!["实验室".to_string()],
            ..Default::default()
        };
        let matched = build_matched(&hit, &query);
        assert_eq!(
            matched,
            MatchedFields {
                title: true,
                content: false,
                tags: true,
                category: false
            }
        );
    }

    #[test]
    fn matched_fields_detects_explicit_category_filter() {
        let hit = SearchHit {
            id: "id".to_string(),
            title: "无关".to_string(),
            content: None,
            url: "https://example.com".to_string(),
            tags: vec![],
            category: Some("实验室".to_string()),
            published_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
            updated_at: Utc.timestamp_opt(1_700_000_000, 0).unwrap(),
        };
        let query = SearchQuery {
            category: Some("实验室".to_string()),
            ..Default::default()
        };
        let matched = build_matched(&hit, &query);
        assert_eq!(
            matched,
            MatchedFields {
                title: false,
                content: false,
                tags: false,
                category: true
            }
        );
    }
}
