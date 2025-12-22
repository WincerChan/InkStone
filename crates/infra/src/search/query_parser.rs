use inkstone_core::domain::search::SearchQuery;
use inkstone_core::types::time_range::TimeRange;
use thiserror::Error;

const MAX_KEYWORDS: usize = 10;

#[derive(Debug, Error)]
pub enum QueryParseError {
    #[error("control characters are not allowed")]
    ControlCharacter,
    #[error("empty search query")]
    EmptyQuery,
    #[error("empty search token")]
    EmptyToken,
    #[error("too many keywords (max {0})")]
    TooManyKeywords(usize),
    #[error("invalid range filter: {0}")]
    InvalidRange(String),
    #[error("duplicate filter: {0}")]
    DuplicateFilter(&'static str),
    #[error("invalid tags filter: {0}")]
    InvalidTags(String),
    #[error("invalid category filter: {0}")]
    InvalidCategory(String),
}

pub fn parse_query(input: &str) -> Result<SearchQuery, QueryParseError> {
    if input.chars().any(|ch| ch.is_control()) {
        return Err(QueryParseError::ControlCharacter);
    }
    let normalized = normalize_whitespace(input);
    if normalized.is_empty() {
        return Err(QueryParseError::EmptyQuery);
    }

    let mut query = SearchQuery::default();
    for token in normalized.split_whitespace() {
        if token.trim().is_empty() {
            return Err(QueryParseError::EmptyToken);
        }
        if let Some(value) = token.strip_prefix("range:") {
            if query.range.is_some() {
                return Err(QueryParseError::DuplicateFilter("range"));
            }
            let range = TimeRange::parse(value)
                .map_err(|_| QueryParseError::InvalidRange(value.to_string()))?;
            query.range = Some(range);
            continue;
        }
        if let Some(value) = token.strip_prefix("tags:") {
            let tags = parse_list(value)
                .ok_or_else(|| QueryParseError::InvalidTags(value.to_string()))?;
            query.tags.extend(tags);
            continue;
        }
        if let Some(value) = token.strip_prefix("category:") {
            if query.category.is_some() {
                return Err(QueryParseError::DuplicateFilter("category"));
            }
            let value = value.trim();
            if value.is_empty() {
                return Err(QueryParseError::InvalidCategory(value.to_string()));
            }
            query.category = Some(value.to_string());
            continue;
        }
        query.keywords.push(token.to_string());
        if query.keywords.len() > MAX_KEYWORDS {
            return Err(QueryParseError::TooManyKeywords(MAX_KEYWORDS));
        }
    }

    Ok(query)
}

fn parse_list(input: &str) -> Option<Vec<String>> {
    let trimmed = input.trim();
    if trimmed.is_empty() {
        return None;
    }
    let mut items = Vec::new();
    for item in trimmed.split(',') {
        let item = item.trim();
        if item.is_empty() {
            continue;
        }
        items.push(item.to_string());
    }
    if items.is_empty() {
        None
    } else {
        Some(items)
    }
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
    use super::*;

    #[test]
    fn parse_keywords_only() {
        let query = parse_query("Python Linux").unwrap();
        assert_eq!(query.keywords, vec!["Python", "Linux"]);
    }

    #[test]
    fn parse_range_filter() {
        let query = parse_query("range:2020-01-01~").unwrap();
        assert!(query.range.is_some());
    }

    #[test]
    fn parse_tags_filter() {
        let query = parse_query("tags:Python,Linux").unwrap();
        assert_eq!(query.tags, vec!["Python", "Linux"]);
    }

    #[test]
    fn parse_category_filter() {
        let query = parse_query("category:share").unwrap();
        assert_eq!(query.category, Some("share".to_string()));
    }

    #[test]
    fn parse_combined_filters() {
        let query = parse_query("Python range:2018-01-01~2020-01-01 tags:Rust").unwrap();
        assert_eq!(query.keywords, vec!["Python"]);
        assert_eq!(query.tags, vec!["Rust"]);
        assert!(query.range.is_some());
    }

    #[test]
    fn parse_empty_query_returns_error() {
        let err = parse_query(" ").unwrap_err();
        assert!(matches!(err, QueryParseError::EmptyQuery));
    }

    #[test]
    fn parse_rejects_control_characters() {
        let err = parse_query("Python\nLinux").unwrap_err();
        assert!(matches!(err, QueryParseError::ControlCharacter));
    }

    #[test]
    fn parse_rejects_too_many_keywords() {
        let query = vec![
            "one", "two", "three", "four", "five", "six", "seven", "eight", "nine", "ten",
            "eleven",
        ]
        .join(" ");
        let err = parse_query(&query).unwrap_err();
        assert!(matches!(err, QueryParseError::TooManyKeywords(_)));
    }
}
