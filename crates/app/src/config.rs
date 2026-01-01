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
    pub comments_sync_interval: Duration,
    pub request_timeout: Duration,
    pub max_search_limit: usize,
    pub database_url: Option<String>,
    pub douban_max_pages: usize,
    pub douban_uid: String,
    pub douban_cookie: String,
    pub douban_user_agent: String,
    pub cookie_secret: Option<String>,
    pub stats_secret: Option<String>,
    pub valid_paths_url: String,
    pub kudos_flush_interval: Duration,
    pub github_webhook_secret: Option<String>,
    pub github_discussion_webhook_secret: Option<String>,
    pub github_app_id: Option<u64>,
    pub github_app_installation_id: Option<u64>,
    pub github_app_private_key: Option<String>,
    pub github_repo_owner: Option<String>,
    pub github_repo_name: Option<String>,
    pub github_discussion_category_id: Option<String>,
    pub cors_allow_origins: Vec<String>,
    pub pulse_allowed_slds: Vec<String>,
    pub admin_password_hash: Option<String>,
    pub admin_token_secret: Option<String>,
}

#[derive(Debug, Error)]
pub enum ConfigError {
    #[error("invalid socket address: {0}")]
    InvalidSocket(String),
    #[error("invalid integer for {0}: {1}")]
    InvalidNumber(&'static str, String),
    #[error("invalid value for {0}: {1}")]
    InvalidValue(&'static str, String),
    #[error("invalid file for {0}: {1}")]
    InvalidFile(&'static str, String),
}

impl AppConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let http_addr_raw = read_string("INKSTONE_HTTP_ADDR", "127.0.0.1:8080")?;
        let http_addr = http_addr_raw
            .parse()
            .map_err(|_| ConfigError::InvalidSocket(http_addr_raw.clone()))?;
        let index_dir = PathBuf::from(read_string("INKSTONE_INDEX_DIR", "./data/index")?);
        let feed_url = read_string(
            "INKSTONE_FEED_URL",
            "https://refactor-styles.blog-8fo.pages.dev/search-index.json",
        )?;
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
        let database_url = read_optional_string("INKSTONE_DATABASE_URL")?;
        let comments_sync_secs = read_u64("INKSTONE_COMMENTS_SYNC_SECS", 432000)?;
        let douban_max_pages = read_usize("INKSTONE_DOUBAN_MAX_PAGES", 1)?;
        let douban_uid = read_string("INKSTONE_DOUBAN_UID", "93562087")?;
        let douban_cookie = read_string("INKSTONE_DOUBAN_COOKIE", "bid=3EHqn8aRvcI")?;
        let douban_user_agent = read_string(
            "INKSTONE_DOUBAN_USER_AGENT",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
        )?;
        let cookie_secret = read_optional_string("INKSTONE_COOKIE_SECRET")?;
        let stats_secret = read_optional_string("INKSTONE_STATS_SECRET")?;
        let valid_paths_url = read_string(
            "INKSTONE_VALID_PATHS_URL",
            "https://velite-refactor.blog-8fo.pages.dev/valid_paths.txt",
        )?;
        let kudos_flush_secs = read_u64("INKSTONE_KUDOS_FLUSH_SECS", 60)?;
        let github_webhook_secret = read_optional_string("INKSTONE_GITHUB_WEBHOOK_SECRET")?;
        let github_discussion_webhook_secret =
            read_optional_string("INKSTONE_GITHUB_DISCUSSION_WEBHOOK_SECRET")?;
        let github_app_id = read_optional_u64("INKSTONE_GITHUB_APP_ID")?;
        let github_app_installation_id =
            read_optional_u64("INKSTONE_GITHUB_APP_INSTALLATION_ID")?;
        let github_app_private_key = read_optional_string("INKSTONE_GITHUB_APP_PRIVATE_KEY")?;
        let github_repo_owner = read_optional_string("INKSTONE_GITHUB_REPO_OWNER")?;
        let github_repo_name = read_optional_string("INKSTONE_GITHUB_REPO_NAME")?;
        let github_discussion_category_id =
            read_optional_string("INKSTONE_GITHUB_DISCUSSION_CATEGORY_ID")?;
        let cors_allow_origins = read_csv("INKSTONE_CORS_ALLOW_ORIGINS")?;
        let pulse_allowed_slds = read_csv("INKSTONE_PULSE_ALLOWED_SLD")?;
        let admin_password_hash = read_optional_string("INKSTONE_ADMIN_PASSWORD_HASH")?;
        let admin_token_secret = read_optional_string("INKSTONE_ADMIN_TOKEN_SECRET")?;

        Ok(Self {
            http_addr,
            index_dir,
            feed_url,
            poll_interval: Duration::from_secs(poll_interval_secs),
            douban_poll_interval: Duration::from_secs(douban_poll_interval_secs),
            comments_sync_interval: Duration::from_secs(comments_sync_secs),
            request_timeout: Duration::from_secs(request_timeout_secs),
            max_search_limit,
            database_url,
            douban_max_pages,
            douban_uid,
            douban_cookie,
            douban_user_agent,
            cookie_secret,
            stats_secret,
            valid_paths_url,
            kudos_flush_interval: Duration::from_secs(kudos_flush_secs),
            github_webhook_secret,
            github_discussion_webhook_secret,
            github_app_id,
            github_app_installation_id,
            github_app_private_key,
            github_repo_owner,
            github_repo_name,
            github_discussion_category_id,
            cors_allow_origins,
            pulse_allowed_slds,
            admin_password_hash,
            admin_token_secret,
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

fn read_string(key: &'static str, default: &'static str) -> Result<String, ConfigError> {
    Ok(read_raw(key)?.unwrap_or_else(|| default.to_string()))
}

fn read_u64(key: &'static str, default: u64) -> Result<u64, ConfigError> {
    let raw = read_raw(key)?.unwrap_or_else(|| default.to_string());
    raw.parse()
        .map_err(|_| ConfigError::InvalidNumber(key, raw))
}

fn read_optional_u64(key: &'static str) -> Result<Option<u64>, ConfigError> {
    let raw = match read_raw(key)? {
        Some(value) => value,
        None => return Ok(None),
    };
    let value = raw
        .parse()
        .map_err(|_| ConfigError::InvalidNumber(key, raw))?;
    Ok(Some(value))
}

fn read_usize(key: &'static str, default: usize) -> Result<usize, ConfigError> {
    let raw = read_raw(key)?.unwrap_or_else(|| default.to_string());
    raw.parse()
        .map_err(|_| ConfigError::InvalidNumber(key, raw))
}

fn read_optional_string(key: &'static str) -> Result<Option<String>, ConfigError> {
    read_raw(key)
}

fn read_csv(key: &'static str) -> Result<Vec<String>, ConfigError> {
    let raw = read_raw(key)?.unwrap_or_default();
    Ok(raw
        .split(',')
        .map(str::trim)
        .filter(|value| !value.is_empty())
        .map(|value| value.to_string())
        .collect())
}

fn read_raw(key: &'static str) -> Result<Option<String>, ConfigError> {
    if let Ok(value) = std::env::var(key) {
        let trimmed = value.trim();
        if !trimmed.is_empty() {
            return Ok(Some(trimmed.to_string()));
        }
    }

    let file_key = format!("{key}_FILE");
    let file_path = std::env::var(&file_key).unwrap_or_default();
    let file_path = file_path.trim();
    if file_path.is_empty() {
        return Ok(None);
    }

    let contents = std::fs::read_to_string(file_path)
        .map_err(|_| ConfigError::InvalidFile(key, file_path.to_string()))?;
    let trimmed = contents.trim();
    if trimmed.is_empty() {
        return Ok(None);
    }
    Ok(Some(trimmed.to_string()))
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
    use super::{parse_dotenv_line, read_string, read_u64, ConfigError};
    use std::path::PathBuf;
    use std::sync::Mutex;
    use std::time::{SystemTime, UNIX_EPOCH};

    static ENV_LOCK: Mutex<()> = Mutex::new(());

    struct EnvGuard {
        vars: Vec<(&'static str, Option<String>)>,
    }

    impl EnvGuard {
        fn new() -> Self {
            Self { vars: Vec::new() }
        }

        fn set(&mut self, key: &'static str, value: Option<&str>) {
            let prev = std::env::var(key).ok();
            self.vars.push((key, prev));
            match value {
                Some(value) => unsafe {
                    std::env::set_var(key, value);
                },
                None => unsafe {
                    std::env::remove_var(key);
                },
            }
        }
    }

    impl Drop for EnvGuard {
        fn drop(&mut self) {
            for (key, prev) in self.vars.drain(..).rev() {
                match prev {
                    Some(value) => unsafe {
                        std::env::set_var(key, value);
                    },
                    None => unsafe {
                        std::env::remove_var(key);
                    },
                }
            }
        }
    }

    fn temp_path(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        let mut path = std::env::temp_dir();
        path.push(format!("inkstone_{name}_{nanos}"));
        path
    }

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

    #[test]
    fn read_string_uses_file_when_env_empty() {
        let _lock = ENV_LOCK.lock().unwrap();
        let path = temp_path("config_string");
        std::fs::write(&path, "from_file\n").unwrap();
        let mut env = EnvGuard::new();
        env.set("INKSTONE_TEST_VALUE", Some(""));
        env.set(
            "INKSTONE_TEST_VALUE_FILE",
            Some(path.to_str().expect("temp path utf8")),
        );

        let value = read_string("INKSTONE_TEST_VALUE", "default").unwrap();
        assert_eq!(value, "from_file");

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_u64_uses_file_when_missing() {
        let _lock = ENV_LOCK.lock().unwrap();
        let path = temp_path("config_number");
        std::fs::write(&path, "42").unwrap();
        let mut env = EnvGuard::new();
        env.set("INKSTONE_TEST_NUM", None);
        env.set(
            "INKSTONE_TEST_NUM_FILE",
            Some(path.to_str().expect("temp path utf8")),
        );

        let value = read_u64("INKSTONE_TEST_NUM", 7).unwrap();
        assert_eq!(value, 42);

        std::fs::remove_file(path).unwrap();
    }

    #[test]
    fn read_string_errors_on_missing_file() {
        let _lock = ENV_LOCK.lock().unwrap();
        let path = temp_path("config_missing");
        let mut env = EnvGuard::new();
        env.set("INKSTONE_TEST_MISSING", None);
        env.set(
            "INKSTONE_TEST_MISSING_FILE",
            Some(path.to_str().expect("temp path utf8")),
        );

        let err = read_string("INKSTONE_TEST_MISSING", "default").unwrap_err();
        assert!(matches!(err, ConfigError::InvalidFile("INKSTONE_TEST_MISSING", _)));
    }
}
