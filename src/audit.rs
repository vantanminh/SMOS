//! Append-only audit journal for management actions.

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use std::fs::{self, OpenOptions};
use std::io::{BufRead, BufReader, Write};
use std::path::{Path, PathBuf};
use uuid::Uuid;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub id: String,
    pub timestamp: DateTime<Utc>,
    pub action: String,
    pub actor: String,
    pub target: String,
    pub detail: serde_json::Value,
    pub success: bool,
}

pub struct AuditLog {
    path: PathBuf,
}

impl AuditLog {
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self { path: path.into() }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn ensure_parent(&self) -> Result<()> {
        if let Some(parent) = self.path.parent() {
            fs::create_dir_all(parent)
                .with_context(|| format!("create audit parent {}", parent.display()))?;
        }
        Ok(())
    }

    pub fn append(
        &self,
        action: impl Into<String>,
        actor: impl Into<String>,
        target: impl Into<String>,
        detail: serde_json::Value,
        success: bool,
    ) -> Result<AuditEntry> {
        self.ensure_parent()?;
        let entry = AuditEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            action: action.into(),
            actor: actor.into(),
            target: target.into(),
            detail,
            success,
        };
        let line = serde_json::to_string(&entry)?;
        let mut file = OpenOptions::new()
            .create(true)
            .append(true)
            .open(&self.path)
            .with_context(|| format!("open audit log {}", self.path.display()))?;
        writeln!(file, "{line}")?;
        file.flush()?;
        Ok(entry)
    }

    /// List newest-first, optional limit.
    pub fn list(&self, limit: usize) -> Result<Vec<AuditEntry>> {
        if !self.path.exists() {
            return Ok(Vec::new());
        }
        let file = fs::File::open(&self.path)
            .with_context(|| format!("read audit log {}", self.path.display()))?;
        let reader = BufReader::new(file);
        let mut entries: Vec<AuditEntry> = Vec::new();
        for line in reader.lines() {
            let line = line?;
            let line = line.trim();
            if line.is_empty() {
                continue;
            }
            match serde_json::from_str::<AuditEntry>(line) {
                Ok(e) => entries.push(e),
                Err(err) => {
                    tracing::warn!("skip corrupt audit line: {err}");
                }
            }
        }
        entries.reverse(); // newest last on disk → newest first
        if limit > 0 && entries.len() > limit {
            entries.truncate(limit);
        }
        Ok(entries)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn append_and_list_roundtrip() {
        let dir = tempdir().unwrap();
        let log = AuditLog::new(dir.path().join("audit.jsonl"));
        let e1 = log
            .append(
                "config.update",
                "operator",
                "config",
                serde_json::json!({"host_label": "a"}),
                true,
            )
            .unwrap();
        let e2 = log
            .append(
                "process.kill",
                "operator",
                "pid:123",
                serde_json::json!({"pid": 123}),
                false,
            )
            .unwrap();

        let listed = log.list(10).unwrap();
        assert_eq!(listed.len(), 2);
        // newest first
        assert_eq!(listed[0].id, e2.id);
        assert_eq!(listed[1].id, e1.id);
        assert_eq!(listed[0].action, "process.kill");
        assert!(!listed[0].success);
        assert!(listed[0].timestamp <= Utc::now());
    }
}
