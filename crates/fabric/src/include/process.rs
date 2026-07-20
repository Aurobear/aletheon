//! Process and operation manager contracts.

use crate::types::operation::{CancelReason, OperationId, OperationRequest, OperationResult};
use crate::types::process::{ExitStatus, ProcessSignal, ProcessSnapshot, SpawnSpec};
use async_trait::async_trait;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ProcessHandle {
    pub id: crate::types::operation::ProcessId,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationHandle {
    pub id: OperationId,
}

#[async_trait]
pub trait ProcessManager: Send + Sync {
    async fn spawn(&self, spec: SpawnSpec) -> anyhow::Result<ProcessHandle>;
    async fn signal(
        &self,
        id: crate::types::operation::ProcessId,
        signal: ProcessSignal,
    ) -> anyhow::Result<()>;
    async fn wait(&self, id: crate::types::operation::ProcessId) -> anyhow::Result<ExitStatus>;
    async fn inspect(
        &self,
        id: crate::types::operation::ProcessId,
    ) -> anyhow::Result<ProcessSnapshot>;
}

#[async_trait]
pub trait OperationManager: Send + Sync {
    async fn submit(&self, req: OperationRequest) -> anyhow::Result<OperationHandle>;
    async fn cancel(&self, id: OperationId, reason: CancelReason) -> anyhow::Result<()>;
    async fn wait(&self, id: OperationId) -> anyhow::Result<OperationResult>;
}
