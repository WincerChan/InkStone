use chrono::Utc;
use tracing::{info, warn};

use crate::jobs::JobError;
use crate::state::AppState;
use inkstone_infra::db::{insert_kudos, load_all_kudos};

pub async fn load(state: &AppState) -> Result<(), JobError> {
    let Some(pool) = state.db.as_ref() else {
        warn!("kudos cache load skipped: db not configured");
        return Ok(());
    };
    let entries = load_all_kudos(pool).await?;
    let inserted = {
        let mut cache = state.kudos_cache.write().await;
        cache.load_existing(
            entries
                .into_iter()
                .map(|entry| (entry.path, entry.interaction_id)),
        )
    };
    info!(inserted, "kudos cache loaded");
    Ok(())
}

pub async fn flush(state: &AppState) -> Result<(), JobError> {
    {
        let mut health = state.admin_health.lock().await;
        health.kudos_flush_last_run = Some(Utc::now());
    }
    let Some(pool) = state.db.as_ref() else {
        warn!("kudos cache flush skipped: db not configured");
        return Ok(());
    };
    let pending = {
        let mut cache = state.kudos_cache.write().await;
        cache.take_pending()
    };
    let pending_len = pending.len();
    if pending_len == 0 {
        return Ok(());
    }
    let mut inserted = 0;
    for (path, interaction_id) in &pending {
        match insert_kudos(pool, path, interaction_id).await {
            Ok(true) => inserted += 1,
            Ok(false) => {}
            Err(err) => {
                let mut cache = state.kudos_cache.write().await;
                cache.restore_pending(pending);
                return Err(err.into());
            }
        }
    }
    info!(pending = pending_len, inserted, "kudos cache flushed");
    {
        let mut health = state.admin_health.lock().await;
        health.kudos_flush_last_success = Some(Utc::now());
    }
    Ok(())
}
