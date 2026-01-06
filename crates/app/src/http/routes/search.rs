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
use inkstone_core::types::time_range::TimeRange;
use inkstone_infra::db::{insert_search_event, SearchEvent};
use inkstone_infra::search::{parse_query, QueryParseError, SearchIndexError, SearchSort};

const MAX_QUERY_LEN: usize = 256;

#[derive(Debug, Deserialize)]
pub struct SearchParams {
    pub q: Option<String>,
    pub limit: Option<usize>,
    pub offset: Option<usize>,
    pub sort: Option<SearchSortParam>,
}

#[derive(Debug, Deserialize, Clone, Copy, PartialEq, Eq)]
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SearchEventKind {
    Search,
    Sort,
    Page,
}

impl SearchEventKind {
    fn as_str(self) -> &'static str {
        match self {
            SearchEventKind::Search => "search",
            SearchEventKind::Sort => "sort",
            SearchEventKind::Page => "page",
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
    pub subtitle: bool,
    pub content: bool,
    pub tags: Vec<String>,
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
        .unwrap_or(8)
        .min(state.config.max_search_limit);
    let offset = params.offset.unwrap_or(0);
    let sort = params.sort.unwrap_or_default();
    let kind = determine_event_kind(offset, sort);

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

    let elapsed_ms = started_at.elapsed().as_millis();
    if let Some(pool) = state.db.as_ref() {
        let event = build_search_event(&query_text, &query, sort, kind, result.total, elapsed_ms);
        if let Err(err) = insert_search_event(pool, &event).await {
            warn!(error = %err, "failed to store search event");
        }
    }
    info!(
        query_text = %query_text,
        limit,
        offset,
        sort = sort.as_str(),
        total = result.total,
        elapsed_ms = elapsed_ms,
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
        elapsed_ms: elapsed_ms,
    }))
}

fn build_search_event(
    query_text: &str,
    query: &SearchQuery,
    sort: SearchSortParam,
    kind: SearchEventKind,
    total: usize,
    elapsed_ms: u128,
) -> SearchEvent {
    let raw = query_text.trim().to_string();
    let keyword_count = query.keywords.len() as i32;
    let mut keywords = query
        .keywords
        .iter()
        .map(|value| normalize_token(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    keywords.sort();
    keywords.dedup();

    let mut tags = query
        .tags
        .iter()
        .map(|value| normalize_token(value))
        .filter(|value| !value.is_empty())
        .collect::<Vec<_>>();
    tags.sort();
    tags.dedup();

    let category = query
        .category
        .as_ref()
        .map(|value| normalize_token(value))
        .filter(|value| !value.is_empty());

    let mut parts = Vec::new();
    if !keywords.is_empty() {
        parts.push(keywords.join(" "));
    }
    if let Some(category_value) = category.as_ref() {
        parts.push(format!("category:{category_value}"));
    }
    if !tags.is_empty() {
        parts.push(format!("tags:{}", tags.join(",")));
    }
    if let Some(range) = query.range.as_ref() {
        parts.push(format!("range:{}", format_range(range)));
    }

    let normalized = if parts.is_empty() {
        raw.clone()
    } else {
        parts.join(" ")
    };

    SearchEvent {
        query_raw: raw,
        query_norm: normalized,
        keyword_count,
        tags,
        category,
        range_start: query.range.as_ref().and_then(|range| range.start),
        range_end: query.range.as_ref().and_then(|range| range.end),
        sort: sort.as_str().to_string(),
        kind: kind.as_str().to_string(),
        result_total: clamp_i32(total as i64),
        elapsed_ms: clamp_i32(elapsed_ms as i64),
    }
}

fn determine_event_kind(offset: usize, sort: SearchSortParam) -> SearchEventKind {
    if offset > 0 {
        return SearchEventKind::Page;
    }
    if sort != SearchSortParam::default() {
        return SearchEventKind::Sort;
    }
    SearchEventKind::Search
}

fn normalize_token(value: &str) -> String {
    value.trim().to_ascii_lowercase()
}

fn format_range(range: &TimeRange) -> String {
    let start = range
        .start
        .map(|value| value.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    let end = range
        .end
        .map(|value| value.format("%Y-%m-%d").to_string())
        .unwrap_or_default();
    format!("{start}~{end}")
}

fn clamp_i32(value: i64) -> i32 {
    if value > i32::MAX as i64 {
        i32::MAX
    } else if value < i32::MIN as i64 {
        i32::MIN
    } else {
        value as i32
    }
}

fn enforce_query_length(query_text: &str) -> Result<(), SearchApiError> {
    if query_text.chars().count() > MAX_QUERY_LEN {
        return Err(SearchApiError::QueryTooLong(MAX_QUERY_LEN));
    }
    Ok(())
}

fn build_matched(hit: &SearchHit, query: &SearchQuery) -> MatchedFields {
    let title = hit.title.contains("<b>");
    let subtitle = hit
        .subtitle
        .as_deref()
        .map(|value| value.contains("<b>"))
        .unwrap_or(false);
    let content = hit
        .content
        .as_deref()
        .map(|value| value.contains("<b>"))
        .unwrap_or(false);

    let mut tag_matches = Vec::new();
    if !hit.tags.is_empty() {
        let mut candidates: Vec<&str> = Vec::new();
        candidates.extend(query.tags.iter().map(|tag| tag.as_str()));
        candidates.extend(query.keywords.iter().map(|keyword| keyword.as_str()));
        if !candidates.is_empty() {
            for hit_tag in &hit.tags {
                if candidates.iter().any(|candidate| hit_tag == candidate) {
                    if !tag_matches.contains(hit_tag) {
                        tag_matches.push(hit_tag.clone());
                    }
                }
            }
        }
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
        subtitle,
        content,
        tags: tag_matches,
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
        build_matched, determine_event_kind, enforce_query_length, MatchedFields, SearchApiError,
        SearchEventKind, SearchSortParam, MAX_QUERY_LEN,
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
            subtitle: Some("副标题".to_string()),
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
                subtitle: false,
                content: false,
                tags: vec!["实验室".to_string()],
                category: false
            }
        );
    }

    #[test]
    fn matched_fields_detects_explicit_category_filter() {
        let hit = SearchHit {
            id: "id".to_string(),
            title: "无关".to_string(),
            subtitle: None,
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
                subtitle: false,
                content: false,
                tags: Vec::new(),
                category: true
            }
        );
    }

    #[test]
    fn determine_event_kind_classifies_offset_and_sort() {
        assert_eq!(
            determine_event_kind(0, SearchSortParam::Relevance),
            SearchEventKind::Search
        );
        assert_eq!(
            determine_event_kind(0, SearchSortParam::Latest),
            SearchEventKind::Sort
        );
        assert_eq!(
            determine_event_kind(20, SearchSortParam::Latest),
            SearchEventKind::Page
        );
        assert_eq!(
            determine_event_kind(10, SearchSortParam::Relevance),
            SearchEventKind::Page
        );
    }
}
