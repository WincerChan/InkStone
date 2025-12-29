use chrono::{DateTime, Utc};
use serde::Serialize;

use crate::types::time_range::TimeRange;

#[derive(Debug, Clone, Default)]
pub struct SearchQuery {
    pub keywords: Vec<String>,
    pub range: Option<TimeRange>,
    pub tags: Vec<String>,
    pub category: Option<String>,
}

#[derive(Debug, Clone)]
pub struct SearchDocument {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub content: String,
    pub url: String,
    pub tags: Vec<String>,
    pub category: Option<String>,
    pub published_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
    pub checksum: String,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchHit {
    pub id: String,
    pub title: String,
    pub subtitle: Option<String>,
    pub content: Option<String>,
    pub url: String,
    pub tags: Vec<String>,
    pub category: Option<String>,
    pub published_at: DateTime<Utc>,
    pub updated_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct SearchResult {
    pub total: usize,
    pub hits: Vec<SearchHit>,
}
