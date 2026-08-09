//! K5 Linux/execd ProcessController adapter (Agent Kernel V2).
//!
//! Introduces the `ProcessController` seam that owns process group, signal,
//! stdio, reconnect and PID generation.  execd has **no silent in-process
//! fallback** — a spawn that cannot start fails closed.  The legacy execd
//! client stays authoritative until the new adapter smoke passes and then
//! switches one-way.  This seam is a contract; the real adapter + installed
//! verification is the K5 PR-C.

use async_trait::async_trait;

/// A generated PID with its generation (prevents stale process handles).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PidGeneration {
    pub pid: u32,
    pub generation: u64,
}

/// ProcessController spawn request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SpawnRequest {
    pub command: String,
    pub args: Vec<String>,
    /// true → own process group (kill -pg on cancel).
    pub own_process_group: bool,
}

/// ProcessController status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ProcessStatus {
    Running,
    Exited { code: Option<i32> },
    FailedToStart,
}

/// The ProcessController port: spawn/read/cancel/restart + PID generation.
/// No silent in-process fallback — FailedToStart is a typed terminal.
#[async_trait]
pub trait ProcessController: Send + Sync {
    async fn spawn(&mut self, request: SpawnRequest) -> Result<PidGeneration, ProcessError>;
    async fn read(
        &mut self,
        pid: &PidGeneration,
        max_bytes: usize,
    ) -> Result<Vec<u8>, ProcessError>;
    async fn cancel(&mut self, pid: &PidGeneration, signal: i32) -> Result<(), ProcessError>;
    async fn restart(&mut self, pid: &PidGeneration) -> Result<PidGeneration, ProcessError>;
    async fn status(&mut self, pid: &PidGeneration) -> Result<ProcessStatus, ProcessError>;
}

/// Typed process errors — fail closed, no silent fallback.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ProcessError {
    #[error("process failed to start (no silent fallback)")]
    FailedToStart,
    #[error("unknown or stale pid generation")]
    UnknownPid,
    #[error("process already terminal")]
    AlreadyTerminal,
    #[error("read timed out")]
    ReadTimeout,
}

/// In-memory controller for contract tests.
#[derive(Default)]
pub struct InMemoryProcessController;

#[async_trait]
impl ProcessController for InMemoryProcessController {
    async fn spawn(&mut self, _request: SpawnRequest) -> Result<PidGeneration, ProcessError> {
        Ok(PidGeneration {
            pid: 1000,
            generation: 1,
        })
    }
    async fn read(
        &mut self,
        _pid: &PidGeneration,
        _max_bytes: usize,
    ) -> Result<Vec<u8>, ProcessError> {
        Ok(b"ok".to_vec())
    }
    async fn cancel(&mut self, _pid: &PidGeneration, _signal: i32) -> Result<(), ProcessError> {
        Ok(())
    }
    async fn restart(&mut self, pid: &PidGeneration) -> Result<PidGeneration, ProcessError> {
        Ok(PidGeneration {
            pid: pid.pid,
            generation: pid.generation + 1,
        })
    }
    async fn status(&mut self, _pid: &PidGeneration) -> Result<ProcessStatus, ProcessError> {
        Ok(ProcessStatus::Running)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn controller_spawns_with_pid_generation() {
        let mut controller = InMemoryProcessController;
        let pid = controller
            .spawn(SpawnRequest {
                command: "ls".into(),
                args: vec![],
                own_process_group: true,
            })
            .await
            .unwrap();
        assert_eq!(pid.generation, 1);

        let restarted = controller.restart(&pid).await.unwrap();
        assert_eq!(restarted.generation, 2);
    }
}
