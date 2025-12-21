use std::io::Cursor;

use chrono::{DateTime, Utc};
use feed_rs::model::Entry;
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
    #[error("missing entry id")]
    MissingId,
    #[error("missing entry title")]
    MissingTitle,
    #[error("missing entry link")]
    MissingLink,
    #[error("missing entry timestamps")]
    MissingTimestamps,
}

pub async fn run(state: &AppState, rebuild: bool) -> Result<JobStats, JobError> {
    let response = state
        .http_client
        .get(&state.config.feed_url)
        .send()
        .await?
        .error_for_status()?;
    let body = response.bytes().await?;
    let feed = feed_rs::parser::parse(Cursor::new(body))?;

    if rebuild {
        state.search.delete_all()?;
    }

    let mut stats = JobStats {
        fetched: 0,
        indexed: 0,
        skipped: 0,
        failed: 0,
    };

    let mut to_index = Vec::new();
    for entry in feed.entries {
        stats.fetched += 1;
        match entry_to_document(&entry) {
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
                warn!(error = %err, "failed to parse feed entry");
            }
        }
    }

    if !to_index.is_empty() {
        state.search.upsert_documents(&to_index)?;
        stats.indexed = to_index.len();
    }

    Ok(stats)
}

fn entry_to_document(entry: &Entry) -> Result<SearchDocument, EntryError> {
    if entry.id.trim().is_empty() {
        return Err(EntryError::MissingId);
    }
    let title = entry
        .title
        .as_ref()
        .map(|text| text.content.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or(EntryError::MissingTitle)?;

    let url = find_link(entry).ok_or(EntryError::MissingLink)?;

    let published_at_fixed = entry
        .published
        .or(entry.updated)
        .ok_or(EntryError::MissingTimestamps)?;
    let updated_at_fixed = entry.updated.unwrap_or(published_at_fixed);
    let published_at = published_at_fixed.with_timezone(&Utc);
    let updated_at = updated_at_fixed.with_timezone(&Utc);

    let summary = entry
        .summary
        .as_ref()
        .map(|text| text.content.trim().to_string())
        .filter(|text| !text.is_empty());
    let content_raw = entry
        .content
        .as_ref()
        .and_then(|content| content.body.as_deref())
        .or_else(|| entry.summary.as_ref().map(|text| text.content.as_str()))
        .unwrap_or_default();
    let content = strip_html_tags(content_raw).trim().to_string();

    let tags: Vec<String> = entry
        .categories
        .iter()
        .filter_map(|category| {
            let term = category.term.trim();
            if term.is_empty() {
                None
            } else {
                Some(term.to_string())
            }
        })
        .collect();
    let category = entry.categories.first().and_then(|category| {
        let label = category.label.as_deref().unwrap_or("").trim();
        if !label.is_empty() {
            return Some(label.to_string());
        }
        let term = category.term.trim();
        if term.is_empty() {
            None
        } else {
            Some(term.to_string())
        }
    });

    let checksum = compute_checksum(
        &entry.id,
        &title,
        summary.as_deref().unwrap_or(""),
        &content,
        &url,
        &tags,
        category.as_deref().unwrap_or(""),
        published_at,
        updated_at,
    );

    Ok(SearchDocument {
        id: entry.id.clone(),
        title,
        summary,
        content,
        url,
        tags,
        category,
        published_at,
        updated_at,
        checksum,
    })
}

fn find_link(entry: &Entry) -> Option<String> {
    entry
        .links
        .iter()
        .find(|link| link.rel.as_deref() == Some("alternate"))
        .or_else(|| entry.links.first())
        .and_then(|link| {
            let href = link.href.trim();
            if href.is_empty() {
                None
            } else {
                Some(href.to_string())
            }
        })
}

fn compute_checksum(
    id: &str,
    title: &str,
    summary: &str,
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
    hasher.update(summary.as_bytes());
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

fn strip_html_tags(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut in_tag = false;
    for ch in input.chars() {
        match ch {
            '<' => in_tag = true,
            '>' => {
                in_tag = false;
                output.push(' ');
            }
            _ if !in_tag => output.push(ch),
            _ => {}
        }
    }
    output
}
