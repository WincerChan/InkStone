use std::time::{Duration, Instant};

use chrono::Utc;
use tracing::{debug, warn};

use crate::jobs::JobError;
use crate::state::AppState;
use crate::jobs::tasks::{comments_sync, feed_index};
use crate::jobs::tasks::feed_index::JobStats;

const FEED_BACKOFF: Duration = Duration::from_secs(60);

pub async fn run(state: &AppState, rebuild: bool, force: bool) -> Result<JobStats, JobError> {
    {
        let mut health = state.admin_health.lock().await;
        health.content_refresh_last_run = Some(Utc::now());
    }
    let now = Instant::now();
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
}

async fn backoff_remaining(
    state: &AppState,
    now: Instant,
    task: RefreshTask,
) -> Option<Duration> {
    let guard = state.content_refresh_backoff.lock().await;
    let next = match task {
        RefreshTask::Feed => guard.next_feed_at,
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
    };
    match task {
        RefreshTask::Feed => guard.next_feed_at = Some(next),
    }
}

async fn clear_backoff(state: &AppState, task: RefreshTask) {
    let mut guard = state.content_refresh_backoff.lock().await;
    match task {
        RefreshTask::Feed => guard.next_feed_at = None,
    }
}
