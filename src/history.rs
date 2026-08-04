//! Durable metrics and log history with configurable retention.

use crate::metrics::MetricsSnapshot;
use anyhow::{Context, Result};
use chrono::{DateTime, Duration, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};

/// Default sample interval for persisted metrics (seconds).
pub const DEFAULT_METRICS_SAMPLE_SECS: u64 = 60;

/// Compact on-disk metrics sample (one JSONL line).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct MetricsSample {
    pub t: DateTime<Utc>,
    pub cpu: f32,
    pub mem: f64,
    pub mem_used: u64,
    pub mem_total: u64,
    pub swap_used: u64,
    pub swap_total: u64,
    pub disks: Vec<DiskSample>,
    pub load: Option<[f64; 3]>,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct DiskSample {
    pub mount: String,
    pub used: u64,
    pub total: u64,
    pub pct: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct MetricsHistoryResponse {
    pub samples: Vec<MetricsSample>,
    pub from: Option<DateTime<Utc>>,
    pub to: Option<DateTime<Utc>>,
    pub count: usize,
    pub retention_days: u32,
    pub sample_interval_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct HistoryStatus {
    pub retention_days: u32,
    pub metrics_path: String,
    pub metrics_samples_on_disk: usize,
    pub metrics_oldest: Option<DateTime<Utc>>,
    pub metrics_newest: Option<DateTime<Utc>>,
    pub metrics_bytes: u64,
    pub log_files: Vec<LogHistoryFile>,
    pub log_bytes_total: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogHistoryFile {
    pub name: String,
    pub path: String,
    pub size_bytes: u64,
    pub modified: Option<DateTime<Utc>>,
}

pub fn history_dir(data_dir: &Path) -> PathBuf {
    data_dir.join("history")
}

pub fn metrics_history_path(data_dir: &Path) -> PathBuf {
    history_dir(data_dir).join("metrics.jsonl")
}

impl From<&MetricsSnapshot> for MetricsSample {
    fn from(s: &MetricsSnapshot) -> Self {
        Self {
            t: s.timestamp,
            cpu: s.cpu.usage_percent,
            mem: s.memory.usage_percent,
            mem_used: s.memory.used_bytes,
            mem_total: s.memory.total_bytes,
            swap_used: s.memory.swap_used_bytes,
            swap_total: s.memory.swap_total_bytes,
            disks: s
                .disks
                .iter()
                .map(|d| DiskSample {
                    mount: d.mount_point.clone(),
                    used: d.used_bytes,
                    total: d.total_bytes,
                    pct: d.usage_percent,
                })
                .collect(),
            load: s.load_avg.as_ref().map(|l| [l.one, l.five, l.fifteen]),
            uptime_secs: s.uptime_secs,
        }
    }
}

/// Append one metrics sample to the JSONL history file.
pub fn append_metrics_sample(data_dir: &Path, sample: &MetricsSample) -> Result<()> {
    let dir = history_dir(data_dir);
    fs::create_dir_all(&dir)
        .with_context(|| format!("create history dir {}", dir.display()))?;
    let path = metrics_history_path(data_dir);
    let mut file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)
        .with_context(|| format!("open metrics history {}", path.display()))?;
    let line = serde_json::to_string(sample)?;
    writeln!(file, "{line}")?;
    file.flush()?;
    Ok(())
}

/// Query samples in `[from, to]` (inclusive bounds when set), oldest-first, optional max points.
///
/// When more samples exist than `max_points`, evenly downsamples for chart-friendly payloads.
pub fn query_metrics(
    data_dir: &Path,
    from: Option<DateTime<Utc>>,
    to: Option<DateTime<Utc>>,
    max_points: usize,
) -> Result<Vec<MetricsSample>> {
    let path = metrics_history_path(data_dir);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(&path)
        .with_context(|| format!("read metrics history {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut samples = Vec::new();
    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let sample: MetricsSample = match serde_json::from_str(line) {
            Ok(s) => s,
            Err(err) => {
                tracing::warn!("skip corrupt metrics history line: {err}");
                continue;
            }
        };
        if let Some(from) = from {
            if sample.t < from {
                continue;
            }
        }
        if let Some(to) = to {
            if sample.t > to {
                continue;
            }
        }
        samples.push(sample);
    }
    // File is append-only chronological; keep order.
    if max_points > 0 && samples.len() > max_points {
        samples = downsample(samples, max_points);
    }
    Ok(samples)
}

fn downsample(samples: Vec<MetricsSample>, max_points: usize) -> Vec<MetricsSample> {
    if samples.len() <= max_points || max_points == 0 {
        return samples;
    }
    let n = samples.len();
    let mut out = Vec::with_capacity(max_points);
    for i in 0..max_points {
        let idx = if max_points == 1 {
            n - 1
        } else {
            i * (n - 1) / (max_points - 1)
        };
        out.push(samples[idx].clone());
    }
    out
}

/// Drop metrics samples older than `retention_days`. Rewrites the file when needed.
pub fn prune_metrics(data_dir: &Path, retention_days: u32) -> Result<usize> {
    let path = metrics_history_path(data_dir);
    if !path.exists() {
        return Ok(0);
    }
    let cutoff = Utc::now() - Duration::days(i64::from(retention_days).max(1));
    let file = File::open(&path)
        .with_context(|| format!("read metrics history {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut kept = Vec::new();
    let mut removed = 0usize;
    for line in reader.lines() {
        let line = line?;
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        match serde_json::from_str::<MetricsSample>(trimmed) {
            Ok(sample) if sample.t >= cutoff => kept.push(line),
            Ok(_) => removed += 1,
            Err(_) => {
                // Drop unreadable lines during prune.
                removed += 1;
            }
        }
    }
    if removed == 0 {
        return Ok(0);
    }
    let tmp = path.with_extension("jsonl.tmp");
    {
        let mut out = OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .open(&tmp)
            .with_context(|| format!("write temp metrics history {}", tmp.display()))?;
        for line in &kept {
            writeln!(out, "{}", line.trim_end())?;
        }
        out.flush()?;
    }
    fs::rename(&tmp, &path)
        .with_context(|| format!("replace metrics history {}", path.display()))?;
    Ok(removed)
}

/// List rotated / live service log files under `data_dir`.
pub fn list_log_history_files(data_dir: &Path) -> Result<Vec<LogHistoryFile>> {
    let mut files = Vec::new();
    if !data_dir.exists() {
        return Ok(files);
    }
    for entry in fs::read_dir(data_dir)? {
        let entry = entry?;
        let name = entry.file_name().to_string_lossy().to_string();
        // Live/legacy log + tracing-appender daily rotations (smos.log.YYYY-MM-DD).
        if name == "smos.log" || (name.starts_with("smos.log.") && !name.ends_with(".tmp")) {
            let meta = entry.metadata()?;
            if !meta.is_file() {
                continue;
            }
            let modified = meta.modified().ok().map(|t| {
                let dt: DateTime<Utc> = t.into();
                dt
            });
            files.push(LogHistoryFile {
                name: name.clone(),
                path: entry.path().to_string_lossy().to_string(),
                size_bytes: meta.len(),
                modified,
            });
        }
    }
    files.sort_by(|a, b| b.name.cmp(&a.name));
    Ok(files)
}

/// Parse date suffix from rotated log names like `smos.log.2020-01-01`.
fn log_file_day(name: &str) -> Option<chrono::NaiveDate> {
    let suffix = name.strip_prefix("smos.log.")?;
    chrono::NaiveDate::parse_from_str(suffix, "%Y-%m-%d").ok()
}

/// Delete rotated service log files older than retention (never deletes live `smos.log`).
///
/// Prefers the `YYYY-MM-DD` suffix in the filename; falls back to mtime.
pub fn prune_log_files(data_dir: &Path, retention_days: u32) -> Result<usize> {
    let cutoff_date = (Utc::now() - Duration::days(i64::from(retention_days).max(1))).date_naive();
    let cutoff_time = Utc::now() - Duration::days(i64::from(retention_days).max(1));
    let mut removed = 0usize;
    for file in list_log_history_files(data_dir)? {
        if file.name == "smos.log" {
            continue;
        }
        let expired = if let Some(day) = log_file_day(&file.name) {
            day < cutoff_date
        } else if let Some(modified) = file.modified {
            modified < cutoff_time
        } else {
            false
        };
        if expired {
            let path = PathBuf::from(&file.path);
            if path.exists() {
                fs::remove_file(&path)
                    .with_context(|| format!("remove old log {}", path.display()))?;
                removed += 1;
                tracing::info!(path = %path.display(), "pruned log file past retention");
            }
        }
    }
    Ok(removed)
}

/// Run metrics + log prune for configured retention.
pub fn prune_all(data_dir: &Path, retention_days: u32) -> Result<(usize, usize)> {
    let m = prune_metrics(data_dir, retention_days)?;
    let l = prune_log_files(data_dir, retention_days)?;
    Ok((m, l))
}

pub fn history_status(data_dir: &Path, retention_days: u32) -> Result<HistoryStatus> {
    let path = metrics_history_path(data_dir);
    let metrics_bytes = fs::metadata(&path).map(|m| m.len()).unwrap_or(0);
    let samples = query_metrics(data_dir, None, None, 0)?;
    let metrics_oldest = samples.first().map(|s| s.t);
    let metrics_newest = samples.last().map(|s| s.t);
    let log_files = list_log_history_files(data_dir)?;
    let log_bytes_total = log_files.iter().map(|f| f.size_bytes).sum();
    Ok(HistoryStatus {
        retention_days,
        metrics_path: path.to_string_lossy().to_string(),
        metrics_samples_on_disk: samples.len(),
        metrics_oldest,
        metrics_newest,
        metrics_bytes,
        log_files,
        log_bytes_total,
    })
}

/// Collect + append one sample (used by the background worker).
pub fn record_metrics_snapshot(data_dir: &Path, snap: &MetricsSnapshot) -> Result<()> {
    let sample = MetricsSample::from(snap);
    append_metrics_sample(data_dir, &sample)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    fn sample_at(secs_ago: i64, cpu: f32) -> MetricsSample {
        MetricsSample {
            t: Utc::now() - Duration::seconds(secs_ago),
            cpu,
            mem: 50.0,
            mem_used: 100,
            mem_total: 200,
            swap_used: 0,
            swap_total: 0,
            disks: vec![DiskSample {
                mount: "/".into(),
                used: 1,
                total: 2,
                pct: 50.0,
            }],
            load: Some([0.1, 0.2, 0.3]),
            uptime_secs: 1000,
        }
    }

    #[test]
    fn append_query_and_prune_metrics() {
        let dir = tempdir().unwrap();
        let data = dir.path();
        append_metrics_sample(data, &sample_at(0, 10.0)).unwrap();
        append_metrics_sample(data, &sample_at(0, 20.0)).unwrap();

        let all = query_metrics(data, None, None, 0).unwrap();
        assert_eq!(all.len(), 2);
        assert_eq!(all[0].cpu, 10.0);
        assert_eq!(all[1].cpu, 20.0);

        // Force one sample older than 1 day retention.
        let path = metrics_history_path(data);
        let mut old = sample_at(0, 1.0);
        old.t = Utc::now() - Duration::days(5);
        let line = serde_json::to_string(&old).unwrap();
        let mut content = fs::read_to_string(&path).unwrap();
        content = format!("{line}\n{content}");
        fs::write(&path, content).unwrap();

        let removed = prune_metrics(data, 1).unwrap();
        assert!(removed >= 1);
        let left = query_metrics(data, None, None, 0).unwrap();
        assert!(left.iter().all(|s| s.cpu != 1.0));
        assert!(left.len() >= 2);
    }

    #[test]
    fn downsample_limits_points() {
        let dir = tempdir().unwrap();
        for i in 0..100 {
            append_metrics_sample(dir.path(), &sample_at(i, i as f32)).unwrap();
        }
        let q = query_metrics(dir.path(), None, None, 10).unwrap();
        assert_eq!(q.len(), 10);
        // First and last preserved approximately
        assert_eq!(q.first().unwrap().cpu, 0.0);
        assert_eq!(q.last().unwrap().cpu, 99.0);
    }

    #[test]
    fn prune_old_rotated_logs_keeps_live() {
        let dir = tempdir().unwrap();
        let data = dir.path();
        fs::write(data.join("smos.log"), "live\n").unwrap();
        let old_name = data.join("smos.log.2020-01-01");
        fs::write(&old_name, "old\n").unwrap();
        let recent = data.join(format!(
            "smos.log.{}",
            Utc::now().format("%Y-%m-%d")
        ));
        fs::write(&recent, "today\n").unwrap();

        let files = list_log_history_files(data).unwrap();
        assert!(files.iter().any(|f| f.name == "smos.log"));
        assert!(files.iter().any(|f| f.name == "smos.log.2020-01-01"));

        let removed = prune_log_files(data, 30).unwrap();
        assert_eq!(removed, 1);
        assert!(data.join("smos.log").exists());
        assert!(!old_name.exists());
        assert!(recent.exists());
    }

    #[test]
    fn history_status_reports_counts() {
        let dir = tempdir().unwrap();
        append_metrics_sample(dir.path(), &sample_at(0, 5.0)).unwrap();
        fs::write(dir.path().join("smos.log"), "x\n").unwrap();
        let st = history_status(dir.path(), 30).unwrap();
        assert_eq!(st.retention_days, 30);
        assert_eq!(st.metrics_samples_on_disk, 1);
        assert!(st.metrics_bytes > 0);
        assert!(!st.log_files.is_empty());
    }
}
