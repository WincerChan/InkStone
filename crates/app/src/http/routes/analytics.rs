use axum::body::Bytes;
use axum::extract::{Extension, State};
use axum::http::header::REFERER;
use axum::http::{HeaderMap, StatusCode};
use axum::response::IntoResponse;
use axum::Json;
use chrono::Utc;
use serde::de::DeserializeOwned;
use serde::Deserialize;
use thiserror::Error;
use uuid::Uuid;

use crate::http::middleware::bid_cookie::ClientIds;
use crate::state::AppState;
use inkstone_infra::db::{upsert_engagement, upsert_page_view, AnalyticsRepoError, PageViewRecord};

const MAX_PATH_LEN: usize = 512;

#[derive(Debug, Deserialize)]
pub struct PulsePvRequest {
    pub page_instance_id: Option<String>,
    pub path: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct PulseEngageRequest {
    pub page_instance_id: Option<String>,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Error)]
pub enum PulseApiError {
    #[error("page_instance_id is required")]
    MissingPageInstanceId,
    #[error("page_instance_id is invalid")]
    InvalidPageInstanceId,
    #[error("path is required")]
    MissingPath,
    #[error("path is invalid")]
    InvalidPath,
    #[error("duration_ms is invalid")]
    InvalidDuration,
    #[error("invalid payload")]
    InvalidPayload,
    #[error("valid paths not loaded")]
    ValidPathsUnavailable,
    #[error("path is not allowed")]
    PathNotAllowed,
    #[error("db not configured")]
    DbUnavailable,
    #[error("db error: {0}")]
    Db(#[from] AnalyticsRepoError),
}

#[derive(Debug, serde::Serialize)]
struct ErrorBody {
    error: String,
}

pub async fn post_pv(
    State(state): State<AppState>,
    Extension(ids): Extension<ClientIds>,
    headers: HeaderMap,
    body: Bytes,
) -> Result<StatusCode, PulseApiError> {
    let payload: PulsePvRequest = parse_json(&body)?;
    let page_instance_id = parse_uuid(payload.page_instance_id.as_deref())?;
    let path = normalize_path(payload.path.as_deref())?;
    ensure_valid_path(&state, &path).await?;
    let ua = header_value(&headers, "user-agent");
    let ua_family = ua.and_then(parse_ua_family);
    let device = ua.and_then(parse_device);
    let ref_host = header_value(&headers, REFERER.as_str()).and_then(parse_ref_host);
    let source_type = Some(if ref_host.is_some() {
        "referral".to_string()
    } else {
        "direct".to_string()
    });
    let country = extract_country(&headers);
    let record = PageViewRecord {
        page_instance_id,
        duration_ms: None,
        user_stats_id: Some(ids.stats_id),
        path: Some(path),
        ts: Utc::now(),
        ua_family,
        device,
        source_type,
        ref_host,
        country,
    };
    let pool = state.db.as_ref().ok_or(PulseApiError::DbUnavailable)?;
    upsert_page_view(pool, &record).await?;
    Ok(StatusCode::NO_CONTENT)
}

pub async fn post_engage(
    State(state): State<AppState>,
    body: Bytes,
) -> Result<StatusCode, PulseApiError> {
    let payload: PulseEngageRequest = parse_json(&body)?;
    let page_instance_id = parse_uuid(payload.page_instance_id.as_deref())?;
    let duration_ms = payload.duration_ms.ok_or(PulseApiError::InvalidDuration)?;
    if duration_ms < 0 {
        return Err(PulseApiError::InvalidDuration);
    }
    let pool = state.db.as_ref().ok_or(PulseApiError::DbUnavailable)?;
    upsert_engagement(pool, page_instance_id, duration_ms).await?;
    Ok(StatusCode::NO_CONTENT)
}

fn parse_uuid(value: Option<&str>) -> Result<Uuid, PulseApiError> {
    let trimmed = value.unwrap_or("").trim();
    if trimmed.is_empty() {
        return Err(PulseApiError::MissingPageInstanceId);
    }
    Uuid::parse_str(trimmed).map_err(|_| PulseApiError::InvalidPageInstanceId)
}

fn parse_json<T>(body: &Bytes) -> Result<T, PulseApiError>
where
    T: DeserializeOwned,
{
    if body.is_empty() {
        return Err(PulseApiError::InvalidPayload);
    }
    serde_json::from_slice(body).map_err(|_| PulseApiError::InvalidPayload)
}

fn normalize_path(path: Option<&str>) -> Result<String, PulseApiError> {
    let trimmed = path.unwrap_or("").trim();
    if trimmed.is_empty() {
        return Err(PulseApiError::MissingPath);
    }
    if trimmed.len() > MAX_PATH_LEN || !trimmed.starts_with('/') {
        return Err(PulseApiError::InvalidPath);
    }
    if trimmed.chars().any(|ch| ch.is_whitespace()) {
        return Err(PulseApiError::InvalidPath);
    }
    Ok(trimmed.to_string())
}

async fn ensure_valid_path(state: &AppState, path: &str) -> Result<(), PulseApiError> {
    let valid_paths = state.valid_paths.read().await;
    if valid_paths.is_empty() {
        return Err(PulseApiError::ValidPathsUnavailable);
    }
    if !valid_paths.contains(path) {
        return Err(PulseApiError::PathNotAllowed);
    }
    Ok(())
}

fn header_value<'a>(headers: &'a HeaderMap, name: &str) -> Option<&'a str> {
    headers
        .get(name)
        .and_then(|value| value.to_str().ok())
        .map(|value| value.trim())
        .filter(|value| !value.is_empty())
}

