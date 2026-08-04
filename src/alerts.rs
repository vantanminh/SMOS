//! Alert threshold evaluation against live metrics snapshots.

use crate::metrics::MetricsSnapshot;
use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// CPU / memory / disk usage thresholds as percents (0.0..=100.0).
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq)]
pub struct AlertThresholds {
    pub cpu_percent: f64,
    pub memory_percent: f64,
    pub disk_percent: f64,
}

impl Default for AlertThresholds {
    fn default() -> Self {
        Self {
            cpu_percent: default_alert_cpu_percent(),
            memory_percent: default_alert_memory_percent(),
            disk_percent: default_alert_disk_percent(),
        }
    }
}

pub fn default_alert_cpu_percent() -> f64 {
    90.0
}
pub fn default_alert_memory_percent() -> f64 {
    90.0
}
pub fn default_alert_disk_percent() -> f64 {
    90.0
}

/// One metric compared to its threshold.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct ThresholdBreach {
    pub metric: String,
    /// Optional scope (e.g. disk mount path).
    #[serde(skip_serializing_if = "Option::is_none")]
    pub scope: Option<String>,
    pub current: f64,
    pub threshold: f64,
    pub breached: bool,
}

/// Full alert status derived from a metrics snapshot + thresholds.
#[derive(Debug, Clone, Serialize, PartialEq)]
pub struct AlertStatus {
    pub timestamp: DateTime<Utc>,
    pub thresholds: AlertThresholds,
    pub cpu: ThresholdBreach,
    pub memory: ThresholdBreach,
    pub disks: Vec<ThresholdBreach>,
    /// True if any CPU, memory, or disk threshold is exceeded.
    pub any_breached: bool,
    pub breach_count: usize,
}

/// Evaluate breaches using pure comparison logic (unit-testable without live host).
pub fn evaluate_alerts(snap: &MetricsSnapshot, thresholds: &AlertThresholds) -> AlertStatus {
    let cpu = ThresholdBreach {
        metric: "cpu".into(),
        scope: None,
        current: snap.cpu.usage_percent as f64,
        threshold: thresholds.cpu_percent,
        breached: (snap.cpu.usage_percent as f64) >= thresholds.cpu_percent,
    };
    let memory = ThresholdBreach {
        metric: "memory".into(),
        scope: None,
        current: snap.memory.usage_percent,
        threshold: thresholds.memory_percent,
        breached: snap.memory.usage_percent >= thresholds.memory_percent,
    };
    let disks: Vec<ThresholdBreach> = snap
        .disks
        .iter()
        .map(|d| ThresholdBreach {
            metric: "disk".into(),
            scope: Some(d.mount_point.clone()),
            current: d.usage_percent,
            threshold: thresholds.disk_percent,
            breached: d.usage_percent >= thresholds.disk_percent,
        })
        .collect();

    let mut breach_count = 0usize;
    if cpu.breached {
        breach_count += 1;
    }
    if memory.breached {
        breach_count += 1;
    }
    breach_count += disks.iter().filter(|d| d.breached).count();

    AlertStatus {
        timestamp: snap.timestamp,
        thresholds: *thresholds,
        cpu,
        memory,
        disks,
        any_breached: breach_count > 0,
        breach_count,
    }
}

/// Convenience: collect live metrics then evaluate.
pub fn collect_alert_status(thresholds: &AlertThresholds) -> AlertStatus {
    let snap = crate::metrics::collect_metrics();
    evaluate_alerts(&snap, thresholds)
}

