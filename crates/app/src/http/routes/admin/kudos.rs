use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;
use thiserror::Error;

use crate::jobs::tasks::kudos_cache;
use crate::jobs::JobError;
use crate::state::AppState;
use inkstone_infra::db::{fetch_kudos_overview, fetch_kudos_top_paths, KudosRepoError};

const DEFAULT_TOP_LIMIT: i64 = 20;
const MAX_TOP_LIMIT: i64 = 200;

#[derive(Debug, serde::Deserialize)]
pub struct KudosTopQuery {
    pub limit: Option<i64>,
}

#[derive(Debug, Error)]
pub enum KudosAdminError {
    #[error("db not configured")]
    DbUnavailable,
    #[error("db error: {0}")]
    Db(#[from] KudosRepoError),
    #[error("job error: {0}")]
    Job(#[from] JobError),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Serialize)]
pub struct KudosStatusResponse {
    cache: KudosCacheStatus,
    database: KudosDbStatus,
}

#[derive(Debug, Serialize)]
pub struct KudosActionResponse {
    action: &'static str,
    cache: KudosCacheStatus,
    database: KudosDbStatus,
}

#[derive(Debug, Serialize)]
pub struct KudosCacheStatus {
    paths: i64,
    total: i64,
    pending: i64,
}

#[derive(Debug, Serialize)]
pub struct KudosDbStatus {
    paths: i64,
    total: i64,
}

#[derive(Debug, Serialize)]
pub struct KudosTopPathsResponse {
    total: usize,
    items: Vec<KudosTopPathEntry>,
}

#[derive(Debug, Serialize)]
pub struct KudosTopPathEntry {
    path: String,
    count: i64,
}

pub async fn get_kudos_status(
    State(state): State<AppState>,
) -> Result<Json<KudosStatusResponse>, KudosAdminError> {
    let (cache, database) = load_status(&state).await?;
    Ok(Json(KudosStatusResponse { cache, database }))
}

pub async fn post_kudos_flush(
    State(state): State<AppState>,
) -> Result<Json<KudosActionResponse>, KudosAdminError> {
    ensure_db(&state)?;
    kudos_cache::flush(&state).await?;
    let (cache, database) = load_status(&state).await?;
    Ok(Json(KudosActionResponse {
        action: "flush",
        cache,
        database,
    }))
}

pub async fn post_kudos_reload(
    State(state): State<AppState>,
) -> Result<Json<KudosActionResponse>, KudosAdminError> {
    ensure_db(&state)?;
    kudos_cache::load(&state).await?;
    let (cache, database) = load_status(&state).await?;
    Ok(Json(KudosActionResponse {
        action: "reload",
        cache,
        database,
    }))
}

pub async fn get_kudos_top_paths(
    State(state): State<AppState>,
    Query(query): Query<KudosTopQuery>,
) -> Result<Json<KudosTopPathsResponse>, KudosAdminError> {
    ensure_db(&state)?;
    let limit = clamp_limit(query.limit);
    let pool = state.db.as_ref().ok_or(KudosAdminError::DbUnavailable)?;
    let rows = fetch_kudos_top_paths(pool, limit).await?;
    let items = rows
        .into_iter()
        .map(|row| KudosTopPathEntry {
            path: row.path,
            count: row.count,
        })
        .collect::<Vec<_>>();
    Ok(Json(KudosTopPathsResponse {
        total: items.len(),
        items,
    }))
}

impl IntoResponse for KudosAdminError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            KudosAdminError::DbUnavailable => (StatusCode::SERVICE_UNAVAILABLE, self.to_string()),
            KudosAdminError::Db(_) | KudosAdminError::Job(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}

fn ensure_db(state: &AppState) -> Result<(), KudosAdminError> {
    if state.db.is_none() {
        return Err(KudosAdminError::DbUnavailable);
    }
    Ok(())
}

fn clamp_limit(limit: Option<i64>) -> i64 {
    match limit {
        Some(value) if value > 0 => value.min(MAX_TOP_LIMIT),
        _ => DEFAULT_TOP_LIMIT,
    }
}

async fn load_status(
    state: &AppState,
) -> Result<(KudosCacheStatus, KudosDbStatus), KudosAdminError> {
    let pool = state.db.as_ref().ok_or(KudosAdminError::DbUnavailable)?;
    let overview = fetch_kudos_overview(pool).await?;
    let cache = {
        let cache = state.kudos_cache.read().await;
        KudosCacheStatus {
            paths: cache.path_count(),
            total: cache.total_count(),
            pending: cache.pending_count(),
        }
    };
    let database = KudosDbStatus {
        paths: overview.paths,
        total: overview.total,
    };
    Ok((cache, database))
}

#[cfg(test)]
mod tests {
    use super::clamp_limit;

    #[test]
    fn clamp_limit_applies_defaults_and_cap() {
        assert_eq!(clamp_limit(None), 20);
        assert_eq!(clamp_limit(Some(0)), 20);
        assert_eq!(clamp_limit(Some(10)), 10);
        assert_eq!(clamp_limit(Some(500)), 200);
    }
}
