use std::sync::{Mutex, OnceLock};
use std::time::{Duration, Instant};

use tracing::{info, warn};

const HEAP_DUMP_DIR: &str = "/data/heap";
use chrono::Utc;
use std::path::{Path, PathBuf};

const DUMP_TRIGGER_BYTES: u64 = 4 * 1024 * 1024;
const DUMP_MIN_INTERVAL: Duration = Duration::from_secs(5 * 60);

static DUMP_STATE: OnceLock<Mutex<DumpState>> = OnceLock::new();

#[derive(Debug, Default)]
struct DumpState {
    last_metric_bytes: Option<u64>,
    last_dump_at: Option<Instant>,
    last_anon_metric_bytes: Option<u64>,
    last_anon_dump_at: Option<Instant>,
}

#[derive(Debug, Default)]
struct MemStats {
    rss_kb: Option<u64>,
    rss_anon_kb: Option<u64>,
    rss_file_kb: Option<u64>,
    pss_kb: Option<u64>,
    pss_anon_kb: Option<u64>,
    pss_file_kb: Option<u64>,
    jemalloc_allocated_bytes: Option<u64>,
    jemalloc_active_bytes: Option<u64>,
    jemalloc_resident_bytes: Option<u64>,
}

#[derive(Clone, Copy)]
pub struct MemProbeContext<'a> {
    pub component: &'a str,
    pub job: Option<&'a str>,
    pub stage: Option<&'a str>,
    pub route: Option<&'a str>,
    pub step: Option<&'a str>,
}

impl<'a> MemProbeContext<'a> {
    pub fn worker(job: &'a str, stage: &'a str) -> Self {
        Self {
            component: "worker",
            job: Some(job),
            stage: Some(stage),
            route: None,
            step: None,
        }
    }

    pub fn worker_step(job: &'a str, stage: &'a str, step: &'a str) -> Self {
        Self {
            component: "worker",
            job: Some(job),
            stage: Some(stage),
            route: None,
            step: Some(step),
        }
    }

    pub fn admin(route: &'a str, stage: &'a str) -> Self {
        Self {
            component: "admin",
            job: None,
            stage: Some(stage),
            route: Some(route),
            step: None,
        }
    }
}

pub fn record(job: &str, stage: &str) {
    record_with_context(MemProbeContext::worker(job, stage));
}

pub fn record_with_context(ctx: MemProbeContext<'_>) {
    match read_smaps_rollup() {
        Ok(stats) => {
            #[cfg(feature = "jemalloc")]
            let mut stats = stats;
            #[cfg(not(feature = "jemalloc"))]
            let stats = stats;
            #[cfg(feature = "jemalloc")]
            if let Err(err) = fill_jemalloc_stats(&mut stats) {
                warn!(
                    component = ctx.component,
                    job = ctx.job,
                    stage = ctx.stage,
                    route = ctx.route,
                    step = ctx.step,
                    error = %err,
                    "failed to read jemalloc stats"
                );
            }
            info!(
                component = ctx.component,
                job = ctx.job,
                stage = ctx.stage,
                route = ctx.route,
                step = ctx.step,
                rss_kb = stats.rss_kb,
                rss_anon_kb = stats.rss_anon_kb,
                rss_file_kb = stats.rss_file_kb,
                pss_kb = stats.pss_kb,
                pss_anon_kb = stats.pss_anon_kb,
                pss_file_kb = stats.pss_file_kb,
                jemalloc_allocated_bytes = stats.jemalloc_allocated_bytes,
                jemalloc_active_bytes = stats.jemalloc_active_bytes,
                jemalloc_resident_bytes = stats.jemalloc_resident_bytes,
                "memory snapshot"
            );
            maybe_dump(ctx, &stats);
        }
        Err(err) => {
            warn!(
                component = ctx.component,
                job = ctx.job,
                stage = ctx.stage,
                route = ctx.route,
                step = ctx.step,
                error = %err,
                "failed to read smaps_rollup"
            );
        }
    }
}

