use std::time::{Duration, Instant};

use chrono::Utc;
use tracing::{debug, warn};

use crate::jobs::JobError;
use crate::state::AppState;
use crate::jobs::tasks::{comments_sync, feed_index, valid_paths_refresh};
use crate::jobs::tasks::feed_index::JobStats;

const FEED_BACKOFF: Duration = Duration::from_secs(60);
const PATHS_BACKOFF: Duration = Duration::from_secs(60);

pub async fn run(state: &AppState, rebuild: bool, force: bool) -> Result<JobStats, JobError> {
    {
        let mut health = state.admin_health.lock().await;
        health.content_refresh_last_run = Some(Utc::now());
    }
    let now = Instant::now();
    let paths_backoff = if force || rebuild {
        None
    } else {
        backoff_remaining(state, now, RefreshTask::Paths).await
    };
    if paths_backoff.is_none() {
        match valid_paths_refresh::run(state).await {
            Ok(()) => clear_backoff(state, RefreshTask::Paths).await,
            Err(err) => {
                warn!(error = %err, "valid paths refresh failed");
                set_backoff(state, now, RefreshTask::Paths).await;
            }
        }
    } else if let Some(remaining) = paths_backoff {
        debug!(
            remaining_secs = remaining.as_secs(),
            "valid paths refresh skipped due to backoff"
        );
    }

    let feed_backoff = if force || rebuild {
        None
    } else {
        backoff_remaining(state, now, RefreshTask::Feed).await
    };
    let stats = if feed_backoff.is_none() {
        match feed_index::run(state, rebuild).await {
            Ok(stats) => {
                clear_backoff(state, RefreshTask::Feed).await;
                stats
            }
            Err(err) => {
                warn!(error = %err, "feed index run failed");
                set_backoff(state, now, RefreshTask::Feed).await;
                JobStats {
                    fetched: 0,
                    indexed: 0,
                    skipped: 0,
                    failed: 1,
                }
            }
        }
    } else if let Some(remaining) = feed_backoff {
        debug!(
            remaining_secs = remaining.as_secs(),
            "feed index skipped due to backoff"
        );
        JobStats {
            fetched: 0,
            indexed: 0,
            skipped: 0,
            failed: 0,
        }
    } else {
        JobStats {
            fetched: 0,
            indexed: 0,
            skipped: 0,
            failed: 0,
        }
    };

    if force && state.db.is_some() && comments_sync::is_enabled(&state.config) {
        match comments_sync::run(state, false).await {
            Ok(stats) => debug!(?stats, "comments sync triggered by content refresh"),
            Err(err) => warn!(error = %err, "comments sync triggered by content refresh failed"),
        }
    }

    {
        let mut health = state.admin_health.lock().await;
        health.content_refresh_last_success = Some(Utc::now());
    }
    Ok(stats)
}

#[derive(Clone, Copy)]
enum RefreshTask {
    Feed,
    Paths,
}

async fn backoff_remaining(
    state: &AppState,
    now: Instant,
    task: RefreshTask,
) -> Option<Duration> {
    let guard = state.content_refresh_backoff.lock().await;
    let next = match task {
        RefreshTask::Feed => guard.next_feed_at,
        RefreshTask::Paths => guard.next_paths_at,
    }?;
    if next > now {
        Some(next.duration_since(now))
    } else {
        None
    }
}

async fn set_backoff(state: &AppState, now: Instant, task: RefreshTask) {
    let mut guard = state.content_refresh_backoff.lock().await;
    let next = match task {
        RefreshTask::Feed => now + FEED_BACKOFF,
        RefreshTask::Paths => now + PATHS_BACKOFF,
    };
    match task {
        RefreshTask::Feed => guard.next_feed_at = Some(next),
        RefreshTask::Paths => guard.next_paths_at = Some(next),
    }
}

async fn clear_backoff(state: &AppState, task: RefreshTask) {
    let mut guard = state.content_refresh_backoff.lock().await;
    match task {
        RefreshTask::Feed => guard.next_feed_at = None,
        RefreshTask::Paths => guard.next_paths_at = None,
    }
}
