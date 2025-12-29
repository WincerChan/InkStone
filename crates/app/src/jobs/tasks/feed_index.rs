use chrono::{DateTime, Utc};
use serde::Deserialize;
use sha2::{Digest, Sha256};
use thiserror::Error;
use tracing::warn;

use crate::jobs::JobError;
use crate::state::AppState;
use inkstone_core::domain::search::SearchDocument;

#[derive(Debug)]
pub struct JobStats {
    pub fetched: usize,
    pub indexed: usize,
    pub skipped: usize,
    pub failed: usize,
}

#[derive(Debug, Error)]
enum EntryError {
    #[error("missing entry title")]
    MissingTitle,
    #[error("missing entry link")]
    MissingLink,
    #[error("missing entry timestamps")]
    MissingTimestamps,
    #[error("invalid entry timestamp: {0}")]
    InvalidTimestamp(String),
}

#[derive(Debug, Deserialize)]
pub(crate) struct SearchIndexEntry {
    pub title: String,
    pub subtitle: Option<String>,
    pub url: String,
    pub date: String,
    pub updated: Option<String>,
    pub category: Option<String>,
    pub tags: Vec<String>,
    pub content: String,
}

pub async fn run(state: &AppState, rebuild: bool) -> Result<JobStats, JobError> {
    let response = state
        .http_client
        .get(&state.config.feed_url)
        .send()
        .await?
        .error_for_status()?;
    let body = response.bytes().await?;
    let entries =
        parse_search_index_entries(&body).map_err(|err| JobError::Feed(err.to_string()))?;

    if rebuild {
        state.search.delete_all()?;
    }

    let mut stats = JobStats {
        fetched: 0,
        indexed: 0,
        skipped: 0,
        failed: 0,
    };
    let base_url = base_url_from_feed(&state.config.feed_url);
    let mut to_index = Vec::new();

    for entry in entries {
        stats.fetched += 1;
        match entry_to_document_from_json(&entry, base_url.as_deref()) {
            Ok(doc) => {
                if !rebuild {
                    if let Some(existing) = state.search.get_checksum(&doc.id)? {
                        if existing == doc.checksum {
                            stats.skipped += 1;
                            continue;
                        }
                    }
                }
                to_index.push(doc);
            }
            Err(err) => {
                stats.failed += 1;
                warn!(error = %err, "failed to parse search index entry");
            }
        }
    }

    if !to_index.is_empty() {
        state.search.upsert_documents(&to_index)?;
        stats.indexed = to_index.len();
    }

    Ok(stats)
}

pub(crate) fn parse_search_index_entries(
    json: &[u8],
) -> Result<Vec<SearchIndexEntry>, serde_json::Error> {
    serde_json::from_slice(json)
}

pub(crate) fn base_url_from_feed(feed_url: &str) -> Option<String> {
    let (scheme, rest) = feed_url.split_once("://")?;
    let host = rest.split('/').next()?;
    if host.is_empty() {
        return None;
    }
    Some(format!("{scheme}://{host}"))
}

fn entry_to_document_from_json(
    entry: &SearchIndexEntry,
    base_url: Option<&str>,
) -> Result<SearchDocument, EntryError> {
    let title = entry.title.trim();
    if title.is_empty() {
        return Err(EntryError::MissingTitle);
    }
    let url_raw = entry.url.trim();
    if url_raw.is_empty() {
        return Err(EntryError::MissingLink);
    }
    let url = resolve_entry_url(url_raw, base_url);

    let published_at = parse_datetime(entry.date.trim())?;
    let updated_at = entry
        .updated
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(parse_datetime)
        .transpose()?
        .unwrap_or(published_at);

    let content_raw = sanitize_markdown(&entry.content);
    let content = normalize_whitespace(&content_raw);

    let category = entry
        .category
        .as_deref()
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string());
    let tags = entry
        .tags
        .iter()
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect::<Vec<_>>();

    let doc_id = url.clone();
    let checksum = compute_checksum(
        &doc_id,
        title,
        &content,
        &url,
        &tags,
        category.as_deref().unwrap_or(""),
        published_at,
        updated_at,
    );

    Ok(SearchDocument {
        id: doc_id,
        title: title.to_string(),
        content,
        url,
        tags,
        category,
        published_at,
        updated_at,
        checksum,
    })
}

