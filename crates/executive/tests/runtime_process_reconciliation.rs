#![cfg(unix)]

use executive::application::agent_control::{
    RuntimeProcessReclaimOutcome, RuntimeProcessSupervisor,
};
use executive::testing::coding_runtime::LinuxRuntimeProcessSupervisor;
use fabric::{AgentId, OsProcessId, ProcessId, RuntimeProcessId};
use std::io;
use std::os::unix::process::CommandExt;
use std::process::Command;
use std::time::Duration;

fn process_start_time_ticks(pid: u32) -> io::Result<u64> {
    let stat = std::fs::read_to_string(format!("/proc/{pid}/stat"))?;
    let command_end = stat
        .rfind(')')
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "process stat lacks command"))?;
    stat[command_end + 1..]
        .split_whitespace()
        .nth(19)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidData, "process stat lacks start time"))?
        .parse()
        .map_err(|error| io::Error::new(io::ErrorKind::InvalidData, error))
}

fn spawn_group() -> std::process::Child {
    let mut command = Command::new("sleep");
    command.arg("60");
    command.process_group(0);
    command.spawn().unwrap()
}

#[tokio::test]
async fn matching_process_generation_is_reclaimed_and_pid_reuse_is_not_signaled() {
    let mut child = spawn_group();
    let pid = child.id();
    let identity = RuntimeProcessId {
        agent_id: AgentId::new(),
        process_id: ProcessId::new(),
        generation: 1,
        os_pid: OsProcessId(pid),
        start_time_ticks: process_start_time_ticks(pid).unwrap(),
    };
    assert_eq!(
        LinuxRuntimeProcessSupervisor
            .reclaim(identity)
            .await
            .unwrap(),
        RuntimeProcessReclaimOutcome::Reclaimed
    );
    let status = tokio::time::timeout(Duration::from_secs(1), async {
        loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(!status.success());

    let mut unrelated = spawn_group();
    let unrelated_pid = unrelated.id();
    let reused = RuntimeProcessId {
        os_pid: OsProcessId(unrelated_pid),
        start_time_ticks: process_start_time_ticks(unrelated_pid)
            .unwrap()
            .saturating_add(1),
        ..identity
    };
    assert_eq!(
        LinuxRuntimeProcessSupervisor.reclaim(reused).await.unwrap(),
        RuntimeProcessReclaimOutcome::IdentityReused
    );
    assert!(unrelated.try_wait().unwrap().is_none());
    unrelated.kill().unwrap();
    let _ = unrelated.wait();
}
