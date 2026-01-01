use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use chrono::{DateTime, Duration, NaiveDate, Utc};
use serde::{Deserialize, Serialize};
use thiserror::Error;

use crate::state::AppState;
use inkstone_infra::db::{
    fetch_active_country_counts, fetch_active_device_counts, fetch_active_ref_host_counts,
    fetch_active_source_counts, fetch_active_top_paths, fetch_active_totals, fetch_active_ua_counts,
    fetch_country_stats, fetch_daily, fetch_device_stats, fetch_ref_host_stats, fetch_source_stats,
    fetch_totals, fetch_top_paths, fetch_ua_stats, list_sites, AnalyticsRepoError, PulseDailyStat,
    PulseDimCount, PulseDimStats, PulseSiteOverview, PulseTopPath, PulseTotals,
};

const DEFAULT_RANGE_DAYS: i64 = 30;
const DEFAULT_ACTIVE_MINUTES: i64 = 5;
const DEFAULT_TOP_LIMIT: i64 = 20;
const MAX_TOP_LIMIT: i64 = 200;
const MAX_SITE_LEN: usize = 255;

#[derive(Debug, Deserialize)]
pub struct PulseSiteQuery {
    pub site: Option<String>,
    pub from: Option<String>,
    pub to: Option<String>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct PulseActiveQuery {
    pub site: Option<String>,
    pub minutes: Option<i64>,
    pub limit: Option<i64>,
}

#[derive(Debug, Deserialize)]
pub struct PulseActiveSummaryQuery {
    pub site: Option<String>,
    pub minutes: Option<i64>,
}

#[derive(Debug, Error)]
pub enum PulseAdminError {
    #[error("site is required")]
    MissingSite,
    #[error("site is invalid")]
    InvalidSite,
    #[error("active window is invalid")]
    InvalidWindow,
    #[error("invalid date")]
    InvalidDate,
    #[error("invalid date range")]
    InvalidDateRange,
    #[error("db not configured")]
    DbUnavailable,
    #[error("db error: {0}")]
    Db(#[from] AnalyticsRepoError),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Serialize)]
pub struct PulseSitesResponse {
    total: usize,
    items: Vec<PulseSiteEntry>,
}

#[derive(Debug, Serialize)]
pub struct PulseSiteEntry {
    site: String,
    pv: i64,
    uv: i64,
    last_seen_at: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct PulseSiteStatsResponse {
    site: String,
    range: PulseRange,
    summary: PulseSummary,
    daily: Vec<PulseDailyEntry>,
    top_paths: Vec<PulseTopPathEntry>,
    devices: Vec<PulseDimStatsEntry>,
    ua_families: Vec<PulseDimStatsEntry>,
    source_types: Vec<PulseDimStatsEntry>,
    ref_hosts: Vec<PulseDimStatsEntry>,
    countries: Vec<PulseDimStatsEntry>,
}

#[derive(Debug, Serialize)]
pub struct PulseActiveResponse {
    site: String,
    range: PulseActiveRange,
    active_pv: i64,
    active_uv: i64,
    top_paths: Vec<PulseTopPathEntry>,
    devices: Vec<PulseDimEntry>,
    ua_families: Vec<PulseDimEntry>,
    source_types: Vec<PulseDimEntry>,
    ref_hosts: Vec<PulseDimEntry>,
    countries: Vec<PulseDimEntry>,
}

#[derive(Debug, Serialize)]
pub struct PulseActiveSummaryResponse {
    site: String,
    range: PulseActiveRange,
    active_pv: i64,
    active_uv: i64,
}

#[derive(Debug, Serialize)]
pub struct PulseRange {
    from: String,
    to: String,
}

#[derive(Debug, Serialize)]
pub struct PulseActiveRange {
    from: String,
    to: String,
}

#[derive(Debug, Serialize)]
pub struct PulseSummary {
    pv: i64,
    uv: i64,
    avg_duration_ms: Option<i64>,
    total_duration_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct PulseDailyEntry {
    day: String,
    pv: i64,
    uv: i64,
    avg_duration_ms: Option<i64>,
    total_duration_ms: i64,
}

#[derive(Debug, Serialize)]
pub struct PulseTopPathEntry {
    path: String,
    pv: i64,
    uv: i64,
}

#[derive(Debug, Serialize)]
pub struct PulseDimEntry {
    value: String,
    uv: i64,
}

#[derive(Debug, Serialize)]
pub struct PulseDimStatsEntry {
    value: String,
    pv: i64,
    uv: i64,
}

pub async fn list_pulse_sites(
    State(state): State<AppState>,
) -> Result<Json<PulseSitesResponse>, PulseAdminError> {
    let pool = state.db.as_ref().ok_or(PulseAdminError::DbUnavailable)?;
    let sites = list_sites(pool).await?;
    let items = sites.into_iter().map(map_site_entry).collect::<Vec<_>>();
    Ok(Json(PulseSitesResponse {
        total: items.len(),
        items,
    }))
}

pub async fn get_pulse_site(
    State(state): State<AppState>,
    Query(query): Query<PulseSiteQuery>,
) -> Result<Json<PulseSiteStatsResponse>, PulseAdminError> {
    let site = normalize_site_param(query.site.as_deref())?;
    let (from, to) = parse_range(query.from.as_deref(), query.to.as_deref())?;
    let limit = clamp_limit(query.limit);
    let pool = state.db.as_ref().ok_or(PulseAdminError::DbUnavailable)?.clone();

    let (totals, daily, top_paths, devices, ua_families, source_types, ref_hosts, countries) =
        tokio::try_join!(
            fetch_totals(&pool, &site, from, to),
            fetch_daily(&pool, &site, from, to),
            fetch_top_paths(&pool, &site, from, to, limit),
            fetch_device_stats(&pool, &site, from, to, limit),
            fetch_ua_stats(&pool, &site, from, to, limit),
            fetch_source_stats(&pool, &site, from, to, limit),
            fetch_ref_host_stats(&pool, &site, from, to, limit),
            fetch_country_stats(&pool, &site, from, to, limit),
        )?;

    Ok(Json(PulseSiteStatsResponse {
        site,
        range: PulseRange {
            from: from.to_string(),
            to: to.to_string(),
        },
        summary: map_summary(totals),
        daily: daily.into_iter().map(map_daily_entry).collect(),
        top_paths: top_paths.into_iter().map(map_top_path).collect(),
        devices: devices.into_iter().map(map_dim_stats_entry).collect(),
        ua_families: ua_families.into_iter().map(map_dim_stats_entry).collect(),
        source_types: source_types.into_iter().map(map_dim_stats_entry).collect(),
        ref_hosts: ref_hosts.into_iter().map(map_dim_stats_entry).collect(),
        countries: countries.into_iter().map(map_dim_stats_entry).collect(),
    }))
}

pub async fn get_pulse_active(
    State(state): State<AppState>,
    Query(query): Query<PulseActiveQuery>,
) -> Result<Json<PulseActiveResponse>, PulseAdminError> {
    let site = normalize_site_param(query.site.as_deref())?;
    let minutes = parse_active_minutes(query.minutes)?;
    let limit = clamp_limit(query.limit);
    let pool = state.db.as_ref().ok_or(PulseAdminError::DbUnavailable)?.clone();
    let (from, to) = active_range(minutes);
    let (totals, top_paths, devices, ua_families, source_types, ref_hosts, countries) =
        tokio::try_join!(
            fetch_active_totals(&pool, &site, from, to),
            fetch_active_top_paths(&pool, &site, from, to, limit),
            fetch_active_device_counts(&pool, &site, from, to, limit),
            fetch_active_ua_counts(&pool, &site, from, to, limit),
            fetch_active_source_counts(&pool, &site, from, to, limit),
            fetch_active_ref_host_counts(&pool, &site, from, to, limit),
            fetch_active_country_counts(&pool, &site, from, to, limit),
        )?;

    Ok(Json(PulseActiveResponse {
        site,
        range: PulseActiveRange {
            from: from.to_rfc3339(),
            to: to.to_rfc3339(),
        },
        active_pv: totals.pv,
        active_uv: totals.uv,
        top_paths: top_paths.into_iter().map(map_top_path).collect(),
        devices: devices.into_iter().map(map_dim_entry).collect(),
        ua_families: ua_families.into_iter().map(map_dim_entry).collect(),
        source_types: source_types.into_iter().map(map_dim_entry).collect(),
        ref_hosts: ref_hosts.into_iter().map(map_dim_entry).collect(),
        countries: countries.into_iter().map(map_dim_entry).collect(),
    }))
}

pub async fn get_pulse_active_summary(
    State(state): State<AppState>,
    Query(query): Query<PulseActiveSummaryQuery>,
) -> Result<Json<PulseActiveSummaryResponse>, PulseAdminError> {
    let site = normalize_site_param(query.site.as_deref())?;
    let minutes = parse_active_minutes(query.minutes)?;
    let pool = state.db.as_ref().ok_or(PulseAdminError::DbUnavailable)?.clone();
    let (from, to) = active_range(minutes);
    let totals = fetch_active_totals(&pool, &site, from, to).await?;

    Ok(Json(PulseActiveSummaryResponse {
        site,
        range: PulseActiveRange {
            from: from.to_rfc3339(),
            to: to.to_rfc3339(),
        },
        active_pv: totals.pv,
        active_uv: totals.uv,
    }))
}
fn map_site_entry(entry: PulseSiteOverview) -> PulseSiteEntry {
    PulseSiteEntry {
        site: entry.site,
        pv: entry.pv,
        uv: entry.uv,
        last_seen_at: entry.last_seen_at.map(|value| value.to_rfc3339()),
    }
}

fn map_summary(totals: PulseTotals) -> PulseSummary {
    PulseSummary {
        pv: totals.pv,
        uv: totals.uv,
        avg_duration_ms: round_duration(totals.avg_duration_ms),
        total_duration_ms: totals.total_duration_ms,
    }
}

fn map_daily_entry(entry: PulseDailyStat) -> PulseDailyEntry {
    PulseDailyEntry {
        day: entry.day.to_string(),
        pv: entry.pv,
        uv: entry.uv,
        avg_duration_ms: round_duration(entry.avg_duration_ms),
        total_duration_ms: entry.total_duration_ms,
    }
}

fn map_top_path(entry: PulseTopPath) -> PulseTopPathEntry {
    PulseTopPathEntry {
        path: entry.path,
        pv: entry.pv,
        uv: entry.uv,
    }
}

fn map_dim_entry(entry: PulseDimCount) -> PulseDimEntry {
    PulseDimEntry {
        value: entry.value,
        uv: entry.count,
    }
}

fn map_dim_stats_entry(entry: PulseDimStats) -> PulseDimStatsEntry {
    PulseDimStatsEntry {
        value: entry.value,
        pv: entry.pv,
        uv: entry.uv,
    }
}

fn active_range(minutes: i64) -> (DateTime<Utc>, DateTime<Utc>) {
    let to = Utc::now();
    let from = to - Duration::minutes(minutes);
    (from, to)
}

fn parse_active_minutes(value: Option<i64>) -> Result<i64, PulseAdminError> {
    let minutes = value.unwrap_or(DEFAULT_ACTIVE_MINUTES);
    if minutes <= 0 {
        return Err(PulseAdminError::InvalidWindow);
    }
    Ok(minutes)
}

fn round_duration(value: Option<f64>) -> Option<i64> {
    value.map(|duration| duration.round() as i64)
}

fn parse_range(from: Option<&str>, to: Option<&str>) -> Result<(NaiveDate, NaiveDate), PulseAdminError> {
    let today = Utc::now().date_naive();
    let default_from = today - Duration::days(DEFAULT_RANGE_DAYS);
    let from = from
        .map(parse_date)
        .transpose()?
        .unwrap_or(default_from);
    let to = to.map(parse_date).transpose()?.unwrap_or(today);
    if from > to {
        return Err(PulseAdminError::InvalidDateRange);
    }
    Ok((from, to))
}

fn parse_date(raw: &str) -> Result<NaiveDate, PulseAdminError> {
    NaiveDate::parse_from_str(raw.trim(), "%Y-%m-%d").map_err(|_| PulseAdminError::InvalidDate)
}

fn normalize_site_param(value: Option<&str>) -> Result<String, PulseAdminError> {
    let raw = value.unwrap_or("").trim();
    if raw.is_empty() {
        return Err(PulseAdminError::MissingSite);
    }
    let normalized = normalize_host_value(raw)?;
    if normalized.len() > MAX_SITE_LEN {
        return Err(PulseAdminError::InvalidSite);
    }
    Ok(normalized)
}

fn normalize_host_value(value: &str) -> Result<String, PulseAdminError> {
    let trimmed = value.trim();
    if trimmed.is_empty() {
        return Err(PulseAdminError::InvalidSite);
    }
    let without_scheme = trimmed
        .strip_prefix("https://")
        .or_else(|| trimmed.strip_prefix("http://"))
        .unwrap_or(trimmed);
    let host_list = without_scheme.split(',').next().unwrap_or(without_scheme);
    let host_port = host_list.split('@').last().unwrap_or(host_list);
    let host = host_port.split(':').next().unwrap_or(host_port).trim();
    if host.is_empty() || host.chars().any(|ch| ch.is_whitespace()) {
        return Err(PulseAdminError::InvalidSite);
    }
    Ok(host.trim_end_matches('.').to_ascii_lowercase())
}

fn clamp_limit(limit: Option<i64>) -> i64 {
    match limit {
        Some(value) if value > 0 => value.min(MAX_TOP_LIMIT),
        _ => DEFAULT_TOP_LIMIT,
    }
}

impl IntoResponse for PulseAdminError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            PulseAdminError::MissingSite
            | PulseAdminError::InvalidSite
            | PulseAdminError::InvalidWindow
            | PulseAdminError::InvalidDate
            | PulseAdminError::InvalidDateRange => (StatusCode::BAD_REQUEST, self.to_string()),
            PulseAdminError::DbUnavailable => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            PulseAdminError::Db(_) => (StatusCode::INTERNAL_SERVER_ERROR, self.to_string()),
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}

#[cfg(test)]
mod tests {
    use super::{normalize_site_param, parse_active_minutes, parse_date, parse_range};

    #[test]
    fn parse_date_rejects_invalid() {
        assert!(parse_date("2025-99-99").is_err());
    }

    #[test]
    fn parse_range_defaults_to_today() {
        let (from, to) = parse_range(None, None).unwrap();
        assert!(from <= to);
    }

    #[test]
    fn normalize_site_param_strips_scheme() {
        let site = normalize_site_param(Some("https://Blog.Itswincer.com:443")).unwrap();
        assert_eq!(site, "blog.itswincer.com");
    }

    #[test]
    fn parse_active_minutes_defaults_to_five() {
        assert_eq!(parse_active_minutes(None).unwrap(), 5);
    }

    #[test]
    fn parse_active_minutes_rejects_zero() {
        assert!(parse_active_minutes(Some(0)).is_err());
    }
}
