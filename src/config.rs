//! SMOS configuration load, validate, persist.

use crate::alerts::{
    self, default_alert_cpu_percent, default_alert_disk_percent, default_alert_memory_percent,
    AlertThresholds,
};
use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use std::fs;
use std::net::SocketAddr;
use std::path::{Path, PathBuf};

/// Runtime configuration for the SMOS service.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SmosConfig {
    /// HTTP bind address (e.g. 127.0.0.1:9090).
    pub bind: String,
    /// Optional shared secret. When set, mutating API routes require
    /// `Authorization: Bearer <token>` or `X-SMOS-Token` header.
    pub auth_token: Option<String>,
    /// Directory for config, audit journal, and service logs.
    pub data_dir: PathBuf,
    /// How many log lines to return by default when tailing.
    pub log_tail_lines: usize,
    /// Optional extra log file paths the operator may browse (allowlist).
    pub log_sources: Vec<LogSource>,
    /// Friendly display name for this host in the dashboard.
    pub host_label: String,
    /// Metrics poll interval hint for the UI (seconds).
    pub metrics_poll_secs: u64,
    /// How many days of metrics/log history to retain on disk (default 30).
    #[serde(default = "default_history_retention_days")]
    pub history_retention_days: u32,
    /// How often to persist a metrics sample for history (seconds).
    #[serde(default = "default_metrics_history_interval_secs")]
    pub metrics_history_interval_secs: u64,
    /// Alert when global CPU usage % is at or above this value (default 90).
    #[serde(default = "default_alert_cpu_percent")]
    pub alert_cpu_percent: f64,
    /// Alert when memory usage % is at or above this value (default 90).
    #[serde(default = "default_alert_memory_percent")]
    pub alert_memory_percent: f64,
    /// Alert when any disk usage % is at or above this value (default 90).
    #[serde(default = "default_alert_disk_percent")]
    pub alert_disk_percent: f64,
}

fn default_history_retention_days() -> u32 {
    30
}

