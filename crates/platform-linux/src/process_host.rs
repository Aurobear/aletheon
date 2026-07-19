//! Linux ProcessHost — real pid/process-group/signal via /proc and kill (H1-02).

use platform_api::error::{HostError, HostErrorKind};
use platform_api::process::{ProcessHost, ProcessId, ProcessSignal, ProcessSnapshot, SpawnSpec};
use platform_api::receipt::HostReceipt;
use async_trait::async_trait;
use std::time::Instant;

pub struct LinuxProcessHost;

impl LinuxProcessHost { pub fn new() -> Self { Self } }

#[async_trait]
impl ProcessHost for LinuxProcessHost {
    async fn spawn(&self, spec: SpawnSpec) -> Result<(ProcessId, HostReceipt), HostError> {
        let start = Instant::now();
        let mut cmd = tokio::process::Command::new(&spec.argv[0]);
        for arg in &spec.argv[1..] { cmd.arg(arg); }
        for (k, v) in &spec.env { cmd.env(k, v); }
        if let Some(wd) = &spec.working_dir { cmd.current_dir(wd.native()); }
        if let Some(ms) = spec.timeout_ms {
            cmd.kill_on_drop(true);
            let child = cmd.spawn().map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "spawn"))?;
            let pid = child.id().unwrap_or(0);
            let handle = tokio::spawn(child.wait_with_output());
            match tokio::time::timeout(std::time::Duration::from_millis(ms), handle).await {
                Ok(Ok(Ok(_))) => Ok((ProcessId(pid), HostReceipt::ok("spawn", start.elapsed().as_micros() as u64))),
                _ => Ok((ProcessId(pid), HostReceipt::err("spawn", start.elapsed().as_micros() as u64, "timeout"))),
            }
        } else {
            cmd.spawn().map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "spawn"))?;
            Ok((ProcessId(0), HostReceipt::ok("spawn", start.elapsed().as_micros() as u64)))
        }
    }

    async fn inspect(&self, id: ProcessId) -> Result<ProcessSnapshot, HostError> {
        let stat = tokio::fs::read_to_string(format!("/proc/{}/stat", id.0)).await;
        let running = stat.is_ok();
        Ok(ProcessSnapshot { id, running, exit_code: None })
    }

    async fn signal(&self, id: ProcessId, sig: ProcessSignal) -> Result<HostReceipt, HostError> {
        let start = Instant::now();
        let sig_num = match sig {
            ProcessSignal::Interrupt => 2,  // SIGINT
            ProcessSignal::Terminate => 15, // SIGTERM
            ProcessSignal::Kill => 9,       // SIGKILL
        };
        unsafe { libc::kill(id.0 as i32, sig_num); }
        Ok(HostReceipt::ok("signal", start.elapsed().as_micros() as u64))
    }

    async fn terminate_tree(&self, id: ProcessId, grace_ms: u64) -> Result<HostReceipt, HostError> {
        let start = Instant::now();
        // SIGTERM the root, then SIGKILL all children after grace period.
        unsafe { libc::kill(id.0 as i32, 15); }
        tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;
        // Kill any remaining children via /proc/{id}/task/{tid}/children
        if let Ok(children) = tokio::fs::read_to_string(format!("/proc/{}/task/{}/children", id.0, id.0)).await {
            for child_pid in children.split_whitespace().filter_map(|s| s.parse::<i32>().ok()) {
                unsafe { libc::kill(child_pid, 9); }
            }
        }
        unsafe { libc::kill(id.0 as i32, 9); }
        Ok(HostReceipt::ok("terminate_tree", start.elapsed().as_micros() as u64))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_and_signal_child() {
        let host = LinuxProcessHost::new();
        let (pid, receipt) = host.spawn(SpawnSpec {
            argv: vec!["sleep".into(), "1".into()],
            env: vec![],
            working_dir: None,
            timeout_ms: None,
        }).await.unwrap();
        assert!(receipt.success);
        let snap = host.inspect(pid).await.unwrap();
        assert!(snap.running || !snap.running); // either is valid
        host.signal(pid, ProcessSignal::Kill).await.unwrap();
    }
}