fn read_smaps_rollup() -> Result<MemStats, std::io::Error> {
    let content = std::fs::read_to_string("/proc/self/smaps_rollup")?;
    let mut stats = MemStats::default();
    for line in content.lines() {
        if let Some(value) = parse_kb(line, "Rss:") {
            stats.rss_kb = Some(value);
        } else if let Some(value) = parse_kb(line, "RssAnon:") {
            stats.rss_anon_kb = Some(value);
        } else if let Some(value) = parse_kb(line, "RssFile:") {
            stats.rss_file_kb = Some(value);
        } else if let Some(value) = parse_kb(line, "Pss:") {
            stats.pss_kb = Some(value);
        } else if let Some(value) = parse_kb(line, "Pss_Anon:") {
            stats.pss_anon_kb = Some(value);
        } else if let Some(value) = parse_kb(line, "Pss_File:") {
            stats.pss_file_kb = Some(value);
        }
    }
    Ok(stats)
}

fn parse_kb(line: &str, key: &str) -> Option<u64> {
    let value = line.strip_prefix(key)?.trim();
    let mut parts = value.split_whitespace();
    let number = parts.next()?.parse::<u64>().ok()?;
    Some(number)
}

fn trigger_metric_bytes(stats: &MemStats) -> Option<u64> {
    stats
        .jemalloc_resident_bytes
        .or(stats.jemalloc_active_bytes)
        .or(stats.jemalloc_allocated_bytes)
        .or(stats.pss_anon_kb.map(|value| value * 1024))
        .or(stats.rss_kb.map(|value| value * 1024))
}

fn anon_metric_bytes(stats: &MemStats) -> Option<u64> {
    stats
        .pss_anon_kb
        .or(stats.rss_anon_kb)
        .or(stats.rss_kb)
        .map(|value| value * 1024)
}

fn maybe_dump(ctx: MemProbeContext<'_>, stats: &MemStats) {
    let state = DUMP_STATE.get_or_init(|| Mutex::new(DumpState::default()));
    let mut guard = state.lock().expect("mem probe dump state poisoned");
    let now = Instant::now();
    if let Some(metric_bytes) = trigger_metric_bytes(stats) {
        let delta = guard
            .last_metric_bytes
            .and_then(|previous| metric_bytes.checked_sub(previous))
            .unwrap_or(0);
        if delta >= DUMP_TRIGGER_BYTES {
            let should_dump = guard
                .last_dump_at
                .map(|last| now.duration_since(last) >= DUMP_MIN_INTERVAL)
                .unwrap_or(true);
            if should_dump {
                let job = ctx.job.unwrap_or(ctx.component);
                let stage = ctx.stage.unwrap_or("snapshot");
                info!(
                    component = ctx.component,
                    job = ctx.job,
                    stage = ctx.stage,
                    route = ctx.route,
                    step = ctx.step,
                    delta_bytes = delta,
                    metric_bytes,
                    "heap dump triggered by memory growth"
                );
                if dump_heap(job, stage) {
                    guard.last_dump_at = Some(now);
                }
            }
        }
        guard.last_metric_bytes = Some(metric_bytes);
    }

    if let Some(metric_bytes) = anon_metric_bytes(stats) {
        let delta = guard
            .last_anon_metric_bytes
            .and_then(|previous| metric_bytes.checked_sub(previous))
            .unwrap_or(0);
        if delta >= DUMP_TRIGGER_BYTES {
            let should_dump = guard
                .last_anon_dump_at
                .map(|last| now.duration_since(last) >= DUMP_MIN_INTERVAL)
                .unwrap_or(true);
            if should_dump {
                let job = ctx.job.unwrap_or(ctx.component);
                let stage = ctx.stage.unwrap_or("snapshot");
                info!(
                    component = ctx.component,
                    job = ctx.job,
                    stage = ctx.stage,
                    route = ctx.route,
                    step = ctx.step,
                    delta_bytes = delta,
                    anon_metric_bytes = metric_bytes,
                    "smaps dump triggered by anon memory growth"
                );
                if dump_smaps(job, stage) {
                    guard.last_anon_dump_at = Some(now);
                }
            }
        }
        guard.last_anon_metric_bytes = Some(metric_bytes);
    }
}

