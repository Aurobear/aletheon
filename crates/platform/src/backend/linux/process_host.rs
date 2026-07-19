//! Linux ProcessHost — real pid/process-group/signal via /proc and kill (H1-02).

use crate::error::{HostError, HostErrorKind};
use crate::process::{ProcessHost, ProcessId, ProcessSignal, ProcessSnapshot, SpawnSpec};
use crate::receipt::HostReceipt;
use async_trait::async_trait;
use std::collections::HashMap;
use std::time::Instant;

pub struct LinuxProcessHost {
    children: tokio::sync::Mutex<HashMap<u32, tokio::process::Child>>,
}

impl Default for LinuxProcessHost {
    fn default() -> Self {
        Self {
            children: tokio::sync::Mutex::new(HashMap::new()),
        }
    }
}

impl LinuxProcessHost {
    pub fn new() -> Self {
        Self::default()
    }
}

#[async_trait]
impl ProcessHost for LinuxProcessHost {
    async fn spawn(&self, spec: SpawnSpec) -> Result<(ProcessId, HostReceipt), HostError> {
        let start = Instant::now();
        let program = spec
            .argv
            .first()
            .ok_or_else(|| HostError::new(HostErrorKind::Conflict("empty argv".into()), "spawn"))?;
        let mut cmd = tokio::process::Command::new(program);
        for arg in &spec.argv[1..] {
            cmd.arg(arg);
        }
        for (k, v) in &spec.env {
            cmd.env(k, v);
        }
        if let Some(wd) = &spec.working_dir {
            cmd.current_dir(wd.native());
        }
        // Every spawned process leads its own process group so tree termination
        // never targets the caller's group and includes descendants reliably.
        cmd.process_group(0);
        if let Some(ms) = spec.timeout_ms {
            cmd.kill_on_drop(true);
            let mut child = cmd
                .spawn()
                .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "spawn"))?;
            let pid = child.id().ok_or_else(|| {
                HostError::new(
                    HostErrorKind::Io("spawned process has no pid".into()),
                    "spawn",
                )
            })?;
            match tokio::time::timeout(std::time::Duration::from_millis(ms), child.wait()).await {
                Ok(Ok(status)) => {
                    let receipt = if status.success() {
                        HostReceipt::ok("spawn", start.elapsed().as_micros() as u64)
                    } else {
                        HostReceipt::err(
                            "spawn",
                            start.elapsed().as_micros() as u64,
                            format!("process exited with {status}"),
                        )
                    };
                    Ok((ProcessId(pid), receipt))
                }
                Ok(Err(error)) => Err(HostError::new(
                    HostErrorKind::Io(error.to_string()),
                    "wait for spawned process",
                )),
                Err(_) => {
                    let _ = signal_target(-(pid as i32), libc::SIGKILL, "spawn timeout");
                    let _ = child.wait().await;
                    Err(HostError::new(
                        HostErrorKind::Timeout(format!("process exceeded {ms} ms")),
                        "spawn timeout",
                    ))
                }
            }
        } else {
            let child = cmd
                .spawn()
                .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "spawn"))?;
            let pid = child.id().ok_or_else(|| {
                HostError::new(
                    HostErrorKind::Io("spawned process has no pid".into()),
                    "spawn",
                )
            })?;
            self.children.lock().await.insert(pid, child);
            Ok((
                ProcessId(pid),
                HostReceipt::ok("spawn", start.elapsed().as_micros() as u64),
            ))
        }
    }

    async fn inspect(&self, id: ProcessId) -> Result<ProcessSnapshot, HostError> {
        let mut children = self.children.lock().await;
        let child = children.get_mut(&id.0).ok_or_else(|| {
            HostError::new(
                HostErrorKind::NotFound(format!("unmanaged process {}", id.0)),
                "inspect",
            )
        })?;
        let status = child
            .try_wait()
            .map_err(|error| HostError::new(HostErrorKind::Io(error.to_string()), "inspect"))?;
        Ok(ProcessSnapshot {
            id,
            running: status.is_none(),
            exit_code: status.and_then(|value| value.code()),
        })
    }

    async fn signal(&self, id: ProcessId, sig: ProcessSignal) -> Result<HostReceipt, HostError> {
        let start = Instant::now();
        let sig_num = match sig {
            ProcessSignal::Interrupt => 2,  // SIGINT
            ProcessSignal::Terminate => 15, // SIGTERM
            ProcessSignal::Kill => 9,       // SIGKILL
        };
        signal_target(checked_pid(id)?, sig_num, "signal")?;
        Ok(HostReceipt::ok(
            "signal",
            start.elapsed().as_micros() as u64,
        ))
    }

    async fn terminate_tree(&self, id: ProcessId, grace_ms: u64) -> Result<HostReceipt, HostError> {
        let start = Instant::now();
        let pid = checked_pid(id)?;
        // Spawn establishes a process group whose id equals the child pid.
        // Signalling the negative id reaches the root and all descendants.
        signal_target(-pid, libc::SIGTERM, "terminate_tree")?;
        tokio::time::sleep(std::time::Duration::from_millis(grace_ms)).await;
        if unsafe { libc::kill(-pid, libc::SIGKILL) } != 0 {
            let error = std::io::Error::last_os_error();
            if error.raw_os_error() != Some(libc::ESRCH) {
                return Err(map_signal_error(error, "terminate_tree"));
            }
        }
        if let Some(mut child) = self.children.lock().await.remove(&id.0) {
            let _ = child.wait().await;
        }
        Ok(HostReceipt::ok(
            "terminate_tree",
            start.elapsed().as_micros() as u64,
        ))
    }
}

fn checked_pid(id: ProcessId) -> Result<i32, HostError> {
    if id.0 == 0 || id.0 > i32::MAX as u32 {
        return Err(HostError::new(
            HostErrorKind::NotFound(format!("invalid process id {}", id.0)),
            "process id must identify one child",
        ));
    }
    Ok(id.0 as i32)
}

fn signal_target(target: i32, signal: i32, operation: &str) -> Result<(), HostError> {
    if unsafe { libc::kill(target, signal) } == 0 {
        Ok(())
    } else {
        Err(map_signal_error(std::io::Error::last_os_error(), operation))
    }
}

fn map_signal_error(error: std::io::Error, operation: &str) -> HostError {
    let kind = match error.raw_os_error() {
        Some(libc::ESRCH) => HostErrorKind::NotFound(error.to_string()),
        Some(libc::EPERM) | Some(libc::EACCES) => {
            HostErrorKind::PermissionDenied(error.to_string())
        }
        _ => HostErrorKind::Io(error.to_string()),
    };
    HostError::new(kind, operation)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn spawn_and_signal_child() {
        let host = LinuxProcessHost::new();
        let (pid, receipt) = host
            .spawn(SpawnSpec {
                argv: vec!["sleep".into(), "1".into()],
                env: vec![],
                working_dir: None,
                timeout_ms: None,
            })
            .await
            .unwrap();
        assert!(receipt.success);
        assert!(
            pid.0 > 0,
            "spawn must return the child pid, never process group 0"
        );
        let snap = host.inspect(pid).await.unwrap();
        assert!(snap.running);
        host.signal(pid, ProcessSignal::Kill).await.unwrap();
    }
}
