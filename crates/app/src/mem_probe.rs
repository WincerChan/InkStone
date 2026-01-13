use std::path::{Path, PathBuf};

use chrono::Utc;
use tracing::{info, warn};

#[cfg(feature = "jemalloc")]
const HEAP_DUMP_DIR: &str = "/data/heap";

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

pub fn record(job: &str, stage: &str) {
    match read_smaps_rollup() {
        Ok(mut stats) => {
            #[cfg(feature = "jemalloc")]
            if let Err(err) = fill_jemalloc_stats(&mut stats) {
                warn!(job, stage, error = %err, "failed to read jemalloc stats");
            }
            info!(
                job,
                stage,
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
        }
        Err(err) => {
            warn!(job, stage, error = %err, "failed to read smaps_rollup");
        }
    }
    dump_heap(job, stage);
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

fn dump_heap(job: &str, stage: &str) {
    let _ = (job, stage);
    #[cfg(feature = "jemalloc")]
    {
        if let Err(err) = dump_heap_with_jemalloc(job, stage) {
            warn!(job, stage, error = %err, "heap dump failed");
        }
    }
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
