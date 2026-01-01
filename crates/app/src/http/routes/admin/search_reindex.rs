use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use serde::Serialize;
use thiserror::Error;

use crate::jobs::tasks::feed_index;
use crate::jobs::JobError;
use crate::state::AppState;

#[derive(Debug, Error)]
pub enum SearchAdminError {
    #[error("{0}")]
    Job(#[from] JobError),
}

#[derive(Debug, Serialize)]
struct ErrorBody {
    error: String,
}

#[derive(Debug, Serialize)]
pub struct SearchIndexActionResponse {
    action: &'static str,
    stats: SearchJobStats,
}

#[derive(Debug, Serialize)]
pub struct SearchIndexStatusResponse {
    index_dir: String,
    doc_count: u64,
    segment_count: usize,
}

#[derive(Debug, Serialize)]
pub struct SearchJobStats {
    fetched: usize,
    indexed: usize,
    skipped: usize,
    failed: usize,
}

pub async fn post_search_reindex(
    State(state): State<AppState>,
) -> Result<Json<SearchIndexActionResponse>, SearchAdminError> {
    let stats = feed_index::run(&state, true).await?;
    Ok(Json(SearchIndexActionResponse {
        action: "reindex",
        stats: map_job_stats(stats),
    }))
}

pub async fn post_search_refresh(
    State(state): State<AppState>,
) -> Result<Json<SearchIndexActionResponse>, SearchAdminError> {
    let stats = feed_index::run(&state, false).await?;
    Ok(Json(SearchIndexActionResponse {
        action: "refresh",
        stats: map_job_stats(stats),
    }))
}

pub async fn get_search_status(
    State(state): State<AppState>,
) -> Result<Json<SearchIndexStatusResponse>, SearchAdminError> {
    let stats = state.search.stats();
    Ok(Json(SearchIndexStatusResponse {
        index_dir: state.config.index_dir.display().to_string(),
        doc_count: stats.num_docs,
        segment_count: stats.num_segments,
    }))
}

impl IntoResponse for SearchAdminError {
    fn into_response(self) -> axum::response::Response {
        let body = Json(ErrorBody {
            error: self.to_string(),
        });
        (StatusCode::INTERNAL_SERVER_ERROR, body).into_response()
    }
}

fn map_job_stats(stats: feed_index::JobStats) -> SearchJobStats {
    SearchJobStats {
        fetched: stats.fetched,
        indexed: stats.indexed,
        skipped: stats.skipped,
        failed: stats.failed,
    }
}