#[cfg(feature = "jemalloc")]
fn fill_jemalloc_stats(stats: &mut MemStats) -> Result<(), String> {
    use tikv_jemalloc_ctl::{epoch, stats as jemalloc_stats};

    epoch::advance().map_err(|err| err.to_string())?;
    let allocated = jemalloc_stats::allocated::read().map_err(|err| err.to_string())?;
    let active = jemalloc_stats::active::read().map_err(|err| err.to_string())?;
    let resident = jemalloc_stats::resident::read().map_err(|err| err.to_string())?;
    stats.jemalloc_allocated_bytes = Some(allocated as u64);
    stats.jemalloc_active_bytes = Some(active as u64);
    stats.jemalloc_resident_bytes = Some(resident as u64);
    Ok(())
}

fn dump_heap(job: &str, stage: &str) -> bool {
    let _ = (job, stage);
    #[cfg(feature = "jemalloc")]
    {
        if let Err(err) = dump_heap_with_jemalloc(job, stage) {
            warn!(job, stage, error = %err, "heap dump failed");
            return false;
        }
        return true;
    }
    #[cfg(not(feature = "jemalloc"))]
    {
        false
    }
}

fn dump_smaps(job: &str, stage: &str) -> bool {
    if let Err(err) = dump_smaps_with_proc(job, stage) {
        warn!(job, stage, error = %err, "smaps dump failed");
        return false;
    }
    true
}

fn dump_smaps_with_proc(job: &str, stage: &str) -> Result<(), String> {
    let dir = Path::new(HEAP_DUMP_DIR);
    std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    let path = smaps_path(dir, job, stage);
    let path_str = path.to_string_lossy().to_string();
    let mut src = std::fs::File::open("/proc/self/smaps").map_err(|err| err.to_string())?;
    let mut dst = std::fs::File::create(&path).map_err(|err| err.to_string())?;
    std::io::copy(&mut src, &mut dst).map_err(|err| err.to_string())?;
    info!(job, stage, path = %path_str, "smaps dump written");
    Ok(())
}

#[cfg(feature = "jemalloc")]
fn dump_heap_with_jemalloc(job: &str, stage: &str) -> Result<(), String> {
    let dir = Path::new(HEAP_DUMP_DIR);
    std::fs::create_dir_all(dir).map_err(|err| err.to_string())?;
    let path = dump_path(dir, job, stage);
    let path_str = path.to_string_lossy().to_string();
    let c_path = std::ffi::CString::new(path_str.clone()).map_err(|err| err.to_string())?;
    unsafe {
        tikv_jemalloc_ctl::raw::write(
            b"prof.dump\0",
            c_path.as_ptr() as *mut std::ffi::c_void,
        )
        .map_err(|err| err.to_string())?;
    }
    info!(job, stage, path = %path_str, "heap dump written");
    Ok(())
}

#[cfg(feature = "jemalloc")]
fn dump_path(dir: &Path, job: &str, stage: &str) -> PathBuf {
    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let name = format!("{job}_{stage}_{timestamp}.heap");
    dir.join(name)
}

fn smaps_path(dir: &Path, job: &str, stage: &str) -> PathBuf {
    let timestamp = Utc::now().format("%Y%m%dT%H%M%SZ");
    let name = format!("smaps_{job}_{stage}_{timestamp}.txt");
    dir.join(name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn anon_metric_prefers_pss_anon() {
        let mut stats = MemStats::default();
        stats.pss_anon_kb = Some(12);
        stats.rss_anon_kb = Some(34);
        stats.rss_kb = Some(56);
        assert_eq!(anon_metric_bytes(&stats), Some(12 * 1024));
    }

    #[test]
    fn anon_metric_falls_back_to_rss_anon() {
        let mut stats = MemStats::default();
        stats.rss_anon_kb = Some(34);
        stats.rss_kb = Some(56);
        assert_eq!(anon_metric_bytes(&stats), Some(34 * 1024));
    }

    #[test]
    fn anon_metric_falls_back_to_rss() {
        let mut stats = MemStats::default();
        stats.rss_kb = Some(56);
        assert_eq!(anon_metric_bytes(&stats), Some(56 * 1024));
    }

    #[test]
    fn trigger_metric_prefers_jemalloc_resident() {
        let mut stats = MemStats::default();
        stats.jemalloc_resident_bytes = Some(42);
        stats.pss_anon_kb = Some(1000);
        assert_eq!(trigger_metric_bytes(&stats), Some(42));
    }
}
