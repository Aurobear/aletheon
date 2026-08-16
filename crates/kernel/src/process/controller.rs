//! Kernel-owned process controller for external delegate leases.
//!
//! Adapters provide a validated, argv-preserving [`::contracts::SandboxCommand`]
//! and trusted working directory. The kernel owns the OS child/process-group
//! boundary and the bounded termination/reap operation; protocol adapters never
//! construct or signal an untracked process directly.

use ::contracts::RuntimeProcessId;
use ::contracts::SandboxCommand;
use ::contracts::{AgentControlError, AgentControlErrorKind};
use async_trait::async_trait;
use runtime::{RuntimeProcessReclaimOutcome, RuntimeProcessSupervisor};
use std::io;
use std::path::Path;
use tokio::process::{Child, Command};

pub struct ManagedProcess {
    pub child: Child,
    pub process_group: u32,
}

/// Result of reconciling a durable external child after a daemon restart.
/// PID is only actionable when its kernel start-time identity still matches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ReclaimOutcome {
    AlreadyExited,
    IdentityReused,
    Reclaimed,
}

#[async_trait]
pub trait ProcessController: Send + Sync {
    async fn spawn(&self, command: SandboxCommand, cwd: &Path) -> anyhow::Result<ManagedProcess>;

    async fn terminate(&self, process_group: u32, child: &mut Child) -> anyhow::Result<()>;

    /// Reconcile a durable process identity without trusting a bare PID.
    /// Implementations that cannot inspect/terminate host processes fail
    /// closed rather than silently claiming recovery.
    async fn reclaim(&self, _identity: RuntimeProcessId) -> anyhow::Result<ReclaimOutcome> {
        anyhow::bail!("process controller does not support durable reclaim")
    }
}

#[derive(Debug, Default)]
pub struct LinuxProcessController;

/// Runtime recovery adapter backed by the kernel-owned Linux process
/// controller. Keeping this adapter beside the controller prevents a second
/// process authority from forming in the composition crate.
#[derive(Debug, Default)]
pub struct LinuxRuntimeProcessSupervisor;

#[async_trait]
impl RuntimeProcessSupervisor for LinuxRuntimeProcessSupervisor {
    async fn reclaim(
        &self,
        identity: RuntimeProcessId,
    ) -> Result<RuntimeProcessReclaimOutcome, AgentControlError> {
        match LinuxProcessController
            .reclaim(identity)
            .await
            .map_err(|error| AgentControlError {
                kind: AgentControlErrorKind::Runtime,
                message: error.to_string(),
            })? {
            ReclaimOutcome::AlreadyExited => Ok(RuntimeProcessReclaimOutcome::AlreadyExited),
            ReclaimOutcome::IdentityReused => Ok(RuntimeProcessReclaimOutcome::IdentityReused),
            ReclaimOutcome::Reclaimed => Ok(RuntimeProcessReclaimOutcome::Reclaimed),
        }
    }
}

#[async_trait]
impl ProcessController for LinuxProcessController {
    async fn spawn(&self, command: SandboxCommand, cwd: &Path) -> anyhow::Result<ManagedProcess> {
        let mut process = Command::new(&command.program);
        process
            .args(&command.args)
            .env_clear()
            .envs(&command.environment)
            .current_dir(cwd)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        process.process_group(0);
        let child = process.spawn()?;
        let process_group = child
            .id()
            .ok_or_else(|| anyhow::anyhow!("managed process lacks an operating-system id"))?;
        Ok(ManagedProcess {
            child,
            process_group,
        })
    }

    async fn terminate(&self, process_group: u32, child: &mut Child) -> anyhow::Result<()> {
        #[cfg(unix)]
        {
            let result = unsafe { libc::kill(-(process_group as i32), libc::SIGKILL) };
            if result != 0 {
                let error = io::Error::last_os_error();
                if error.raw_os_error() != Some(libc::ESRCH) {
                    return Err(anyhow::anyhow!(
                        "terminating managed process group {process_group}: {error}"
                    ));
                }
            }
        }
        #[cfg(not(unix))]
        child.kill().await?;
        child.wait().await?;
        Ok(())
    }

