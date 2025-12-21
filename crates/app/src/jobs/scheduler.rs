use std::future::Future;
use std::time::Duration;

use tokio::time::{interval, sleep};
use tracing::warn;

use crate::jobs::JobError;

pub async fn run_interval<F, Fut>(
    name: &'static str,
    interval_duration: Duration,
    mut job: F,
) -> Result<(), JobError>
where
    F: FnMut() -> Fut,
    Fut: Future<Output = Result<(), JobError>>,
{
    let mut ticker = interval(interval_duration);
    loop {
        ticker.tick().await;
        if let Err(err) = job().await {
            warn!(error = %err, job = name, "job execution failed");
            sleep(Duration::from_secs(30)).await;
        }
    }
}
