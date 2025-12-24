use tracing::warn;

use crate::jobs::JobError;
use crate::state::AppState;
use crate::jobs::tasks::{feed_index, valid_paths_refresh};
use crate::jobs::tasks::feed_index::JobStats;

pub async fn run(state: &AppState, rebuild: bool) -> Result<JobStats, JobError> {
    let valid_paths_result = valid_paths_refresh::run(state).await;
    let stats = feed_index::run(state, rebuild).await?;
    if let Err(err) = valid_paths_result {
        warn!(error = %err, "valid paths refresh failed");
    }
    Ok(stats)
}