/// Validate threshold percents are in range.
pub fn validate_thresholds(t: &AlertThresholds) -> Result<(), String> {
    for (name, v) in [
        ("alert_cpu_percent", t.cpu_percent),
        ("alert_memory_percent", t.memory_percent),
        ("alert_disk_percent", t.disk_percent),
    ] {
        if !(0.0..=100.0).contains(&v) || !v.is_finite() {
            return Err(format!("{name} must be between 0 and 100"));
        }
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::metrics::{
        CpuMetrics, DiskMetrics, MemoryMetrics, MetricsSnapshot, NetworkInterfaceMetrics,
    };
    use chrono::Utc;

    fn snap(cpu: f32, mem: f64, disks: Vec<(String, f64)>) -> MetricsSnapshot {
        MetricsSnapshot {
            timestamp: Utc::now(),
            hostname: "test".into(),
            cpu: CpuMetrics {
                usage_percent: cpu,
                core_count: 4,
                brand: "test".into(),
                per_core: vec![cpu],
            },
            memory: MemoryMetrics {
                total_bytes: 1000,
                used_bytes: (mem * 10.0) as u64,
                available_bytes: 100,
                usage_percent: mem,
                swap_total_bytes: 0,
                swap_used_bytes: 0,
            },
            disks: disks
                .into_iter()
                .map(|(mp, pct)| DiskMetrics {
                    name: mp.clone(),
                    mount_point: mp,
                    file_system: "ext4".into(),
                    total_bytes: 1000,
                    available_bytes: 100,
                    used_bytes: 900,
                    usage_percent: pct,
                })
                .collect(),
            networks: vec![NetworkInterfaceMetrics {
                name: "lo".into(),
                bytes_received: 0,
                bytes_transmitted: 0,
                packets_received: 0,
                packets_transmitted: 0,
                errors_on_received: 0,
                errors_on_transmitted: 0,
            }],
            load_avg: None,
            uptime_secs: 1,
        }
    }

    #[test]
    fn no_breach_when_below_thresholds() {
        let s = snap(10.0, 20.0, vec![("/".into(), 30.0)]);
        let t = AlertThresholds {
            cpu_percent: 90.0,
            memory_percent: 90.0,
            disk_percent: 90.0,
        };
        let st = evaluate_alerts(&s, &t);
        assert!(!st.any_breached);
        assert_eq!(st.breach_count, 0);
        assert!(!st.cpu.breached);
        assert!(!st.memory.breached);
        assert!(st.disks.iter().all(|d| !d.breached));
    }

    #[test]
    fn cpu_breach_when_at_or_above_threshold() {
        let s = snap(95.0, 10.0, vec![]);
        let t = AlertThresholds {
            cpu_percent: 90.0,
            memory_percent: 90.0,
            disk_percent: 90.0,
        };
        let st = evaluate_alerts(&s, &t);
        assert!(st.cpu.breached);
        assert!(!st.memory.breached);
        assert!(st.any_breached);
        assert_eq!(st.breach_count, 1);
        assert!((st.cpu.current - 95.0).abs() < 0.01);
        assert!((st.cpu.threshold - 90.0).abs() < 0.01);
    }

    #[test]
    fn memory_and_disk_breaches() {
        let s = snap(
            5.0,
            92.0,
            vec![("/".into(), 91.0), ("/data".into(), 50.0)],
        );
        let t = AlertThresholds {
            cpu_percent: 90.0,
            memory_percent: 90.0,
            disk_percent: 90.0,
        };
        let st = evaluate_alerts(&s, &t);
        assert!(!st.cpu.breached);
        assert!(st.memory.breached);
        assert_eq!(st.disks.len(), 2);
        assert!(st.disks[0].breached);
        assert!(!st.disks[1].breached);
        assert_eq!(st.disks[0].scope.as_deref(), Some("/"));
        assert_eq!(st.breach_count, 2);
        assert!(st.any_breached);
    }

    #[test]
    fn exact_threshold_counts_as_breach() {
        let s = snap(90.0, 90.0, vec![("/".into(), 90.0)]);
        let t = AlertThresholds::default();
        let st = evaluate_alerts(&s, &t);
        assert!(st.cpu.breached);
        assert!(st.memory.breached);
        assert!(st.disks[0].breached);
        assert_eq!(st.breach_count, 3);
    }

    #[test]
    fn validate_thresholds_range() {
        assert!(validate_thresholds(&AlertThresholds::default()).is_ok());
        assert!(validate_thresholds(&AlertThresholds {
            cpu_percent: -1.0,
            ..AlertThresholds::default()
        })
        .is_err());
        assert!(validate_thresholds(&AlertThresholds {
            memory_percent: 101.0,
            ..AlertThresholds::default()
        })
        .is_err());
    }

    #[test]
    fn live_collect_alert_status_shaped() {
        let st = collect_alert_status(&AlertThresholds::default());
        assert!(st.cpu.current.is_finite());
        assert!(st.memory.current.is_finite());
        assert!((0.0..=100.0).contains(&st.thresholds.cpu_percent));
        // disks come from real snapshot
        for d in &st.disks {
            assert_eq!(d.metric, "disk");
            assert!(d.scope.is_some());
        }
    }
}