fn resolve_entry_url(url: &str, base_url: Option<&str>) -> String {
    if url.starts_with("http://") || url.starts_with("https://") {
        return url.to_string();
    }
    let Some(base) = base_url else {
        return url.to_string();
    };
    if url.starts_with('/') {
        return format!("{base}{url}");
    }
    format!("{base}/{url}")
}

fn sanitize_markdown(value: &str) -> String {
    value
        .replace("<!--more-->", "")
        .replace("<!-- more -->", "")
}

fn parse_datetime(value: &str) -> Result<DateTime<Utc>, EntryError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(EntryError::MissingTimestamps);
    }
    let parsed = DateTime::parse_from_rfc3339(trimmed)
        .map_err(|_| EntryError::InvalidTimestamp(trimmed.to_string()))?;
    Ok(parsed.with_timezone(&Utc))
}

fn compute_checksum(
    id: &str,
    title: &str,
    content: &str,
    url: &str,
    tags: &[String],
    category: &str,
    published_at: DateTime<Utc>,
    updated_at: DateTime<Utc>,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(id.as_bytes());
    hasher.update([0]);
    hasher.update(title.as_bytes());
    hasher.update([0]);
    hasher.update(content.as_bytes());
    hasher.update([0]);
    hasher.update(url.as_bytes());
    hasher.update([0]);
    for tag in tags {
        hasher.update(tag.as_bytes());
        hasher.update([0]);
    }
    hasher.update(category.as_bytes());
    hasher.update([0]);
    hasher.update(published_at.timestamp().to_string().as_bytes());
    hasher.update([0]);
    hasher.update(updated_at.timestamp().to_string().as_bytes());
    hex::encode(hasher.finalize())
}

fn normalize_whitespace(input: &str) -> String {
    let mut parts = input.split_whitespace();
    let Some(first) = parts.next() else {
        return String::new();
    };
    let mut output = String::with_capacity(input.len());
    output.push_str(first);
    for part in parts {
        output.push(' ');
        output.push_str(part);
    }
    output
}

#[cfg(test)]
mod tests {
    use super::{
        entry_to_document_from_json, parse_search_index_entries, SearchIndexEntry,
    };

    fn base_entry() -> SearchIndexEntry {
        SearchIndexEntry {
            title: "Title".to_string(),
            subtitle: None,
            url: "/posts/hello/".to_string(),
            date: "2025-01-01T00:00:00Z".to_string(),
            updated: None,
            category: Some("分享境".to_string()),
            tags: vec!["Rust".to_string()],
            content: "Hi".to_string(),
        }
    }

    #[test]
    fn content_keeps_html_tags_in_json() {
        let mut entry = base_entry();
        entry.content = "<Suspense><div>Hi</div></Suspense>".to_string();

        let doc = entry_to_document_from_json(&entry, Some("https://example.com")).unwrap();
        assert!(doc.content.contains("<Suspense>"));
    }

    #[test]
    fn json_entry_builds_document() {
        let json = r#"
[
  {
    "title": "Hello",
    "subtitle": "Sub",
    "url": "/posts/hello/",
    "date": "2025-01-01T00:00:00Z",
    "updated": "2025-01-02T00:00:00Z",
    "category": "分享境",
    "tags": ["Rust"],
    "content": "Hi<!--more-->there"
  }
]
"#;
        let entries = parse_search_index_entries(json.as_bytes()).unwrap();
        let doc =
            entry_to_document_from_json(&entries[0], Some("https://example.com")).unwrap();
        assert_eq!(doc.url, "https://example.com/posts/hello/");
        assert!(doc.content.contains("Hi"));
        assert!(!doc.content.contains("<!--more-->"));
    }
}
