use std::net::SocketAddr;
use std::path::{Path, PathBuf};
use std::time::Duration;

use thiserror::Error;

#[derive(Debug, Clone)]
pub struct PublicConfig {
    pub http_addr: SocketAddr,
    pub index_dir: PathBuf,
    pub max_search_limit: usize,
    pub database_url: Option<String>,
    pub cookie_secret: Option<String>,
    pub stats_secret: Option<String>,
    pub search_hash_secret: Option<String>,
    pub public_token_secret: Option<String>,
    pub cors_allow_origins: Vec<String>,
    pub pulse_allowed_slds: Vec<String>,
}

#[derive(Debug, Clone)]
pub struct AdminConfig {
    pub http_addr: SocketAddr,
    pub index_dir: PathBuf,
    pub feed_url: String,
    pub poll_interval: Duration,
    pub douban_poll_interval: Duration,
    pub comments_sync_interval: Duration,
    pub request_timeout: Duration,
    pub database_url: Option<String>,
    pub douban_max_pages: usize,
    pub douban_uid: String,
    pub douban_cookie: String,
    pub douban_user_agent: String,
    pub douban_poster_r2_endpoint: Option<String>,
    pub douban_poster_r2_bucket: Option<String>,
    pub douban_poster_r2_access_key_id: Option<String>,
    pub douban_poster_r2_secret_access_key: Option<String>,
    pub douban_poster_r2_public_base_url: Option<String>,
    pub douban_poster_r2_region: String,
    pub cookie_secret: Option<String>,
    pub stats_secret: Option<String>,
    pub public_token_secret: Option<String>,
    pub admin_password_hash: Option<String>,
    pub admin_token_secret: Option<String>,
    pub github_webhook_secret: Option<String>,
    pub github_discussion_webhook_secret: Option<String>,
    pub github_app_id: Option<u64>,
    pub github_app_installation_id: Option<u64>,
    pub github_app_private_key: Option<String>,
    pub github_repo_owner: Option<String>,
    pub github_repo_name: Option<String>,
    pub github_discussion_category_id: Option<String>,
    pub cors_allow_origins: Vec<String>,
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

impl PublicConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let shared = read_shared()?;
        let max_search_limit = read_usize("INKSTONE_MAX_SEARCH_LIMIT", 50)?;
        let search_hash_secret = read_optional_string("INKSTONE_SEARCH_HASH_SECRET")?;
        let pulse_allowed_slds = read_csv("INKSTONE_PULSE_ALLOWED_SLD")?;
        Ok(Self {
            http_addr: shared.http_addr,
            index_dir: shared.index_dir,
            max_search_limit,
            database_url: shared.database_url,
            cookie_secret: shared.cookie_secret,
            stats_secret: shared.stats_secret,
            search_hash_secret,
            public_token_secret: shared.public_token_secret,
            cors_allow_origins: shared.cors_allow_origins,
            pulse_allowed_slds,
        })
    }
}

