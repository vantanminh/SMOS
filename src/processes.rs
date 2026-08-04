//! Process listing and lifecycle actions.

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

/// List running processes with identity and resource fields.
pub fn list_processes() -> Vec<ProcessInfo> {
    let mut sys = System::new();
    sys.refresh_processes(ProcessesToUpdate::All, true);
    // CPU usage needs a second sample for meaningful values
    std::thread::sleep(std::time::Duration::from_millis(150));
    sys.refresh_processes(ProcessesToUpdate::All, true);

    let mut list: Vec<ProcessInfo> = sys
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

    list.sort_by(|a, b| {
        b.cpu_usage
            .partial_cmp(&a.cpu_usage)
            .unwrap_or(std::cmp::Ordering::Equal)
    });
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
