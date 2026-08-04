//! Process listing, query (filter/sort), and lifecycle actions.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sysinfo::{Pid, ProcessesToUpdate, Signal, System};
use thiserror::Error;

#[derive(Debug, Clone, Serialize)]
pub struct ProcessInfo {
    pub pid: u32,
    pub name: String,
    pub cmd: String,
    pub exe: String,
    pub user: String,
    pub cpu_usage: f32,
    pub memory_bytes: u64,
    pub status: String,
    pub start_time: u64,
    pub parent_pid: Option<u32>,
}

/// Query parameters for filtering and sorting the process list.
#[derive(Debug, Clone, Default, Deserialize, Serialize, PartialEq, Eq)]
pub struct ProcessQuery {
    /// Case-insensitive substring match against process name, cmd, or exe.
    pub name: Option<String>,
    /// Exact PID match when set.
    pub pid: Option<u32>,
    /// Sort field (default: cpu).
    pub sort: Option<ProcessSort>,
    /// Sort direction (default: desc for cpu/memory, asc for name/pid).
    pub order: Option<SortOrder>,
    /// Max results after filter/sort (default: no cap beyond full list).
    pub limit: Option<usize>,
}

#[derive(Debug, Clone, Copy, Default, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessSort {
    #[default]
    Cpu,
    Memory,
    Name,
    Pid,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum SortOrder {
    Asc,
    Desc,
}

#[derive(Debug, Clone, Deserialize)]
pub struct ProcessActionRequest {
    pub action: ProcessAction,
}

#[derive(Debug, Clone, Copy, Deserialize, Serialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ProcessAction {
    /// Request graceful termination (SIGTERM on Unix; TerminateProcess best-effort elsewhere).
    Terminate,
    /// Force kill (SIGKILL on Unix).
    Kill,
}

#[derive(Debug, Clone, Serialize)]
pub struct ProcessActionResult {
    pub pid: u32,
    pub action: ProcessAction,
    pub success: bool,
    pub message: String,
    pub timestamp: DateTime<Utc>,
}

#[derive(Debug, Error)]
pub enum ProcessError {
    #[error("process {0} not found")]
    NotFound(u32),
    #[error("refusing to act on SMOS self process (pid {0})")]
    SelfProcess(u32),
    #[error("action failed for pid {pid}: {message}")]
    ActionFailed { pid: u32, message: String },
}

/// List running processes with identity and resource fields (sorted by CPU desc).
pub fn list_processes() -> Vec<ProcessInfo> {
    query_processes(&ProcessQuery::default())
}

/// Live process list with optional name/pid filter and sort.
pub fn query_processes(query: &ProcessQuery) -> Vec<ProcessInfo> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    // CPU usage needs a second sample for meaningful values
    std::thread::sleep(std::time::Duration::from_millis(150));
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let list: Vec<ProcessInfo> = sys
        .processes()
        .iter()
        .map(|(pid, p)| ProcessInfo {
            pid: pid.as_u32(),
            name: p.name().to_string_lossy().to_string(),
            cmd: p
                .cmd()
                .iter()
                .map(|s| s.to_string_lossy().to_string())
                .collect::<Vec<_>>()
                .join(" "),
            exe: p
                .exe()
                .map(|path| path.to_string_lossy().to_string())
                .unwrap_or_default(),
            user: p
                .user_id()
                .map(|u| u.to_string())
                .unwrap_or_else(|| "-".into()),
            cpu_usage: p.cpu_usage(),
            memory_bytes: p.memory(),
            status: format!("{:?}", p.status()),
            start_time: p.start_time(),
            parent_pid: p.parent().map(|p| p.as_u32()),
        })
        .collect();

    apply_process_query(list, query)
}

