pub mod scheduler;
pub mod tasks;

use thiserror::Error;
use tracing::{info, warn};

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
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub async fn start(state: AppState, rebuild: bool) -> Result<(), JobError> {
    if rebuild {
        info!("running feed index rebuild before scheduler");
        let stats = tasks::feed_index::run(&state, true).await?;
        info!(?stats, "rebuild complete");
        info!("running douban crawl rebuild before scheduler");
        tasks::douban_crawl::run(&state, true).await?;
    }

    let feed_interval = state.config.poll_interval;
    let feed_state = state.clone();
    let feed_job = scheduler::run_interval("feed_index", feed_interval, move || {
        let state = feed_state.clone();
        async move {
            match tasks::feed_index::run(&state, false).await {
                Ok(stats) => info!(?stats, "feed index run complete"),
                Err(err) => warn!(error = %err, "feed index run failed"),
            }
            Ok(())
        }
    });

    let douban_interval = state.config.douban_poll_interval;
    let douban_state = state.clone();
    let douban_job = scheduler::run_interval("douban_crawl", douban_interval, move || {
        let state = douban_state.clone();
        async move {
            if let Err(err) = tasks::douban_crawl::run(&state, false).await {
                warn!(error = %err, "douban crawl failed");
            }
            Ok(())
        }
    });

    if let Err(err) = tasks::valid_paths_refresh::run(&state).await {
        warn!(error = %err, "valid paths refresh failed");
    }

    if state.db.is_some() {
        if let Err(err) = tasks::kudos_cache::load(&state).await {
            warn!(error = %err, "kudos cache load failed");
        }
    } else {
        warn!("db not configured; skipping kudos cache load/flush");
    }

    let paths_interval = state.config.valid_paths_refresh_interval;
    let paths_state = state.clone();
    let paths_job = scheduler::run_interval("valid_paths_refresh", paths_interval, move || {
        let state = paths_state.clone();
        async move {
            if let Err(err) = tasks::valid_paths_refresh::run(&state).await {
                warn!(error = %err, "valid paths refresh failed");
            }
            Ok(())
        }
    });

    let kudos_interval = state.config.kudos_flush_interval;
    if state.db.is_some() && kudos_interval.as_secs() > 0 {
        let kudos_state = state.clone();
        let kudos_job = scheduler::run_interval("kudos_cache_flush", kudos_interval, move || {
            let state = kudos_state.clone();
            async move {
                if let Err(err) = tasks::kudos_cache::flush(&state).await {
                    warn!(error = %err, "kudos cache flush failed");
                }
                Ok(())
            }
        });
        tokio::try_join!(feed_job, douban_job, paths_job, kudos_job)?;
    } else {
        tokio::try_join!(feed_job, douban_job, paths_job)?;
    }
    Ok(())
}