fn parse_ua_family(ua: &str) -> Option<String> {
    let ua = ua.trim();
    if ua.is_empty() {
        return None;
    }
    if ua.contains("Firefox/") {
        return Some("Firefox".to_string());
    }
    if ua.contains("Edg/") {
        return Some("Edge".to_string());
    }
    if ua.contains("Chrome/") {
        return Some("Chrome".to_string());
    }
    if ua.contains("Safari/") {
        return Some("Safari".to_string());
    }
    if ua.contains("curl/") {
        return Some("curl".to_string());
    }
    let first = ua.split_whitespace().next().unwrap_or(ua);
    let family = first.split('/').next().unwrap_or(first).trim();
    if family.is_empty() {
        None
    } else {
        Some(family.to_string())
    }
}

fn parse_device(ua: &str) -> Option<String> {
    let ua = ua.to_ascii_lowercase();
    if ua.is_empty() {
        return None;
    }
    if ua.contains("ipad") || ua.contains("tablet") {
        return Some("tablet".to_string());
    }
    if ua.contains("mobile") || ua.contains("iphone") || ua.contains("android") {
        return Some("mobile".to_string());
    }
    if ua.contains("bot") || ua.contains("spider") || ua.contains("crawler") {
        return Some("bot".to_string());
    }
    Some("desktop".to_string())
}

fn parse_ref_host(referer: &str) -> Option<String> {
    let trimmed = referer.trim();
    if trimmed.is_empty() {
        return None;
    }
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host_port = without_scheme.split('/').next().unwrap_or(without_scheme);
    let host_port = host_port.split('@').last().unwrap_or(host_port);
    let host = host_port.split(':').next().unwrap_or(host_port).trim();
    if host.is_empty() {
        None
    } else {
        Some(host.to_string())
    }
}

fn extract_country(headers: &HeaderMap) -> Option<String> {
    if let Some(value) = header_value(headers, "cf-ipcountry") {
        if !value.eq_ignore_ascii_case("xx") {
            return Some(value.to_string());
        }
    }
    if let Some(value) = header_value(headers, "x-forwarded-for") {
        let first = value.split(',').next().unwrap_or(value).trim();
        if !first.is_empty() {
            return Some(first.to_string());
        }
    }
    None
}

impl IntoResponse for PulseApiError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            PulseApiError::MissingPageInstanceId
            | PulseApiError::InvalidPageInstanceId
            | PulseApiError::MissingPath
            | PulseApiError::InvalidPath
            | PulseApiError::InvalidDuration
            | PulseApiError::InvalidPayload => (StatusCode::BAD_REQUEST, self.to_string()),
            PulseApiError::PathNotAllowed => (StatusCode::NOT_FOUND, self.to_string()),
            PulseApiError::ValidPathsUnavailable | PulseApiError::DbUnavailable => {
                (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
            }
            PulseApiError::Db(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{extract_country, normalize_path, parse_ref_host};
    use axum::http::HeaderMap;

    #[test]
    fn normalize_path_rejects_whitespace() {
        assert!(normalize_path(Some("/posts/hello world")).is_err());
    }

    #[test]
    fn parse_ref_host_extracts_host() {
        let host = parse_ref_host("https://example.com/path").unwrap();
        assert_eq!(host, "example.com");
    }

    #[test]
    fn extract_country_uses_cf_header() {
        let mut headers = HeaderMap::new();
        headers.insert("cf-ipcountry", "JP".parse().unwrap());
        let value = extract_country(&headers).unwrap();
        assert_eq!(value, "JP");
    }
}