/// Pure filter/sort/limit applied to a process list (unit-testable without live host scan).
pub fn apply_process_query(mut list: Vec<ProcessInfo>, query: &ProcessQuery) -> Vec<ProcessInfo> {
    if let Some(pid) = query.pid {
        list.retain(|p| p.pid == pid);
    }
    if let Some(ref name) = query.name {
        let needle = name.trim().to_lowercase();
        if !needle.is_empty() {
            list.retain(|p| {
                p.name.to_lowercase().contains(&needle)
                    || p.cmd.to_lowercase().contains(&needle)
                    || p.exe.to_lowercase().contains(&needle)
            });
        }
    }

    let sort = query.sort.unwrap_or(ProcessSort::Cpu);
    let order = query.order.unwrap_or(match sort {
        ProcessSort::Cpu | ProcessSort::Memory => SortOrder::Desc,
        ProcessSort::Name | ProcessSort::Pid => SortOrder::Asc,
    });
    let desc = matches!(order, SortOrder::Desc);

    list.sort_by(|a, b| {
        let ord = match sort {
            ProcessSort::Cpu => a
                .cpu_usage
                .partial_cmp(&b.cpu_usage)
                .unwrap_or(std::cmp::Ordering::Equal),
            ProcessSort::Memory => a.memory_bytes.cmp(&b.memory_bytes),
            ProcessSort::Name => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
            ProcessSort::Pid => a.pid.cmp(&b.pid),
        };
        if desc {
            ord.reverse()
        } else {
            ord
        }
    });

    if let Some(limit) = query.limit {
        if limit < list.len() {
            list.truncate(limit);
        }
    }
    list
}

