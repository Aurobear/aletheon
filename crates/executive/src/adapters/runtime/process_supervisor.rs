//! Linux process-generation reconciliation for durable external runtimes.

use std::io;
use std::time::Duration;

use async_trait::async_trait;
use fabric::{AgentControlError, AgentControlErrorKind, RuntimeProcessId};

use crate::application::agent_control::{RuntimeProcessReclaimOutcome, RuntimeProcessSupervisor};

#[derive(Debug, Default)]
pub struct LinuxRuntimeProcessSupervisor;

#[async_trait]
impl RuntimeProcessSupervisor for LinuxRuntimeProcessSupervisor {
    async fn reclaim(
        &self,
        identity: RuntimeProcessId,
    ) -> Result<RuntimeProcessReclaimOutcome, AgentControlError> {
        let pid = identity.os_pid.0;
        if pid <= 1 || pid == std::process::id() {
            return Err(runtime_error("refusing to signal an unsafe runtime PID"));
        }
        match process_state_and_start_time(pid) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => {
                return Ok(RuntimeProcessReclaimOutcome::AlreadyExited);
            }
            Err(error) => {
                return Err(runtime_error(format!(
                    "reading durable runtime process identity: {error}"
                )));
            }
            Ok((_, actual)) if actual != identity.start_time_ticks => {
                return Ok(RuntimeProcessReclaimOutcome::IdentityReused);
            }
            Ok(('Z', _)) => return Ok(RuntimeProcessReclaimOutcome::AlreadyExited),
            Ok(_) => {}
        }

        signal_process_group(pid, libc::SIGTERM)?;
        if wait_until_gone_or_reused(&identity, Duration::from_millis(500)).await? {
            return Ok(RuntimeProcessReclaimOutcome::Reclaimed);
        }
        signal_process_group(pid, libc::SIGKILL)?;
        if wait_until_gone_or_reused(&identity, Duration::from_secs(2)).await? {
            return Ok(RuntimeProcessReclaimOutcome::Reclaimed);
        }
        Err(runtime_error(
            "external runtime process survived bounded SIGTERM/SIGKILL reconciliation",
        ))
    }
}

fn signal_process_group(pid: u32, signal: i32) -> Result<(), AgentControlError> {
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
        Err(runtime_error(format!(
            "signaling external runtime process group {pid}: {error}"
        )))
    }
    #[cfg(not(unix))]
    {
        let _ = (pid, signal);
        Err(runtime_error(
            "external runtime process reconciliation requires Unix process groups",
        ))
    }
}

async fn wait_until_gone_or_reused(
    identity: &RuntimeProcessId,
    timeout: Duration,
) -> Result<bool, AgentControlError> {
    let deadline = tokio::time::Instant::now() + timeout;
    loop {
        match process_state_and_start_time(identity.os_pid.0) {
            Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(true),
            Ok((_, actual)) if actual != identity.start_time_ticks => return Ok(true),
            Ok(('Z', _)) => return Ok(true),
            Ok(_) => {}
            Err(error) => {
                return Err(runtime_error(format!(
                    "checking external runtime process exit: {error}"
                )));
            }
        }
        if tokio::time::Instant::now() >= deadline {
            return Ok(false);
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

pub(crate) fn process_start_time_ticks(pid: u32) -> io::Result<u64> {
    process_state_and_start_time(pid).map(|(_, start_time)| start_time)
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

fn runtime_error(message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Runtime,
        message: message.into(),
    }
}
