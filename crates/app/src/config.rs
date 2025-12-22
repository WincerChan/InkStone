use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct AppConfig {
    pub http_addr: SocketAddr,
    pub index_dir: PathBuf,
    pub feed_url: String,
    pub poll_interval: Duration,
    pub douban_poll_interval: Duration,
    pub request_timeout: Duration,
    pub max_search_limit: usize,
    pub database_url: Option<String>,
    pub douban_max_pages: usize,
    pub douban_uid: String,
    pub douban_cookie: String,
    pub douban_user_agent: String,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid socket address: {0}")]
    InvalidSocket(String),
    #[error("invalid integer for {0}: {1}")]
    InvalidNumber(&'static str, String),
    #[error("invalid value for {0}: {1}")]
    InvalidValue(&'static str, String),
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let http_addr_raw = read_string("INKSTONE_HTTP_ADDR", "127.0.0.1:8080");
        let http_addr = http_addr_raw
            .parse()
            .map_err(|_| ConfigError::InvalidSocket(http_addr_raw.clone()))?;
        let index_dir = PathBuf::from(read_string("INKSTONE_INDEX_DIR", "./data/index"));
        let feed_url = read_string(
            "INKSTONE_FEED_URL",
            "https://velite-refactor.blog-8fo.pages.dev/atom.xml",
        );
        if feed_url.trim().is_empty() {
            return Err(ConfigError::InvalidValue(
                "INKSTONE_FEED_URL",
                feed_url,
            ));
        }
        let poll_interval_secs = read_u64("INKSTONE_POLL_INTERVAL_SECS", 300)?;
        let douban_poll_interval_secs =
            read_u64("INKSTONE_DOUBAN_POLL_INTERVAL_SECS", poll_interval_secs)?;
        let request_timeout_secs = read_u64("INKSTONE_REQUEST_TIMEOUT_SECS", 15)?;
        let max_search_limit = read_usize("INKSTONE_MAX_SEARCH_LIMIT", 50)?;
        let database_url = read_optional_string("INKSTONE_DATABASE_URL");
        let douban_max_pages = read_usize("INKSTONE_DOUBAN_MAX_PAGES", 1)?;
        let douban_uid = read_string("INKSTONE_DOUBAN_UID", "93562087");
        let douban_cookie = read_string("INKSTONE_DOUBAN_COOKIE", "bid=3EHqn8aRvcI");
        let douban_user_agent = read_string(
            "INKSTONE_DOUBAN_USER_AGENT",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
        );

        Ok(Self {
            http_addr,
            index_dir,
            feed_url,
            poll_interval: Duration::from_secs(poll_interval_secs),
            douban_poll_interval: Duration::from_secs(douban_poll_interval_secs),
            request_timeout: Duration::from_secs(request_timeout_secs),
            max_search_limit,
            database_url,
            douban_max_pages,
            douban_uid,
            douban_cookie,
            douban_user_agent,
        })
    }
}

pub fn load_dotenv() -> Result<(), std::io::Error> {
    let path = Path::new(".env");
    if !path.exists() {
        return Ok(());
    }
    let contents = std::fs::read_to_string(path)?;
    for (key, value) in parse_dotenv(&contents) {
        if std::env::var_os(&key).is_none() {
            // Safety: invoked during startup before any threads are spawned.
            unsafe {
                std::env::set_var(key, value);
            }
        }
    }
    Ok(())
}

fn read_string(key: &'static str, default: &'static str) -> String {
    std::env::var(key).unwrap_or_else(|_| default.to_string())
}

fn read_u64(key: &'static str, default: u64) -> Result<u64, ConfigError> {
    let raw = std::env::var(key).unwrap_or_else(|_| default.to_string());
    raw.parse()
        .map_err(|_| ConfigError::InvalidNumber(key, raw))
}

fn read_usize(key: &'static str, default: usize) -> Result<usize, ConfigError> {
    let raw = std::env::var(key).unwrap_or_else(|_| default.to_string());
    raw.parse()
        .map_err(|_| ConfigError::InvalidNumber(key, raw))
}

fn read_optional_string(key: &'static str) -> Option<String> {
    let value = std::env::var(key).unwrap_or_default();
    let trimmed = value.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed.to_string())
    }
}

fn parse_dotenv(contents: &str) -> Vec<(String, String)> {
    contents
        .lines()
        .filter_map(parse_dotenv_line)
        .collect()
}

fn parse_dotenv_line(line: &str) -> Option<(String, String)> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('#') {
        return None;
    }
    let trimmed = trimmed.strip_prefix("export ").unwrap_or(trimmed);
    let (key, value) = trimmed.split_once('=')?;
    let key = key.trim();
    if key.is_empty() {
        return None;
    }
    let value = parse_dotenv_value(value.trim());
    Some((key.to_string(), value))
}

fn parse_dotenv_value(value: &str) -> String {
    if let Some(stripped) = value.strip_prefix('"').and_then(|inner| inner.strip_suffix('"')) {
        return unescape_double_quoted(stripped);
    }
    if let Some(stripped) = value.strip_prefix('\'').and_then(|inner| inner.strip_suffix('\'')) {
        return stripped.to_string();
    }
    value.to_string()
}

fn unescape_double_quoted(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch == '\\' {
            match chars.next() {
                Some('n') => output.push('\n'),
                Some('r') => output.push('\r'),
                Some('t') => output.push('\t'),
                Some('\\') => output.push('\\'),
                Some('"') => output.push('"'),
                Some(other) => {
                    output.push('\\');
                    output.push(other);
                }
                None => output.push('\\'),
            }
        } else {
            output.push(ch);
        }
    }
    output
}

#[cfg(test)]
mod tests {
    use super::parse_dotenv_line;

    #[test]
    fn parse_dotenv_line_basic() {
        let (key, value) = parse_dotenv_line("FOO=bar").unwrap();
        assert_eq!(key, "FOO");
        assert_eq!(value, "bar");
    }

    #[test]
    fn parse_dotenv_line_export() {
        let (key, value) = parse_dotenv_line("export FOO=bar").unwrap();
        assert_eq!(key, "FOO");
        assert_eq!(value, "bar");
    }

    #[test]
    fn parse_dotenv_line_double_quotes() {
        let (key, value) = parse_dotenv_line(r#"FOO="hello world""#).unwrap();
        assert_eq!(key, "FOO");
        assert_eq!(value, "hello world");
    }

    #[test]
    fn parse_dotenv_line_single_quotes() {
        let (key, value) = parse_dotenv_line("FOO='hello world'").unwrap();
        assert_eq!(key, "FOO");
        assert_eq!(value, "hello world");
    }

    #[test]
    fn parse_dotenv_line_escaped() {
        let (key, value) = parse_dotenv_line(r#"FOO="line\n\"quote\"""#).unwrap();
        assert_eq!(key, "FOO");
        assert_eq!(value, "line\n\"quote\"");
    }

    #[test]
    fn parse_dotenv_line_comment() {
        assert!(parse_dotenv_line("# comment").is_none());
        assert!(parse_dotenv_line("   ").is_none());
    }
}