/// Perform a lifecycle action on a process.
pub fn act_on_process(pid: u32, action: ProcessAction) -> Result<ProcessActionResult, ProcessError> {
    let self_pid = std::process::id();
    if pid == self_pid {
        return Err(ProcessError::SelfProcess(pid));
    }

    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let target = Pid::from_u32(pid);
    let Some(proc_) = sys.process(target) else {
        return Err(ProcessError::NotFound(pid));
    };

    let signal = match action {
        ProcessAction::Terminate => Signal::Term,
        ProcessAction::Kill => Signal::Kill,
    };

    // On Windows, kill()/terminate may use different underlying APIs;
    // sysinfo maps Signal appropriately where supported.
    let ok = proc_.kill_with(signal).unwrap_or_else(|| proc_.kill());

    if ok {
        Ok(ProcessActionResult {
            pid,
            action,
            success: true,
            message: format!("signal {:?} delivered to pid {pid}", signal),
            timestamp: Utc::now(),
        })
    } else {
        // Fallback: try plain kill
        if proc_.kill() {
            Ok(ProcessActionResult {
                pid,
                action,
                success: true,
                message: format!("kill delivered to pid {pid}"),
                timestamp: Utc::now(),
            })
        } else {
            Err(ProcessError::ActionFailed {
                pid,
                message: "OS refused process signal/kill (permission or unsupported)".into(),
            })
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample(pid: u32, name: &str, cpu: f32, mem: u64) -> ProcessInfo {
        ProcessInfo {
            pid,
            name: name.into(),
            cmd: format!("/usr/bin/{name}"),
            exe: format!("/usr/bin/{name}"),
            user: "root".into(),
            cpu_usage: cpu,
            memory_bytes: mem,
            status: "Run".into(),
            start_time: 0,
            parent_pid: None,
        }
    }

    #[test]
    fn list_processes_non_empty_on_running_host() {
        let procs = list_processes();
        assert!(
            !procs.is_empty(),
            "a normal OS should have at least one process"
        );
        let self_pid = std::process::id();
        assert!(
            procs.iter().any(|p| p.pid == self_pid) || procs.iter().any(|p| !p.name.is_empty()),
            "process entries should carry identity fields"
        );
        // Every entry should have a name or pid
        for p in procs.iter().take(20) {
            assert!(p.pid > 0);
            assert!(!p.name.is_empty() || !p.cmd.is_empty());
        }
    }

    #[test]
    fn apply_filter_by_name_reduces_set() {
        let list = vec![
            sample(1, "nginx", 10.0, 100),
            sample(2, "postgres", 5.0, 200),
            sample(3, "smos", 2.0, 50),
            sample(4, "NGINX-worker", 8.0, 80),
        ];
        let filtered = apply_process_query(
            list.clone(),
            &ProcessQuery {
                name: Some("nginx".into()),
                ..Default::default()
            },
        );
        assert_eq!(filtered.len(), 2, "case-insensitive name filter");
        assert!(filtered.iter().all(|p| p.name.to_lowercase().contains("nginx")));
        assert!(filtered.len() < list.len());
    }

    #[test]
    fn apply_filter_by_pid_exact() {
        let list = vec![
            sample(10, "a", 1.0, 1),
            sample(20, "b", 2.0, 2),
            sample(30, "c", 3.0, 3),
        ];
        let filtered = apply_process_query(
            list,
            &ProcessQuery {
                pid: Some(20),
                ..Default::default()
            },
        );
        assert_eq!(filtered.len(), 1);
        assert_eq!(filtered[0].pid, 20);
        assert_eq!(filtered[0].name, "b");
    }

    #[test]
    fn apply_sort_by_cpu_desc_and_memory_asc() {
        let list = vec![
            sample(1, "low", 1.0, 300),
            sample(2, "high", 50.0, 100),
            sample(3, "mid", 10.0, 200),
        ];
        let by_cpu = apply_process_query(
            list.clone(),
            &ProcessQuery {
                sort: Some(ProcessSort::Cpu),
                order: Some(SortOrder::Desc),
                ..Default::default()
            },
        );
        assert_eq!(by_cpu[0].name, "high");
        assert_eq!(by_cpu[1].name, "mid");
        assert_eq!(by_cpu[2].name, "low");
        for w in by_cpu.windows(2) {
            assert!(w[0].cpu_usage >= w[1].cpu_usage);
        }

        let by_mem = apply_process_query(
            list,
            &ProcessQuery {
                sort: Some(ProcessSort::Memory),
                order: Some(SortOrder::Asc),
                ..Default::default()
            },
        );
        assert_eq!(by_mem[0].memory_bytes, 100);
        assert_eq!(by_mem[2].memory_bytes, 300);
        for w in by_mem.windows(2) {
            assert!(w[0].memory_bytes <= w[1].memory_bytes);
        }
    }

    #[test]
    fn apply_limit_truncates_after_sort() {
        let list = vec![
            sample(1, "a", 30.0, 1),
            sample(2, "b", 20.0, 1),
            sample(3, "c", 10.0, 1),
        ];
        let top = apply_process_query(
            list,
            &ProcessQuery {
                sort: Some(ProcessSort::Cpu),
                order: Some(SortOrder::Desc),
                limit: Some(2),
                ..Default::default()
            },
        );
        assert_eq!(top.len(), 2);
        assert_eq!(top[0].name, "a");
        assert_eq!(top[1].name, "b");
    }

    #[test]
    fn live_query_name_filter_matches_self_binary_or_empty() {
        // Drive the real list + filter path on the live host.
        let all = list_processes();
        assert!(!all.is_empty());
        // Filter by a fragment of our own process name when present.
        let self_pid = std::process::id();
        let me = all.iter().find(|p| p.pid == self_pid);
        if let Some(me) = me {
            let needle = me.name.chars().take(3).collect::<String>();
            if !needle.is_empty() {
                let q = ProcessQuery {
                    name: Some(needle.clone()),
                    ..Default::default()
                };
                let filtered = query_processes(&q);
                assert!(
                    !filtered.is_empty(),
                    "name filter {needle:?} should match at least self"
                );
                assert!(
                    filtered.iter().any(|p| p.pid == self_pid)
                        || filtered
                            .iter()
                            .any(|p| p.name.to_lowercase().contains(&needle.to_lowercase())),
                    "filtered set must relate to needle"
                );
                assert!(filtered.len() <= all.len());
            }
        }
        // Exact pid filter for self
        let only = query_processes(&ProcessQuery {
            pid: Some(self_pid),
            ..Default::default()
        });
        // Self may or may not appear depending on refresh timing; if present must be exact.
        for p in &only {
            assert_eq!(p.pid, self_pid);
        }
    }

    #[test]
    fn refuse_self_kill() {
        let self_pid = std::process::id();
        let err = act_on_process(self_pid, ProcessAction::Kill).unwrap_err();
        match err {
            ProcessError::SelfProcess(p) => assert_eq!(p, self_pid),
            other => panic!("expected SelfProcess, got {other}"),
        }
    }

    #[test]
    fn not_found_for_impossible_pid() {
        // PID 0 is typically not a killable userspace process; use a huge pid.
        let err = act_on_process(u32::MAX - 7, ProcessAction::Terminate).unwrap_err();
        match err {
            ProcessError::NotFound(_) => {}
            other => panic!("expected NotFound, got {other}"),
        }
    }
}