    async fn reclaim(&self, identity: RuntimeProcessId) -> anyhow::Result<ReclaimOutcome> {
        let pid = identity.os_pid.0;
        if pid <= 1 || pid == std::process::id() {
            anyhow::bail!("refusing to signal an unsafe runtime PID");
        }
        match process_state_and_start_time(pid) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(ReclaimOutcome::AlreadyExited)
            }
            Err(error) => anyhow::bail!("reading durable runtime process identity: {error}"),
            Ok((_, actual)) if actual != identity.start_time_ticks => {
                return Ok(ReclaimOutcome::IdentityReused)
            }
            Ok(('Z', _)) => return Ok(ReclaimOutcome::AlreadyExited),
            Ok(_) => {}
        }

        signal_process_group(pid, libc::SIGTERM)?;
        if wait_until_gone_or_reused(&identity, std::time::Duration::from_millis(500)).await? {
            return Ok(ReclaimOutcome::Reclaimed);
        }
        signal_process_group(pid, libc::SIGKILL)?;
        if wait_until_gone_or_reused(&identity, std::time::Duration::from_secs(2)).await? {
            return Ok(ReclaimOutcome::Reclaimed);
        }
        anyhow::bail!("external runtime process survived bounded SIGTERM/SIGKILL reconciliation")
    }
}

/// Read the Linux process start-time ticks used by the durable runtime
/// identity. The value is intentionally not wall-clock time.
pub fn process_start_time_ticks(pid: u32) -> io::Result<u64> {
    process_state_and_start_time(pid).map(|(_, start_time)| start_time)
}

fn signal_process_group(pid: u32, signal: i32) -> io::Result<()> {
    #[cfg(unix)]
    {
        let result = unsafe { libc::kill(-(pid as i32), signal) };
        if result == 0 {
            return Ok(());
        }
        let error = io::Error::last_os_error();
        if error.raw_os_error() == Some(libc::ESRCH) {
            return Ok(());
        }
        Err(error)
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, signal);
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "external runtime process reconciliation requires Unix process groups",
        ))
    }
}

async fn wait_until_gone_or_reused(
    identity: &RuntimeProcessId,
    timeout: std::time::Duration,
) -> io::Result<bool> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match process_state_and_start_time(identity.os_pid.0) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
            Ok((_, actual)) if actual != identity.start_time_ticks => return Ok(true),
            Ok(('Z', _)) => return Ok(true),
            Ok(_) => {}
            Err(error) => return Err(error),
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(std::time::Duration::from_millis(20)).await;
    }
}

fn process_state_and_start_time(pid: u32) -> io::Result<(char, u64)> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let command_end = stat
        .rfind(')')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "process stat lacks command"))?;
    let mut fields = stat[command_end + 1..].split_whitespace();
    let state = fields
        .next()
        .and_then(|value| value.chars().next())
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "process stat lacks state"))?;
    let start_time = fields
        .nth(18)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "process stat lacks start time"))?
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))?;
    Ok((state, start_time))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::BTreeMap;
    use std::path::PathBuf;

    #[cfg(unix)]
    #[tokio::test]
    async fn controller_owns_group_spawn_and_reap() {
        let controller = LinuxProcessController;
        let managed = controller
            .spawn(
                SandboxCommand {
                    program: PathBuf::from("/bin/sh"),
                    args: vec!["-c".into(), "sleep 60".into()],
                    environment: BTreeMap::new(),
                },
                Path::new("/tmp"),
            )
            .await
            .unwrap();
        let ManagedProcess {
            mut child,
            process_group,
        } = managed;
        assert!(process_group > 1);
        controller
            .terminate(process_group, &mut child)
            .await
            .unwrap();
        assert!(child.id().is_none());
    }
}