fn default_metrics_history_interval_secs() -> u64 {
    60
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct LogSource {
    pub id: String,
    pub label: String,
    pub path: PathBuf,
}

/// On-disk subset operators may edit through the API (validated).
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct SmosConfigFile {
    pub bind: String,
    pub auth_token: Option<String>,
    pub data_dir: PathBuf,
    pub log_tail_lines: usize,
    pub log_sources: Vec<LogSource>,
    pub host_label: String,
    pub metrics_poll_secs: u64,
    #[serde(default = "default_history_retention_days")]
    pub history_retention_days: u32,
    #[serde(default = "default_metrics_history_interval_secs")]
    pub metrics_history_interval_secs: u64,
    #[serde(default = "default_alert_cpu_percent")]
    pub alert_cpu_percent: f64,
    #[serde(default = "default_alert_memory_percent")]
    pub alert_memory_percent: f64,
    #[serde(default = "default_alert_disk_percent")]
    pub alert_disk_percent: f64,
}

impl Default for SmosConfig {
    fn default() -> Self {
        Self {
            bind: "127.0.0.1:9090".into(),
            auth_token: None,
            data_dir: PathBuf::from("smos-data"),
            log_tail_lines: 200,
            log_sources: Vec::new(),
            host_label: "smos-host".into(),
            metrics_poll_secs: 2,
            history_retention_days: default_history_retention_days(),
            metrics_history_interval_secs: default_metrics_history_interval_secs(),
            alert_cpu_percent: default_alert_cpu_percent(),
            alert_memory_percent: default_alert_memory_percent(),
            alert_disk_percent: default_alert_disk_percent(),
        }
    }
}

impl From<SmosConfig> for SmosConfigFile {
    fn from(c: SmosConfig) -> Self {
        Self {
            bind: c.bind,
            auth_token: c.auth_token,
            data_dir: c.data_dir,
            log_tail_lines: c.log_tail_lines,
            log_sources: c.log_sources,
            host_label: c.host_label,
            metrics_poll_secs: c.metrics_poll_secs,
            history_retention_days: c.history_retention_days,
            metrics_history_interval_secs: c.metrics_history_interval_secs,
            alert_cpu_percent: c.alert_cpu_percent,
            alert_memory_percent: c.alert_memory_percent,
            alert_disk_percent: c.alert_disk_percent,
        }
    }
}

impl From<SmosConfigFile> for SmosConfig {
    fn from(c: SmosConfigFile) -> Self {
        Self {
            bind: c.bind,
            auth_token: c.auth_token,
            data_dir: c.data_dir,
            log_tail_lines: c.log_tail_lines,
            log_sources: c.log_sources,
            host_label: c.host_label,
            metrics_poll_secs: c.metrics_poll_secs,
            history_retention_days: c.history_retention_days,
            metrics_history_interval_secs: c.metrics_history_interval_secs,
            alert_cpu_percent: c.alert_cpu_percent,
            alert_memory_percent: c.alert_memory_percent,
            alert_disk_percent: c.alert_disk_percent,
        }
    }
}

/// Partial update payload (only provided fields change).
#[derive(Debug, Clone, Deserialize, Default)]
pub struct ConfigUpdate {
    pub bind: Option<String>,
    pub auth_token: Option<Option<String>>,
    pub log_tail_lines: Option<usize>,
    pub host_label: Option<String>,
    pub metrics_poll_secs: Option<u64>,
    pub log_sources: Option<Vec<LogSource>>,
    pub history_retention_days: Option<u32>,
    pub metrics_history_interval_secs: Option<u64>,
    pub alert_cpu_percent: Option<f64>,
    pub alert_memory_percent: Option<f64>,
    pub alert_disk_percent: Option<f64>,
}

impl SmosConfig {
    pub fn config_path(data_dir: &Path) -> PathBuf {
        data_dir.join("config.json")
    }

    /// Active service log path for the current UTC day (`smos.log.YYYY-MM-DD`).
    /// Falls back to legacy `smos.log` when present (tests / older installs).
    pub fn service_log_path(data_dir: &Path) -> PathBuf {
        let today = chrono::Utc::now().format("%Y-%m-%d");
        let rotated = data_dir.join(format!("smos.log.{today}"));
        if rotated.exists() {
            return rotated;
        }
        let legacy = data_dir.join("smos.log");
        if legacy.exists() {
            return legacy;
        }
        // Prefer daily name so it matches tracing_appender::rolling::daily.
        rotated
    }

    pub fn audit_path(data_dir: &Path) -> PathBuf {
        data_dir.join("audit.jsonl")
    }

    pub fn ensure_data_dir(&self) -> Result<()> {
        fs::create_dir_all(&self.data_dir)
            .with_context(|| format!("create data dir {}", self.data_dir.display()))?;
        Ok(())
    }

    pub fn load_or_default(data_dir: &Path) -> Result<Self> {
        fs::create_dir_all(data_dir)
            .with_context(|| format!("create data dir {}", data_dir.display()))?;
        let path = Self::config_path(data_dir);
        if path.exists() {
            let raw = fs::read_to_string(&path)
                .with_context(|| format!("read config {}", path.display()))?;
            let file: SmosConfigFile =
                serde_json::from_str(&raw).with_context(|| "parse config.json")?;
            let mut cfg: SmosConfig = file.into();
            // data_dir from CLI/env takes precedence over on-disk value for path stability
            cfg.data_dir = data_dir.to_path_buf();
            cfg.validate()?;
            Ok(cfg)
        } else {
            let mut cfg = Self::default();
            cfg.data_dir = data_dir.to_path_buf();
            cfg.save()?;
            Ok(cfg)
        }
    }

    pub fn save(&self) -> Result<()> {
        self.validate()?;
        self.ensure_data_dir()?;
        let path = Self::config_path(&self.data_dir);
        let file: SmosConfigFile = self.clone().into();
        let raw = serde_json::to_string_pretty(&file)?;
        fs::write(&path, raw).with_context(|| format!("write config {}", path.display()))?;
        Ok(())
    }

    pub fn validate(&self) -> Result<()> {
        if self.bind.trim().is_empty() {
            bail!("bind address must not be empty");
        }
        // Accept host:port form
        if self.bind.parse::<SocketAddr>().is_err() {
            // allow hostnames like localhost:9090
            let parts: Vec<&str> = self.bind.rsplitn(2, ':').collect();
            if parts.len() != 2 || parts[0].parse::<u16>().is_err() || parts[1].is_empty() {
                bail!("invalid bind address '{}'; expected host:port", self.bind);
            }
        }
        if self.log_tail_lines == 0 || self.log_tail_lines > 50_000 {
            bail!("log_tail_lines must be between 1 and 50000");
        }
        if self.host_label.trim().is_empty() || self.host_label.len() > 128 {
            bail!("host_label must be 1..=128 characters");
        }
        if self.metrics_poll_secs == 0 || self.metrics_poll_secs > 3600 {
            bail!("metrics_poll_secs must be between 1 and 3600");
        }
        if self.history_retention_days == 0 || self.history_retention_days > 3650 {
            bail!("history_retention_days must be between 1 and 3650");
        }
        if self.metrics_history_interval_secs < 10
            || self.metrics_history_interval_secs > 3600
        {
            bail!("metrics_history_interval_secs must be between 10 and 3600");
        }
        if let Err(msg) = alerts::validate_thresholds(&self.alert_thresholds()) {
            bail!("{msg}");
        }
        for src in &self.log_sources {
            if src.id.trim().is_empty() {
                bail!("log source id must not be empty");
            }
            if src.label.trim().is_empty() {
                bail!("log source label must not be empty");
            }
        }
        if let Some(ref token) = self.auth_token {
            if token.is_empty() {
                bail!("auth_token if set must not be empty string");
            }
        }
        Ok(())
    }

    /// Apply a partial update atomically: on validation failure, `self` is unchanged.
    pub fn apply_update(&mut self, update: ConfigUpdate) -> Result<()> {
        let mut next = self.clone();
        if let Some(bind) = update.bind {
            next.bind = bind;
        }
        if let Some(token) = update.auth_token {
            next.auth_token = token;
        }
        if let Some(n) = update.log_tail_lines {
            next.log_tail_lines = n;
        }
        if let Some(label) = update.host_label {
            next.host_label = label;
        }
        if let Some(secs) = update.metrics_poll_secs {
            next.metrics_poll_secs = secs;
        }
        if let Some(sources) = update.log_sources {
            next.log_sources = sources;
        }
        if let Some(days) = update.history_retention_days {
            next.history_retention_days = days;
        }
        if let Some(secs) = update.metrics_history_interval_secs {
            next.metrics_history_interval_secs = secs;
        }
        if let Some(v) = update.alert_cpu_percent {
            next.alert_cpu_percent = v;
        }
        if let Some(v) = update.alert_memory_percent {
            next.alert_memory_percent = v;
        }
        if let Some(v) = update.alert_disk_percent {
            next.alert_disk_percent = v;
        }
        next.validate()?;
        *self = next;
        Ok(())
    }

    /// Alert thresholds as a dedicated struct for evaluation.
    pub fn alert_thresholds(&self) -> AlertThresholds {
        AlertThresholds {
            cpu_percent: self.alert_cpu_percent,
            memory_percent: self.alert_memory_percent,
            disk_percent: self.alert_disk_percent,
        }
    }

    /// Public view redacts auth token value (presence only).
    pub fn public_view(&self) -> PublicConfig {
        PublicConfig {
            bind: self.bind.clone(),
            auth_token_set: self.auth_token.is_some(),
            data_dir: self.data_dir.display().to_string(),
            log_tail_lines: self.log_tail_lines,
            log_sources: self.log_sources.clone(),
            host_label: self.host_label.clone(),
            metrics_poll_secs: self.metrics_poll_secs,
            history_retention_days: self.history_retention_days,
            metrics_history_interval_secs: self.metrics_history_interval_secs,
            alert_cpu_percent: self.alert_cpu_percent,
            alert_memory_percent: self.alert_memory_percent,
            alert_disk_percent: self.alert_disk_percent,
        }
    }
}

#[derive(Debug, Clone, Serialize)]
pub struct PublicConfig {
    pub bind: String,
    pub auth_token_set: bool,
    pub data_dir: String,
    pub log_tail_lines: usize,
    pub log_sources: Vec<LogSource>,
    pub host_label: String,
    pub metrics_poll_secs: u64,
    pub history_retention_days: u32,
    pub metrics_history_interval_secs: u64,
    pub alert_cpu_percent: f64,
    pub alert_memory_percent: f64,
    pub alert_disk_percent: f64,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn default_config_validates() {
        let cfg = SmosConfig::default();
        cfg.validate().unwrap();
    }

    #[test]
    fn rejects_empty_host_label() {
        let mut cfg = SmosConfig::default();
        cfg.host_label = "  ".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn rejects_bad_bind() {
        let mut cfg = SmosConfig::default();
        cfg.bind = "not-an-address".into();
        assert!(cfg.validate().is_err());
    }

    #[test]
    fn load_save_roundtrip() {
        let dir = tempdir().unwrap();
        let mut cfg = SmosConfig::load_or_default(dir.path()).unwrap();
        cfg.host_label = "vps-alpha".into();
        cfg.log_tail_lines = 500;
        cfg.save().unwrap();

        let loaded = SmosConfig::load_or_default(dir.path()).unwrap();
        assert_eq!(loaded.host_label, "vps-alpha");
        assert_eq!(loaded.log_tail_lines, 500);
    }

    #[test]
    fn apply_update_validates() {
        let mut cfg = SmosConfig::default();
        let err = cfg
            .apply_update(ConfigUpdate {
                log_tail_lines: Some(0),
                ..Default::default()
            })
            .unwrap_err();
        assert!(err.to_string().contains("log_tail_lines"));
    }

    #[test]
    fn history_retention_defaults_and_validates() {
        let cfg = SmosConfig::default();
        assert_eq!(cfg.history_retention_days, 30);
        assert_eq!(cfg.metrics_history_interval_secs, 60);
        cfg.validate().unwrap();

        let mut bad = SmosConfig::default();
        bad.history_retention_days = 0;
        assert!(bad.validate().is_err());
        bad.history_retention_days = 30;
        bad.metrics_history_interval_secs = 5;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn load_missing_history_fields_uses_defaults() {
        let dir = tempdir().unwrap();
        let path = SmosConfig::config_path(dir.path());
        // Legacy config without history keys must still load.
        fs::write(
            &path,
            r#"{
              "bind": "127.0.0.1:9090",
              "auth_token": null,
              "data_dir": "ignored",
              "log_tail_lines": 200,
              "log_sources": [],
              "host_label": "legacy",
              "metrics_poll_secs": 2
            }"#,
        )
        .unwrap();
        let loaded = SmosConfig::load_or_default(dir.path()).unwrap();
        assert_eq!(loaded.host_label, "legacy");
        assert_eq!(loaded.history_retention_days, 30);
        assert_eq!(loaded.metrics_history_interval_secs, 60);
        assert_eq!(loaded.alert_cpu_percent, 90.0);
        assert_eq!(loaded.alert_memory_percent, 90.0);
        assert_eq!(loaded.alert_disk_percent, 90.0);
    }

    #[test]
    fn alert_thresholds_defaults_and_validate() {
        let cfg = SmosConfig::default();
        assert_eq!(cfg.alert_cpu_percent, 90.0);
        let t = cfg.alert_thresholds();
        assert_eq!(t.cpu_percent, 90.0);
        cfg.validate().unwrap();

        let mut bad = SmosConfig::default();
        bad.alert_cpu_percent = 150.0;
        assert!(bad.validate().is_err());
        bad.alert_cpu_percent = 80.0;
        bad.alert_disk_percent = -5.0;
        assert!(bad.validate().is_err());
    }

    #[test]
    fn apply_update_alert_thresholds() {
        let mut cfg = SmosConfig::default();
        cfg.apply_update(ConfigUpdate {
            alert_cpu_percent: Some(75.5),
            alert_memory_percent: Some(85.0),
            alert_disk_percent: Some(95.0),
            ..Default::default()
        })
        .unwrap();
        assert_eq!(cfg.alert_cpu_percent, 75.5);
        assert_eq!(cfg.alert_memory_percent, 85.0);
        assert_eq!(cfg.alert_disk_percent, 95.0);
        let view = cfg.public_view();
        assert_eq!(view.alert_cpu_percent, 75.5);
    }

    #[test]
    fn apply_update_is_atomic_on_validation_failure() {
        let mut cfg = SmosConfig::default();
        let before = cfg.clone();
        cfg.host_label = "original-label".into();
        cfg.log_tail_lines = 200;
        let snapshot = cfg.clone();

        let err = cfg
            .apply_update(ConfigUpdate {
                host_label: Some("would-be-changed".into()),
                log_tail_lines: Some(0), // invalid
                ..Default::default()
            })
            .unwrap_err();
        assert!(err.to_string().contains("log_tail_lines"));
        // No field from the rejected update may stick.
        assert_eq!(cfg.host_label, snapshot.host_label);
        assert_eq!(cfg.log_tail_lines, snapshot.log_tail_lines);
        assert_eq!(cfg, snapshot);
        assert_ne!(cfg.host_label, "would-be-changed");
        let _ = before;
    }
}
