//! Log browse / tail for service and allowlisted sources.

use anyhow::{bail, Context, Result};
use chrono::{DateTime, Utc};
use serde::Serialize;
use std::fs::{self, File};
use std::io::{BufRead, BufReader, Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};

#[derive(Debug, Clone, Serialize)]
pub struct LogSourceInfo {
    pub id: String,
    pub label: String,
    pub path: String,
    pub exists: bool,
    pub size_bytes: Option<u64>,
}

#[derive(Debug, Clone, Serialize)]
pub struct LogTail {
    pub source_id: String,
    pub path: String,
    pub lines: Vec<String>,
    pub line_count: usize,
    pub truncated: bool,
    pub timestamp: DateTime<Utc>,
}

/// Built-in service log source id.
pub const SERVICE_LOG_ID: &str = "smos-service";

pub fn service_log_source(data_dir: &Path) -> LogSourceInfo {
    let path = crate::config::SmosConfig::service_log_path(data_dir);
    source_info(SERVICE_LOG_ID, "SMOS service log", &path)
}

pub fn source_info(id: &str, label: &str, path: &Path) -> LogSourceInfo {
    let meta = fs::metadata(path).ok();
    LogSourceInfo {
        id: id.to_string(),
        label: label.to_string(),
        path: path.to_string_lossy().to_string(),
        exists: meta.is_some(),
        size_bytes: meta.map(|m| m.len()),
    }
}

/// Tail the last `max_lines` lines from a file. Reads from end for efficiency.
pub fn tail_file(path: &Path, max_lines: usize) -> Result<Vec<String>> {
    if max_lines == 0 {
        return Ok(Vec::new());
    }
    if !path.exists() {
        // Empty rather than hard error — service log may not exist yet.
        return Ok(Vec::new());
    }

    let mut file = File::open(path).with_context(|| format!("open log {}", path.display()))?;
    let len = file.metadata()?.len();
    if len == 0 {
        return Ok(Vec::new());
    }

    // Read up to 1 MiB from the end for tailing.
    let window = 1024 * 1024u64;
    let start = len.saturating_sub(window);
    file.seek(SeekFrom::Start(start))?;

    let mut buf = String::new();
    file.read_to_string(&mut buf)?;

    let mut lines: Vec<String> = buf.lines().map(|s| s.to_string()).collect();
    // If we started mid-file, drop the first partial line.
    if start > 0 && !lines.is_empty() {
        lines.remove(0);
    }

    let truncated = lines.len() > max_lines;
    if truncated {
        let skip = lines.len() - max_lines;
        lines = lines.split_off(skip);
    }
    Ok(lines)
}

/// Full sequential read capped (for small files / browse).
pub fn read_file_lines(path: &Path, max_lines: usize) -> Result<Vec<String>> {
    if !path.exists() {
        return Ok(Vec::new());
    }
    let file = File::open(path).with_context(|| format!("open log {}", path.display()))?;
    let reader = BufReader::new(file);
    let mut lines = Vec::new();
    for line in reader.lines() {
        lines.push(line?);
        if lines.len() >= max_lines {
            break;
        }
    }
    Ok(lines)
}

pub fn tail_source(
    source_id: &str,
    path: &Path,
    max_lines: usize,
) -> Result<LogTail> {
    if max_lines == 0 || max_lines > 50_000 {
        bail!("max_lines must be 1..=50000");
    }
    let lines = tail_file(path, max_lines)?;
    let line_count = lines.len();
    Ok(LogTail {
        source_id: source_id.to_string(),
        path: path.to_string_lossy().to_string(),
        lines,
        line_count,
        truncated: line_count >= max_lines,
        timestamp: Utc::now(),
    })
}

/// Append a line to the active service log (used by tests / controlled writes).
pub fn append_service_line(data_dir: &Path, line: &str) -> Result<PathBuf> {
    fs::create_dir_all(data_dir)?;
    let path = crate::config::SmosConfig::service_log_path(data_dir);
    use std::io::Write;
    let mut f = fs::OpenOptions::new()
        .create(true)
        .append(true)
        .open(&path)?;
    writeln!(f, "{line}")?;
    Ok(path)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn tail_reads_recent_lines_from_real_file() {
        let dir = tempdir().unwrap();
        let path = dir.path().join("test.log");
        let mut content = String::new();
        for i in 0..50 {
            content.push_str(&format!("line-{i}\n"));
        }
        fs::write(&path, content).unwrap();

        let lines = tail_file(&path, 10).unwrap();
        assert_eq!(lines.len(), 10);
        assert_eq!(lines[0], "line-40");
        assert_eq!(lines[9], "line-49");
    }

    #[test]
    fn service_log_append_and_tail() {
        let dir = tempdir().unwrap();
        let path = append_service_line(dir.path(), "hello-smos-log").unwrap();
        append_service_line(dir.path(), "second-line").unwrap();
        let tail = tail_source(SERVICE_LOG_ID, &path, 50).unwrap();
        assert!(tail.lines.iter().any(|l| l.contains("hello-smos-log")));
        assert!(tail.lines.iter().any(|l| l.contains("second-line")));
        assert!(tail.line_count >= 2);
    }
}
