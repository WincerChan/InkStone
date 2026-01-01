use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;
use thiserror::Error;

use crate::jobs::tasks::comments_sync;
use crate::jobs::JobError;
use crate::state::AppState;
use inkstone_infra::db::{fetch_comments_overview, CommentsRepoError};

#[derive(Debug, Error)]
pub enum CommentsAdminError {
    #[error("db not configured")]
    DbUnavailable,
    #[error("comments sync not configured")]
    Disabled,
    #[error("db error: {0}")]
    Db(#[from] CommentsRepoError),
    #[error("job error: {0}")]
    Job(#[from] JobError),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Serialize)]
pub struct CommentsSyncResponse {
    action: &'static str,
    stats: CommentsSyncStatsResponse,
}

#[derive(Debug, Serialize)]
pub struct CommentsSyncStatsResponse {
    posts_seen: usize,
    discussions_created: usize,
    discussions_synced: usize,
    errors: usize,
}

#[derive(Debug, Serialize)]
pub struct CommentsStatusResponse {
    enabled: bool,
    discussions: i64,
    comments: i64,
    last_updated_at: Option<String>,
}

pub async fn post_comments_sync(
    State(state): State<AppState>,
) -> Result<Json<CommentsSyncResponse>, CommentsAdminError> {
    ensure_comments_configured(&state)?;
    let stats = comments_sync::run(&state, false).await?;
    Ok(Json(CommentsSyncResponse {
        action: "sync",
        stats: map_stats(stats),
    }))
}

pub async fn post_comments_rebuild(
    State(state): State<AppState>,
) -> Result<Json<CommentsSyncResponse>, CommentsAdminError> {
    ensure_comments_configured(&state)?;
    let stats = comments_sync::run(&state, true).await?;
    Ok(Json(CommentsSyncResponse {
        action: "rebuild",
        stats: map_stats(stats),
    }))
}

pub async fn get_comments_status(
    State(state): State<AppState>,
) -> Result<Json<CommentsStatusResponse>, CommentsAdminError> {
    let pool = state.db.as_ref().ok_or(CommentsAdminError::DbUnavailable)?;
    let overview = fetch_comments_overview(pool).await?;
    Ok(Json(CommentsStatusResponse {
        enabled: comments_sync::is_enabled(&state.config),
        discussions: overview.discussions,
        comments: overview.comments,
        last_updated_at: format_timestamp(overview.last_updated_at),
    }))
}

impl IntoResponse for CommentsAdminError {
    fn into_response(self) -> axum::response::Response {
        let (status, message) = match &self {
            CommentsAdminError::DbUnavailable | CommentsAdminError::Disabled => {
                (StatusCode::SERVICE_UNAVAILABLE, self.to_string())
            }
            CommentsAdminError::Db(_) | CommentsAdminError::Job(_) => {
                (StatusCode::INTERNAL_SERVER_ERROR, self.to_string())
            }
        };
        let body = Json(ErrorBody { error: message });
        (status, body).into_response()
    }
}

fn ensure_comments_configured(state: &AppState) -> Result<(), CommentsAdminError> {
    if state.db.is_none() {
        return Err(CommentsAdminError::DbUnavailable);
    }
    if !comments_sync::is_enabled(&state.config) {
        return Err(CommentsAdminError::Disabled);
    }
    Ok(())
}

fn map_stats(stats: comments_sync::CommentsSyncStats) -> CommentsSyncStatsResponse {
    CommentsSyncStatsResponse {
        posts_seen: stats.posts_seen,
        discussions_created: stats.discussions_created,
        discussions_synced: stats.discussions_synced,
        errors: stats.errors,
    }
}

fn format_timestamp(value: Option<chrono::DateTime<chrono::Utc>>) -> Option<String> {
    value.map(|timestamp| timestamp.to_rfc3339())
}

#[cfg(test)]
mod tests {
    use super::format_timestamp;
    use chrono::{TimeZone, Utc};

    #[test]
    fn format_timestamp_renders_rfc3339() {
        let ts = Utc.with_ymd_and_hms(2025, 1, 1, 0, 0, 0).unwrap();
        assert_eq!(
            format_timestamp(Some(ts)),
            Some("2025-01-01T00:00:00+00:00".to_string())
        );
    }
}
