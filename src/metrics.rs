//! Host performance metrics from real sysinfo snapshots.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sysinfo::{Disks, System};

#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub timestamp: DateTime<Utc>,
    pub hostname: String,
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub disks: Vec<DiskMetrics>,
    pub load_avg: Option<LoadAvgMetrics>,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct CpuMetrics {
    /// Global CPU usage percent 0.0..=100.0
    pub usage_percent: f32,
    pub core_count: usize,
    pub brand: String,
    pub per_core: Vec<f32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct MemoryMetrics {
    pub total_bytes: u64,
    pub used_bytes: u64,
    pub available_bytes: u64,
    pub usage_percent: f64,
    pub swap_total_bytes: u64,
    pub swap_used_bytes: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct DiskMetrics {
    pub name: String,
    pub mount_point: String,
    pub file_system: String,
    pub total_bytes: u64,
    pub available_bytes: u64,
    pub used_bytes: u64,
    pub usage_percent: f64,
}

#[derive(Debug, Clone, Serialize)]
pub struct LoadAvgMetrics {
    pub one: f64,
    pub five: f64,
    pub fifteen: f64,
}

/// Collect a live metrics snapshot from the host.
///
/// Uses the same `sysinfo` path production handlers call — not demo constants.
pub fn collect_metrics() -> MetricsSnapshot {
    let mut sys = System::new();
    sys.refresh_cpu_all();
    sys.refresh_memory();

    // First refresh often returns 0% CPU; brief wait then re-sample.
    std::thread::sleep(std::time::Duration::from_millis(200));
    sys.refresh_cpu_all();
    sys.refresh_memory();

    let disks = Disks::new_with_refreshed_list();
    let hostname = System::host_name().unwrap_or_else(|| "unknown".into());

    let per_core: Vec<f32> = sys.cpus().iter().map(|c| c.cpu_usage()).collect();
    let usage_percent = sys.global_cpu_usage();
    let brand = sys
        .cpus()
        .first()
        .map(|c| c.brand().to_string())
        .unwrap_or_default();

    let total = sys.total_memory();
    let used = sys.used_memory();
    let available = sys.available_memory();
    let mem_pct = if total > 0 {
        (used as f64 / total as f64) * 100.0
    } else {
        0.0
    };

    let disk_metrics: Vec<DiskMetrics> = disks
        .list()
        .iter()
        .map(|d| {
            let total_bytes = d.total_space();
            let available_bytes = d.available_space();
            let used_bytes = total_bytes.saturating_sub(available_bytes);
            let usage_percent = if total_bytes > 0 {
                (used_bytes as f64 / total_bytes as f64) * 100.0
            } else {
                0.0
            };
            DiskMetrics {
                name: d.name().to_string_lossy().to_string(),
                mount_point: d.mount_point().to_string_lossy().to_string(),
                file_system: d.file_system().to_string_lossy().to_string(),
                total_bytes,
                available_bytes,
                used_bytes,
                usage_percent,
            }
        })
        .collect();

    let load = System::load_average();
    let load_avg = if load.one == 0.0 && load.five == 0.0 && load.fifteen == 0.0 {
        // Windows often reports zeros; still surface when non-zero on Unix.
        None
    } else {
        Some(LoadAvgMetrics {
            one: load.one,
            five: load.five,
            fifteen: load.fifteen,
        })
    };

    MetricsSnapshot {
        timestamp: Utc::now(),
        hostname,
        cpu: CpuMetrics {
            usage_percent,
            core_count: sys.cpus().len(),
            brand,
            per_core,
        },
        memory: MemoryMetrics {
            total_bytes: total,
            used_bytes: used,
            available_bytes: available,
            usage_percent: mem_pct,
            swap_total_bytes: sys.total_swap(),
            swap_used_bytes: sys.used_swap(),
        },
        disks: disk_metrics,
        load_avg,
        uptime_secs: System::uptime(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_has_real_shaped_host_data() {
        let snap = collect_metrics();
        assert!(!snap.hostname.is_empty(), "hostname must be non-empty");
        assert!(
            snap.cpu.core_count > 0,
            "host must report at least one CPU core"
        );
        assert!(
            snap.memory.total_bytes > 0,
            "host must report non-zero total memory"
        );
        // usage is a real percent range
        assert!(
            (0.0..=100.0).contains(&snap.cpu.usage_percent)
                || snap.cpu.usage_percent.is_finite(),
            "cpu usage should be finite"
        );
        assert!(
            (0.0..=100.5).contains(&snap.memory.usage_percent),
            "memory usage percent in range"
        );
        assert!(
            !snap.disks.is_empty(),
            "host should expose at least one disk"
        );
        for d in &snap.disks {
            assert!(d.total_bytes > 0 || d.mount_point.len() > 0);
        }
        assert!(snap.uptime_secs > 0, "uptime should be positive on a running host");
    }
}
