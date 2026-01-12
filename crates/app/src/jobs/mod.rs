pub mod scheduler;
pub mod tasks;

use std::time::Duration;

use thiserror::Error;
use tracing::{info, warn};

use crate::mem_probe;
use crate::state::AppState;

#[derive(Debug, Error)]
pub enum JobError {
    #[error("http error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("feed parse error: {0}")]
    Feed(String),
    #[error("search index error: {0}")]
    Search(#[from] inkstone_infra::search::SearchIndexError),
    #[error("db error: {0}")]
    Db(#[from] inkstone_infra::db::DoubanRepoError),
    #[error("kudos db error: {0}")]
    KudosDb(#[from] inkstone_infra::db::KudosRepoError),
    #[error("comments db error: {0}")]
    CommentsDb(#[from] inkstone_infra::db::CommentsRepoError),
    #[error("github error: {0}")]
    Github(#[from] inkstone_infra::github::GithubError),
    #[error("comments error: {0}")]
    Comments(String),
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

const HEAP_DUMP_INTERVAL: Duration = Duration::from_secs(10 * 60);

pub async fn start(state: AppState, rebuild: bool) -> Result<(), JobError> {
    if rebuild {
        info!("running content refresh rebuild before scheduler");
        mem_probe::record("content_refresh_rebuild", "before");
        let stats = tasks::content_refresh::run(&state, true, true).await?;
        mem_probe::record("content_refresh_rebuild", "after");
        info!(?stats, "content refresh rebuild complete");
        info!("running douban crawl rebuild before scheduler");
        mem_probe::record("douban_crawl_rebuild", "before");
        tasks::douban_crawl::run(&state, true).await?;
        mem_probe::record("douban_crawl_rebuild", "after");
        if state.db.is_some() && tasks::comments_sync::is_enabled(&state.config) {
            mem_probe::record("comments_sync_rebuild", "before");
            match tasks::comments_sync::run(&state, true).await {
                Ok(stats) => info!(?stats, "comments sync rebuild complete"),
                Err(err) => warn!(error = %err, "comments sync rebuild failed"),
            }
            mem_probe::record("comments_sync_rebuild", "after");
        }
    }

    let refresh_interval = state.config.poll_interval;
    let refresh_state = state.clone();
    let refresh_job = scheduler::run_interval("content_refresh", refresh_interval, move || {
        let state = refresh_state.clone();
        async move {
            mem_probe::record("content_refresh", "before");
            match tasks::content_refresh::run(&state, false, false).await {
                Ok(stats) => info!(?stats, "content refresh run complete"),
                Err(err) => warn!(error = %err, "content refresh run failed"),
            }
            mem_probe::record("content_refresh", "after");
            Ok(())
        }
    });

    let douban_interval = state.config.douban_poll_interval;
    let douban_state = state.clone();
    let douban_job = scheduler::run_interval("douban_crawl", douban_interval, move || {
        let state = douban_state.clone();
        async move {
            mem_probe::record("douban_crawl", "before");
            if let Err(err) = tasks::douban_crawl::run(&state, false).await {
                warn!(error = %err, "douban crawl failed");
            }
            mem_probe::record("douban_crawl", "after");
            Ok(())
        }
    });

    if state.db.is_some() {
        if let Err(err) = tasks::kudos_cache::load(&state).await {
            warn!(error = %err, "kudos cache load failed");
        }
    } else {
        warn!("db not configured; skipping kudos cache load/flush");
    }

    let kudos_interval = state.config.kudos_flush_interval;
    let comments_interval = state.config.comments_sync_interval;
    let comments_job = if state.db.is_some()
        && comments_interval.as_secs() > 0
        && tasks::comments_sync::is_enabled(&state.config)
    {
        let comments_state = state.clone();
        Some(scheduler::run_interval(
            "comments_sync",
            comments_interval,
            move || {
                let state = comments_state.clone();
                async move {
                    mem_probe::record("comments_sync", "before");
                    match tasks::comments_sync::run(&state, false).await {
                        Ok(stats) => info!(?stats, "comments sync complete"),
                        Err(err) => warn!(error = %err, "comments sync failed"),
                    }
                    mem_probe::record("comments_sync", "after");
                    Ok(())
                }
            },
        ))
    } else {
        None
    };

    if state.db.is_some() && kudos_interval.as_secs() > 0 {
        let kudos_state = state.clone();
        let kudos_job = scheduler::run_interval("kudos_cache_flush", kudos_interval, move || {
            let state = kudos_state.clone();
            async move {
                mem_probe::record("kudos_cache_flush", "before");
                if let Err(err) = tasks::kudos_cache::flush(&state).await {
                    warn!(error = %err, "kudos cache flush failed");
                }
                mem_probe::record("kudos_cache_flush", "after");
                Ok(())
            }
        });
        let heap_job = scheduler::run_interval("heap_dump", HEAP_DUMP_INTERVAL, move || async {
            mem_probe::record("heap_dump", "interval");
            Ok(())
        });
        match comments_job {
            Some(comments_job) => {
                tokio::try_join!(refresh_job, douban_job, kudos_job, comments_job, heap_job)?;
            }
            None => {
                tokio::try_join!(refresh_job, douban_job, kudos_job, heap_job)?;
            }
        }
    } else {
        let heap_job = scheduler::run_interval("heap_dump", HEAP_DUMP_INTERVAL, move || async {
            mem_probe::record("heap_dump", "interval");
            Ok(())
        });
        match comments_job {
            Some(comments_job) => {
                tokio::try_join!(refresh_job, douban_job, comments_job, heap_job)?;
            }
            None => {
                tokio::try_join!(refresh_job, douban_job, heap_job)?;
            }
        }
    }
    Ok(())
}
