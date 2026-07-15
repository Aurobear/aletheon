//! Opaque, domain-neutral Kernel lifecycle composition.

use crate::chronos::SystemClock;
use crate::operation::OperationTable;
use crate::process::ProcessTable;
use crate::space::InMemorySpaceManager;
use fabric::{
    CancelReason, Clock, ContextSpace, ExitReason, ExitStatus, OperationHandle, OperationId,
    OperationManager, OperationRecord, OperationRequest, OperationResult, ProcessHandle, ProcessId,
    ProcessManager, ProcessSignal, ProcessSnapshot, SpaceId, SpawnSpec,
};
use std::sync::Arc;

/// The sole cross-table lifecycle handle.
///
/// Its components are deliberately private. Callers receive immutable typed
/// snapshots/results rather than table or lock handles.
pub struct KernelRuntime {
    clock: Arc<dyn Clock>,
    spaces: Arc<InMemorySpaceManager>,
    processes: Arc<ProcessTable>,
    operations: Arc<OperationTable>,
}

impl KernelRuntime {
    pub fn new() -> Self {
        Self::with_clock(Arc::new(SystemClock::new()))
    }

    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        let spaces = Arc::new(InMemorySpaceManager::new());
        let processes = Arc::new(ProcessTable::with_space_manager(
            clock.clone(),
            spaces.clone(),
        ));
        let operations = Arc::new(OperationTable::new(clock.clone()));
        Self {
            clock,
            spaces,
            processes,
            operations,
        }
    }

    pub fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }

    pub async fn spawn_process(&self, spec: SpawnSpec) -> anyhow::Result<ProcessHandle> {
        self.processes.spawn(spec).await
    }

    pub async fn signal_process(&self, id: ProcessId, signal: ProcessSignal) -> anyhow::Result<()> {
        self.processes.signal(id, signal).await
    }

    pub async fn exit_process(&self, id: ProcessId, reason: ExitReason) -> anyhow::Result<()> {
        self.processes.mark_exit(id, reason).await
    }

    pub async fn inspect_process(&self, id: ProcessId) -> anyhow::Result<ProcessSnapshot> {
        self.processes.inspect(id).await
    }

    pub async fn wait_process(&self, id: ProcessId) -> anyhow::Result<ExitStatus> {
        self.processes.wait(id).await
    }

    pub async fn submit_operation(
        &self,
        request: OperationRequest,
    ) -> anyhow::Result<OperationHandle> {
        let owner = self.processes.inspect(request.owner).await?;
        anyhow::ensure!(!owner.state.is_terminal(), "operation owner is terminal");
        self.operations.submit(request).await
    }

    pub async fn start_operation(&self, id: OperationId) -> anyhow::Result<()> {
        self.operations.start(id).await
    }

    pub async fn succeed_operation(&self, id: OperationId) -> anyhow::Result<()> {
        self.operations.succeed(id).await
    }

    pub async fn fail_operation(
        &self,
        id: OperationId,
        message: impl Into<String>,
    ) -> anyhow::Result<()> {
        self.operations.fail(id, message).await
    }

    pub async fn panic_operation(
        &self,
        id: OperationId,
        message: impl Into<String>,
    ) -> anyhow::Result<()> {
        self.operations.panic(id, message).await
    }

    pub async fn cancel_operation(
        &self,
        id: OperationId,
        reason: CancelReason,
    ) -> anyhow::Result<()> {
        self.operations.cancel(id, reason).await
    }

    pub async fn inspect_operation(&self, id: OperationId) -> anyhow::Result<OperationRecord> {
        self.operations.inspect(id).await
    }

    pub async fn wait_operation(&self, id: OperationId) -> anyhow::Result<OperationResult> {
        self.operations.wait(id).await
    }

    pub fn inspect_space(&self, id: SpaceId) -> Option<ContextSpace> {
        self.spaces.get_space(id)
    }
}

impl Default for KernelRuntime {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Debug for KernelRuntime {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("KernelRuntime")
            .finish_non_exhaustive()
    }
}

#[async_trait::async_trait]
impl ProcessManager for KernelRuntime {
    async fn spawn(&self, spec: SpawnSpec) -> anyhow::Result<ProcessHandle> {
        self.spawn_process(spec).await
    }

    async fn signal(&self, id: ProcessId, signal: ProcessSignal) -> anyhow::Result<()> {
        self.signal_process(id, signal).await
    }

    async fn wait(&self, id: ProcessId) -> anyhow::Result<ExitStatus> {
        self.wait_process(id).await
    }

    async fn inspect(&self, id: ProcessId) -> anyhow::Result<ProcessSnapshot> {
        self.inspect_process(id).await
    }
}

#[async_trait::async_trait]
impl OperationManager for KernelRuntime {
    async fn submit(&self, request: OperationRequest) -> anyhow::Result<OperationHandle> {
        self.submit_operation(request).await
    }

    async fn cancel(&self, id: OperationId, reason: CancelReason) -> anyhow::Result<()> {
        self.cancel_operation(id, reason).await
    }

    async fn wait(&self, id: OperationId) -> anyhow::Result<OperationResult> {
        self.wait_operation(id).await
    }
}