impl AdminConfig {
    pub fn from_env() -> Result<Self, ConfigError> {
        let shared = read_shared()?;
        let feed_url = read_string(
            "INKSTONE_FEED_URL",
            "https://refactor-styles.blog-8fo.pages.dev/search-index.json",
        )?;
        if feed_url.trim().is_empty() {
            return Err(ConfigError::InvalidValue("INKSTONE_FEED_URL", feed_url));
        }
        let poll_interval_secs = read_u64("INKSTONE_POLL_INTERVAL_SECS", 300)?;
        let douban_poll_interval_secs =
            read_u64("INKSTONE_DOUBAN_POLL_INTERVAL_SECS", poll_interval_secs)?;
        let comments_sync_secs = read_u64("INKSTONE_COMMENTS_SYNC_SECS", 432000)?;
        let request_timeout_secs = read_u64("INKSTONE_REQUEST_TIMEOUT_SECS", 15)?;
        let douban_max_pages = read_usize("INKSTONE_DOUBAN_MAX_PAGES", 1)?;
        let douban_uid = read_string("INKSTONE_DOUBAN_UID", "93562087")?;
        let douban_cookie = read_string("INKSTONE_DOUBAN_COOKIE", "bid=3EHqn8aRvcI")?;
        let douban_user_agent = read_string(
            "INKSTONE_DOUBAN_USER_AGENT",
            "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/120.0 Safari/537.36",
        )?;
        let douban_poster_r2_endpoint = read_optional_string("INKSTONE_DOUBAN_POSTER_R2_ENDPOINT")?;
        let douban_poster_r2_bucket = read_optional_string("INKSTONE_DOUBAN_POSTER_R2_BUCKET")?;
        let douban_poster_r2_access_key_id =
            read_optional_string("INKSTONE_DOUBAN_POSTER_R2_ACCESS_KEY_ID")?;
        let douban_poster_r2_secret_access_key =
            read_optional_string("INKSTONE_DOUBAN_POSTER_R2_SECRET_ACCESS_KEY")?;
        let douban_poster_r2_public_base_url =
            read_optional_string("INKSTONE_DOUBAN_POSTER_R2_PUBLIC_BASE_URL")?;
        let douban_poster_r2_region = read_string("INKSTONE_DOUBAN_POSTER_R2_REGION", "auto")?;
        let admin_password_hash = read_optional_string("INKSTONE_ADMIN_PASSWORD_HASH")?;
        let admin_token_secret = read_optional_string("INKSTONE_ADMIN_TOKEN_SECRET")?;
        let github_webhook_secret = read_optional_string("INKSTONE_GITHUB_WEBHOOK_SECRET")?;
        let github_discussion_webhook_secret =
            read_optional_string("INKSTONE_GITHUB_DISCUSSION_WEBHOOK_SECRET")?;
        let github_app_id = read_optional_u64("INKSTONE_GITHUB_APP_ID")?;
        let github_app_installation_id = read_optional_u64("INKSTONE_GITHUB_APP_INSTALLATION_ID")?;
        let github_app_private_key = read_optional_string("INKSTONE_GITHUB_APP_PRIVATE_KEY")?;
        let github_repo_owner = read_optional_string("INKSTONE_GITHUB_REPO_OWNER")?;
        let github_repo_name = read_optional_string("INKSTONE_GITHUB_REPO_NAME")?;
        let github_discussion_category_id =
            read_optional_string("INKSTONE_GITHUB_DISCUSSION_CATEGORY_ID")?;

        Ok(Self {
            http_addr: shared.http_addr,
            index_dir: shared.index_dir,
            feed_url,
            poll_interval: Duration::from_secs(poll_interval_secs),
            douban_poll_interval: Duration::from_secs(douban_poll_interval_secs),
            comments_sync_interval: Duration::from_secs(comments_sync_secs),
            request_timeout: Duration::from_secs(request_timeout_secs),
            database_url: shared.database_url,
            douban_max_pages,
            douban_uid,
            douban_cookie,
            douban_user_agent,
            douban_poster_r2_endpoint,
            douban_poster_r2_bucket,
            douban_poster_r2_access_key_id,
            douban_poster_r2_secret_access_key,
            douban_poster_r2_public_base_url,
            douban_poster_r2_region,
            cookie_secret: shared.cookie_secret,
            stats_secret: shared.stats_secret,
            public_token_secret: shared.public_token_secret,
            admin_password_hash,
            admin_token_secret,
            github_webhook_secret,
            github_discussion_webhook_secret,
            github_app_id,
            github_app_installation_id,
            github_app_private_key,
            github_repo_owner,
            github_repo_name,
            github_discussion_category_id,
            cors_allow_origins: shared.cors_allow_origins,
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

#[derive(Debug)]
struct SharedConfig {
    http_addr: SocketAddr,
    index_dir: PathBuf,
    database_url: Option<String>,
    cookie_secret: Option<String>,
    stats_secret: Option<String>,
    public_token_secret: Option<String>,
    cors_allow_origins: Vec<String>,
}

fn read_shared() -> Result<SharedConfig, ConfigError> {
    let http_addr_raw = read_string("INKSTONE_HTTP_ADDR", "127.0.0.1:8080")?;
    let http_addr = http_addr_raw
        .parse()
        .map_err(|_| ConfigError::InvalidSocket(http_addr_raw.clone()))?;
    let index_dir = PathBuf::from(read_string("INKSTONE_INDEX_DIR", "./data/index")?);
    let database_url = read_optional_string("INKSTONE_DATABASE_URL")?;
    let cookie_secret = read_optional_string("INKSTONE_COOKIE_SECRET")?;
    let stats_secret = read_optional_string("INKSTONE_STATS_SECRET")?;
    let public_token_secret = read_optional_string("INKSTONE_PUBLIC_TOKEN_SECRET")?;
    let cors_allow_origins = read_csv("INKSTONE_CORS_ALLOW_ORIGINS")?;
    Ok(SharedConfig {
        http_addr,
        index_dir,
        database_url,
        cookie_secret,
        stats_secret,
        public_token_secret,
        cors_allow_origins,
    })
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
        .filter_map(|line| parse_dotenv_line(line))
        .collect()
}

fn parse_dotenv_line(raw: &str) -> Option<(String, String)> {
    let line = raw.trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    let line = line.strip_prefix("export ").unwrap_or(line);
    let (key, value) = line.split_once('=')?;
    let key = key.trim();
    let value = value.trim();
    if key.is_empty() {
        return None;
    }
    let unquoted = if let Some(value) = value.strip_prefix('"').and_then(|v| v.strip_suffix('"')) {
        Some(
            value
                .replace("\\n", "\n")
                .replace("\\r", "\r")
                .replace("\\t", "\t"),
        )
    } else if let Some(value) = value.strip_prefix('\'').and_then(|v| v.strip_suffix('\'')) {
        Some(value.to_string())
    } else {
        None
    };
    let value = unquoted.unwrap_or_else(|| value.to_string());
    Some((key.to_string(), value))
}

#[cfg(test)]
mod tests {
    use super::{ConfigError, parse_dotenv_line, read_raw};

    #[test]
    fn parse_dotenv_line_basic() {
        let (key, value) = parse_dotenv_line("FOO=bar").unwrap();
        assert_eq!(key, "FOO");
        assert_eq!(value, "bar");
    }

    #[test]
    fn parse_dotenv_line_comment() {
        assert!(parse_dotenv_line("# foo=bar").is_none());
    }

    #[test]
    fn parse_dotenv_line_export() {
        let (key, value) = parse_dotenv_line("export FOO=bar").unwrap();
        assert_eq!(key, "FOO");
        assert_eq!(value, "bar");
    }

    #[test]
    fn parse_dotenv_line_double_quotes() {
        let (key, value) = parse_dotenv_line("FOO=\"bar\"").unwrap();
        assert_eq!(key, "FOO");
        assert_eq!(value, "bar");
    }

    #[test]
    fn parse_dotenv_line_single_quotes() {
        let (key, value) = parse_dotenv_line("FOO='bar'").unwrap();
        assert_eq!(key, "FOO");
        assert_eq!(value, "bar");
    }

    #[test]
    fn parse_dotenv_line_escaped() {
        let (_, value) = parse_dotenv_line("FOO=\"bar\\n\"").unwrap();
        assert_eq!(value, "bar\n");
    }

    #[test]
    fn read_string_errors_on_missing_file() {
        unsafe {
            std::env::set_var("INKSTONE_SAMPLE_FILE", "/tmp/missing.txt");
        }
        let err = read_raw("INKSTONE_SAMPLE").unwrap_err();
        assert!(matches!(err, ConfigError::InvalidFile(_, _)));
        unsafe {
            std::env::remove_var("INKSTONE_SAMPLE_FILE");
        }
    }

    #[test]
    fn read_string_uses_file_when_env_empty() {
        let path = std::env::temp_dir().join("inkstone-config.txt");
        std::fs::write(&path, "value").unwrap();
        unsafe {
            std::env::set_var("INKSTONE_SAMPLE_FILE", &path);
        }
        let value = read_raw("INKSTONE_SAMPLE").unwrap().unwrap();
        assert_eq!(value, "value");
        unsafe {
            std::env::remove_var("INKSTONE_SAMPLE_FILE");
        }
        let _ = std::fs::remove_file(path);
    }
}
