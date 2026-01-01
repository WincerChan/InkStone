use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;
use thiserror::Error;

use crate::jobs::tasks::kudos_cache;
use crate::jobs::JobError;
use crate::state::AppState;
use inkstone_infra::db::{fetch_kudos_overview, KudosRepoError};

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
