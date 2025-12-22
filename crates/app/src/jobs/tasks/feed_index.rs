use std::io::Cursor;

use chrono::{DateTime, Utc};
use quick_xml::events::{BytesRef, BytesStart, Event};
use quick_xml::Reader;
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

#[derive(Debug, Default)]
struct AtomEntry {
    title: Option<String>,
    link: Option<String>,
    published: Option<String>,
    updated: Option<String>,
    content: Option<String>,
    categories: Vec<AtomCategory>,
}

#[derive(Debug)]
struct AtomCategory {
    term: Option<String>,
    label: Option<String>,
}

#[derive(Debug, Clone, Copy)]
enum TextTarget {
    Title,
    Published,
    Updated,
    Content,
}

pub async fn run(state: &AppState, rebuild: bool) -> Result<JobStats, JobError> {
    let response = state
        .http_client
        .get(&state.config.feed_url)
        .send()
        .await?
        .error_for_status()?;
    let body = response.bytes().await?;
    let entries = parse_feed_entries(&body).map_err(|err| JobError::Feed(err.to_string()))?;

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
    for entry in entries {
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

fn parse_feed_entries(xml: &[u8]) -> Result<Vec<AtomEntry>, quick_xml::Error> {
    let mut reader = Reader::from_reader(Cursor::new(xml));
    reader.config_mut().trim_text(true);
    let mut buf = Vec::new();
    let mut entries = Vec::new();
    let mut current: Option<AtomEntry> = None;
    let mut text_target: Option<TextTarget> = None;
    let mut text_buffer = String::new();

    loop {
        match reader.read_event_into(&mut buf)? {
            Event::Start(event) => {
                let name = event.name();
                if name.as_ref() == b"entry" {
                    current = Some(AtomEntry::default());
                } else if let Some(entry) = current.as_mut() {
                    handle_start_event(
                        name.as_ref(),
                        &event,
                        entry,
                        &mut text_target,
                        &mut text_buffer,
                    )?;
                }
            }
            Event::Empty(event) => {
                if let Some(entry) = current.as_mut() {
                    handle_empty_event(event.name().as_ref(), &event, entry)?;
                }
            }
            Event::Text(text) => {
                if let Some(target) = text_target {
                    let chunk = match target {
                        TextTarget::Content => String::from_utf8_lossy(text.as_ref()).into_owned(),
                        _ => text.decode()?.into_owned(),
                    };
                    text_buffer.push_str(&chunk);
                }
            }
            Event::GeneralRef(reference) => {
                if text_target.is_some() {
                    append_general_ref(&reference, &mut text_buffer)?;
                }
            }
            Event::CData(text) => {
                if let Some(target) = text_target {
                    let chunk = match target {
                        TextTarget::Content => String::from_utf8_lossy(text.as_ref()).into_owned(),
                        _ => text.decode()?.into_owned(),
                    };
                    text_buffer.push_str(&chunk);
                }
            }
            Event::End(event) => {
                let name = event.name();
                if name.as_ref() == b"entry" {
                    if let Some(entry) = current.take() {
                        entries.push(entry);
                    }
                } else if let Some(target) = text_target {
                    if target.matches(name.as_ref()) {
                        if let Some(entry) = current.as_mut() {
                            assign_text(entry, target, &text_buffer);
                        }
                        text_buffer.clear();
                        text_target = None;
                    }
                }
            }
            Event::Eof => break,
            _ => {}
        }
        buf.clear();
    }

    Ok(entries)
}

fn handle_start_event(
    name: &[u8],
    event: &BytesStart<'_>,
    entry: &mut AtomEntry,
    text_target: &mut Option<TextTarget>,
    text_buffer: &mut String,
) -> Result<(), quick_xml::Error> {
    if name == b"link" {
        parse_link_attrs(event, entry)?;
    } else if name == b"category" {
        parse_category_attrs(event, entry)?;
    }

    if text_target.is_none() {
        if let Some(target) = TextTarget::from_name(name) {
            *text_target = Some(target);
            text_buffer.clear();
        }
    }

    Ok(())
}

fn handle_empty_event(
    name: &[u8],
    event: &BytesStart<'_>,
    entry: &mut AtomEntry,
) -> Result<(), quick_xml::Error> {
    if name == b"link" {
        parse_link_attrs(event, entry)?;
    } else if name == b"category" {
        parse_category_attrs(event, entry)?;
    }
    Ok(())
}

fn parse_link_attrs(event: &BytesStart<'_>, entry: &mut AtomEntry) -> Result<(), quick_xml::Error> {
    let mut href = None;
    let mut rel = None;
    for attr in event.attributes() {
        let attr = attr?;
        match attr.key.as_ref() {
            b"href" => href = Some(attr.unescape_value()?.to_string()),
            b"rel" => rel = Some(attr.unescape_value()?.to_string()),
            _ => {}
        }
    }

    if let Some(href) = href {
        let rel_alt = rel.as_deref() == Some("alternate");
        if entry.link.is_none() || rel_alt {
            entry.link = Some(href);
        }
    }
    Ok(())
}

fn parse_category_attrs(
    event: &BytesStart<'_>,
    entry: &mut AtomEntry,
) -> Result<(), quick_xml::Error> {
    let mut term = None;
    let mut label = None;
    for attr in event.attributes() {
        let attr = attr?;
        match attr.key.as_ref() {
            b"term" => term = Some(attr.unescape_value()?.to_string()),
            b"label" => label = Some(attr.unescape_value()?.to_string()),
            _ => {}
        }
    }

    let term = term.map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    let label = label.map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    if term.is_some() || label.is_some() {
        entry.categories.push(AtomCategory { term, label });
    }
    Ok(())
}

fn assign_text(entry: &mut AtomEntry, target: TextTarget, value: &str) {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return;
    }
    match target {
        TextTarget::Title => entry.title = Some(trimmed.to_string()),
        TextTarget::Published => entry.published = Some(trimmed.to_string()),
        TextTarget::Updated => entry.updated = Some(trimmed.to_string()),
        TextTarget::Content => entry.content = Some(trimmed.to_string()),
    }
}

fn append_general_ref(
    reference: &BytesRef<'_>,
    text_buffer: &mut String,
) -> Result<(), quick_xml::Error> {
    if let Some(ch) = reference.resolve_char_ref()? {
        text_buffer.push(ch);
        return Ok(());
    }

    let name = reference.decode()?;
    match name.as_ref() {
        "lt" => text_buffer.push('<'),
        "gt" => text_buffer.push('>'),
        "amp" => text_buffer.push('&'),
        "quot" => text_buffer.push('"'),
        "apos" => text_buffer.push('\''),
        "nbsp" => text_buffer.push(' '),
        _ => {
            text_buffer.push('&');
            text_buffer.push_str(name.as_ref());
            text_buffer.push(';');
        }
    }
    Ok(())
}

impl TextTarget {
    fn from_name(name: &[u8]) -> Option<Self> {
        match name {
            b"title" => Some(TextTarget::Title),
            b"published" => Some(TextTarget::Published),
            b"updated" => Some(TextTarget::Updated),
            b"content" => Some(TextTarget::Content),
            _ => None,
        }
    }

    fn matches(self, name: &[u8]) -> bool {
        match self {
            TextTarget::Title => name == b"title",
            TextTarget::Published => name == b"published",
            TextTarget::Updated => name == b"updated",
            TextTarget::Content => name == b"content",
        }
    }
}

fn entry_to_document(entry: &AtomEntry) -> Result<SearchDocument, EntryError> {
    let title = entry
        .title
        .as_ref()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or(EntryError::MissingTitle)?;

    let url = entry
        .link
        .as_ref()
        .map(|text| text.trim().to_string())
        .filter(|text| !text.is_empty())
        .ok_or(EntryError::MissingLink)?;

    let published_raw = entry
        .published
        .as_deref()
        .or(entry.updated.as_deref())
        .ok_or(EntryError::MissingTimestamps)?;
    let published_at = parse_datetime(published_raw)?;
    let updated_at = entry
        .updated
        .as_deref()
        .map(parse_datetime)
        .transpose()?
        .unwrap_or(published_at);

    let content_raw = entry.content.as_deref().unwrap_or("");
    let content = normalize_whitespace(&strip_html_tags(content_raw));

    let (category, category_term) = entry.categories.first().map_or((None, None), |category| {
        let label = category.label.as_deref().unwrap_or("").trim();
        if !label.is_empty() {
            return (Some(label.to_string()), category.term.as_deref());
        }
        let term = category.term.as_deref().unwrap_or("").trim();
        if term.is_empty() {
            (None, None)
        } else {
            (Some(term.to_string()), Some(term))
        }
    });
    let tags: Vec<String> = entry
        .categories
        .iter()
        .filter_map(|category| category.term.as_ref())
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
        .filter(|value| {
            if let Some(term) = category_term {
                if value == term {
                    return false;
                }
            }
            if let Some(category) = category.as_deref() {
                if value == category {
                    return false;
                }
            }
            true
        })
        .collect();

    let doc_id = url.clone();
    let checksum = compute_checksum(
        &doc_id,
        &title,
        &content,
        &url,
        &tags,
        category.as_deref().unwrap_or(""),
        published_at,
        updated_at,
    );

    Ok(SearchDocument {
        id: doc_id,
        title,
        content,
        url,
        tags,
        category,
        published_at,
        updated_at,
        checksum,
    })
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

fn strip_html_tags(input: &str) -> String {
    let mut text = input.to_string();
    for _ in 0..2 {
        let decoded = decode_html_entities(&text);
        text = remove_html_tags(&decoded);
    }
    decode_html_entities(&text)
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

fn remove_html_tags(input: &str) -> String {
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

fn decode_html_entities(input: &str) -> String {
    let mut output = String::with_capacity(input.len());
    let mut chars = input.chars().peekable();
    while let Some(ch) = chars.next() {
        if ch != '&' {
            output.push(ch);
            continue;
        }

        let mut entity = String::new();
        let mut valid = false;
        while let Some(&next) = chars.peek() {
            if next == ';' {
                chars.next();
                valid = true;
                break;
            }
            if next.is_whitespace() || entity.len() > 32 {
                break;
            }
            chars.next();
            entity.push(next);
        }

        if !valid || entity.is_empty() {
            output.push('&');
            if !entity.is_empty() {
                output.push_str(&entity);
            }
            continue;
        }

        match entity.as_str() {
            "amp" => output.push('&'),
            "lt" => output.push('<'),
            "gt" => output.push('>'),
            "quot" => output.push('"'),
            "apos" => output.push('\''),
            "nbsp" => output.push(' '),
            _ => {
                if let Some(decoded) = decode_numeric_entity(&entity) {
                    output.push(decoded);
                } else {
                    output.push('&');
                    output.push_str(&entity);
                    output.push(';');
                }
            }
        }
    }
    output
}

fn decode_numeric_entity(entity: &str) -> Option<char> {
    let trimmed = entity.strip_prefix('#')?;
    if let Some(hex) = trimmed.strip_prefix(['x', 'X']) {
        u32::from_str_radix(hex, 16).ok().and_then(char::from_u32)
    } else {
        trimmed.parse::<u32>().ok().and_then(char::from_u32)
    }
}

#[cfg(test)]
mod tests {
    use super::{entry_to_document, parse_feed_entries, AtomCategory, AtomEntry};

    fn base_entry() -> AtomEntry {
        AtomEntry {
            title: Some("Title".to_string()),
            link: Some("https://example.com/post".to_string()),
            published: Some("2025-01-01T00:00:00Z".to_string()),
            updated: None,
            content: None,
            categories: Vec::new(),
        }
    }

    #[test]
    fn category_term_is_not_in_tags() {
        let mut entry = base_entry();
        entry.categories = vec![
            AtomCategory {
                term: Some("category".to_string()),
                label: None,
            },
            AtomCategory {
                term: Some("tag1".to_string()),
                label: None,
            },
        ];

        let doc = entry_to_document(&entry).unwrap();
        assert_eq!(doc.category, Some("category".to_string()));
        assert_eq!(doc.tags, vec!["tag1".to_string()]);
    }

    #[test]
    fn category_label_excludes_first_term() {
        let mut entry = base_entry();
        entry.categories = vec![
            AtomCategory {
                term: Some("category-term".to_string()),
                label: Some("Category".to_string()),
            },
            AtomCategory {
                term: Some("tag1".to_string()),
                label: None,
            },
        ];

        let doc = entry_to_document(&entry).unwrap();
        assert_eq!(doc.category, Some("Category".to_string()));
        assert_eq!(doc.tags, vec!["tag1".to_string()]);
    }

    #[test]
    fn content_empty_when_missing() {
        let mut entry = base_entry();
        entry.content = Some("<p></p>".to_string());

        let doc = entry_to_document(&entry).unwrap();
        assert!(doc.content.is_empty());
    }

    #[test]
    fn id_uses_url() {
        let entry = base_entry();
        let doc = entry_to_document(&entry).unwrap();
        assert_eq!(doc.id, "https://example.com/post");
    }

    #[test]
    fn content_strips_html_and_decodes_entities() {
        let mut entry = base_entry();
        entry.content = Some("&lt;p&gt;Tom &amp; Jerry &quot;show&quot;&lt;/p&gt;".to_string());

        let doc = entry_to_document(&entry).unwrap();
        assert_eq!(doc.content, "Tom & Jerry \"show\"");
    }

    #[test]
    fn content_strips_double_encoded_html() {
        let mut entry = base_entry();
        entry.content = Some(
            "&amp;lt;del&amp;gt;我爱你&amp;lt;/del&amp;gt;".to_string(),
        );

        let doc = entry_to_document(&entry).unwrap();
        assert_eq!(doc.content, "我爱你");
    }

    #[test]
    fn content_from_atom_is_decoded_and_stripped() {
        let xml = r#"
<feed xmlns="http://www.w3.org/2005/Atom">
  <entry>
    <title>Example</title>
    <link rel="alternate" type="text/html" href="https://example.com/post" />
    <id>urn:uuid:1234</id>
    <published>2025-01-01T00:00:00Z</published>
    <updated>2025-01-02T00:00:00Z</updated>
    <content type="html">&lt;p&gt;&lt;img src="https://img13.360buyimg.com/ddimg/jfs/t1/256238/30/29757/30939/67c95492F0a2613af/29d8241f4de5174e.jpg" alt="cover" /&gt;&lt;/p&gt;&lt;p&gt;好久没在&lt;a href="/category/%E6%96%87%E5%AD%97%E9%98%81/"&gt;文字阁&lt;/a&gt;分类里发文了&lt;del&gt;这当然不是因为我好久没读书了&lt;/del&gt;。&lt;/p&gt;</content>
  </entry>
</feed>
"#;
        let entries = parse_feed_entries(xml.as_bytes()).unwrap();
        let raw = entries[0].content.as_deref().unwrap_or("");
        assert!(
            raw.contains('<') || raw.contains("&lt;"),
            "raw content missing tag markers: {}",
            raw
        );
        let doc = entry_to_document(&entries[0]).unwrap();
        let normalized = normalize_space(&doc.content);
        assert!(normalized.contains("好久没在"));
        assert!(normalized.contains("文字阁"));
        assert!(normalized.contains("这当然不是因为我好久没读书了"));
        assert!(
            !doc.content.contains("pimg"),
            "content still has tag text: {}",
            doc.content
        );
        assert!(!doc.content.contains("href"));
        assert!(!doc.content.contains("del"));
        assert!(!doc.content.contains('<'));
        assert!(!doc.content.contains('>'));
    }

    fn normalize_space(input: &str) -> String {
        input
            .split_whitespace()
            .filter(|part| !part.is_empty())
            .collect::<Vec<_>>()
            .join(" ")
    }
}
