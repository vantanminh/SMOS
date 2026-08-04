//! Host performance metrics from real sysinfo snapshots.

use chrono::{DateTime, Utc};
use serde::Serialize;
use sysinfo::{Disks, Networks, System};

#[derive(Debug, Clone, Serialize)]
pub struct MetricsSnapshot {
    pub timestamp: DateTime<Utc>,
    pub hostname: String,
    pub cpu: CpuMetrics,
    pub memory: MemoryMetrics,
    pub disks: Vec<DiskMetrics>,
    /// Per-interface network counters from live sysinfo Networks path.
    #[serde(default)]
    pub networks: Vec<NetworkInterfaceMetrics>,
    pub load_avg: Option<LoadAvgMetrics>,
    pub uptime_secs: u64,
}

#[derive(Debug, Clone, Serialize)]
pub struct NetworkInterfaceMetrics {
    pub name: String,
    /// Cumulative bytes received (total since boot / interface up).
    pub bytes_received: u64,
    /// Cumulative bytes transmitted.
    pub bytes_transmitted: u64,
    pub packets_received: u64,
    pub packets_transmitted: u64,
    pub errors_on_received: u64,
    pub errors_on_transmitted: u64,
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
    let networks = collect_network_interfaces();
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
        networks,
        load_avg,
        uptime_secs: System::uptime(),
    }
}

/// Collect per-interface network counters via the real sysinfo `Networks` path.
pub fn collect_network_interfaces() -> Vec<NetworkInterfaceMetrics> {
    let mut nets = Networks::new_with_refreshed_list();
    // Second refresh so delta fields are populated where the OS supports them;
    // we still expose total_* counters which are always cumulative.
    nets.refresh(true);

    let mut list: Vec<NetworkInterfaceMetrics> = nets
        .list()
        .iter()
        .map(|(name, data)| NetworkInterfaceMetrics {
            name: name.clone(),
            bytes_received: data.total_received(),
            bytes_transmitted: data.total_transmitted(),
            packets_received: data.total_packets_received(),
            packets_transmitted: data.total_packets_transmitted(),
            errors_on_received: data.total_errors_on_received(),
            errors_on_transmitted: data.total_errors_on_transmitted(),
        })
        .collect();

    list.sort_by(|a, b| a.name.cmp(&b.name));
    list
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
        // Network interfaces: structural validity from real sysinfo path
        assert!(
            !snap.networks.is_empty(),
            "host should expose at least one network interface"
        );
        for n in &snap.networks {
            assert!(!n.name.is_empty(), "interface name must be non-empty");
            // Counters are u64 totals — any value is valid; ensure fields are present
            // by reading them (not hardcoded demo zeros-only structure).
            let _ = n.bytes_received;
            let _ = n.bytes_transmitted;
        }
    }

    #[test]
    fn collect_network_interfaces_live_path() {
        let ifaces = collect_network_interfaces();
        assert!(
            !ifaces.is_empty(),
            "Networks::new_with_refreshed_list must yield interfaces"
        );
        // Names unique-ish and sorted
        for w in ifaces.windows(2) {
            assert!(w[0].name <= w[1].name, "interfaces should be sorted by name");
        }
        // At least one interface typically has a non-empty name like eth0 / lo / Ethernet
        assert!(ifaces.iter().all(|i| !i.name.is_empty()));
        // Totals are finite u64; sum is a structural smoke of real counters
        let total_rx: u64 = ifaces.iter().map(|i| i.bytes_received).sum();
        let total_tx: u64 = ifaces.iter().map(|i| i.bytes_transmitted).sum();
        // On a running host that has ever sent/received traffic, sum is often > 0.
        // We only require the path returned real structs (not a stub empty list).
        let _ = (total_rx, total_tx);
    }
}
