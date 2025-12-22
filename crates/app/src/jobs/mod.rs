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
    #[error("io error: {0}")]
    Io(#[from] std::io::Error),
}

pub async fn start(state: AppState, rebuild: bool) -> Result<(), JobError> {
    if rebuild {
        info!("running feed index rebuild before scheduler");
        let stats = tasks::feed_index::run(&state, true).await?;
        info!(?stats, "rebuild complete");
    }

    let interval = state.config.poll_interval;
    let feed_state = state.clone();
    let feed_job = scheduler::run_interval("feed_index", interval, move || {
        let state = feed_state.clone();
        async move {
            match tasks::feed_index::run(&state, false).await {
                Ok(stats) => info!(?stats, "feed index run complete"),
                Err(err) => warn!(error = %err, "feed index run failed"),
            }
            Ok(())
        }
    });

    let douban_state = state.clone();
    let douban_job = scheduler::run_interval("douban_crawl", interval, move || {
        let state = douban_state.clone();
        async move {
            if let Err(err) = tasks::douban_crawl::run(&state).await {
                warn!(error = %err, "douban crawl failed");
            }
            Ok(())
        }
    });

    tokio::try_join!(feed_job, douban_job)?;
    Ok(())
}
