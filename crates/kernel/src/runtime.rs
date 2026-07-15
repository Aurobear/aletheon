//! Opaque, domain-neutral Kernel lifecycle composition.

use crate::admission::budget::InMemoryBudgetController;
use crate::admission::lease::InMemoryResourceLeaseManager;
use crate::admission::ProductionAdmissionController;
use crate::chronos::SystemClock;
use crate::operation::OperationTable;
use crate::process::ProcessTable;
use crate::space::InMemorySpaceManager;
use crate::supervision::{RestartDecision, RestartPolicy, SupervisorTree};
use fabric::ipc::mailbox::InProcessMailboxService;
use fabric::{
    AdmissionController, CancelReason, Clock, ContextBinding, ContextSpace, ExitReason, ExitStatus,
    OperationHandle, OperationId, OperationManager, OperationRecord, OperationRequest,
    OperationResult, ProcessHandle, ProcessId, ProcessManager, ProcessSignal, ProcessSnapshot,
    SpaceId, SpawnSpec,
};
use std::sync::Arc;
use tokio::sync::Mutex;

/// The sole cross-table lifecycle handle.
///
/// Its components are deliberately private. Callers receive immutable typed
/// snapshots/results rather than table or lock handles.
pub struct KernelRuntime {
    clock: Arc<dyn Clock>,
    spaces: Arc<InMemorySpaceManager>,
    processes: Arc<ProcessTable>,
    operations: Arc<OperationTable>,
    supervisor: Mutex<SupervisorTree>,
    mailboxes: Arc<InProcessMailboxService>,
    admission: Arc<dyn AdmissionController>,
    budget: Arc<InMemoryBudgetController>,
    leases: Arc<InMemoryResourceLeaseManager>,
}

impl KernelRuntime {
    pub fn new() -> Self {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let budget = Arc::new(InMemoryBudgetController::new());
        let leases = Arc::new(InMemoryResourceLeaseManager::new());
        let admission: Arc<dyn AdmissionController> = Arc::new(
            ProductionAdmissionController::new(clock.clone())
                .with_budget(budget.clone())
                .with_leases(leases.clone())
                .with_sandbox_available(false),
        );
        Self::with_components(clock, admission, budget, leases)
    }

    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        let budget = Arc::new(InMemoryBudgetController::new());
        let leases = Arc::new(InMemoryResourceLeaseManager::new());
        let admission: Arc<dyn AdmissionController> = Arc::new(
            ProductionAdmissionController::new(clock.clone())
                .with_budget(budget.clone())
                .with_leases(leases.clone())
                .with_sandbox_available(false),
        );
        Self::with_components(clock, admission, budget, leases)
    }

    pub fn with_admission(clock: Arc<dyn Clock>, admission: Arc<dyn AdmissionController>) -> Self {
        Self::with_components(
            clock,
            admission,
            Arc::new(InMemoryBudgetController::new()),
            Arc::new(InMemoryResourceLeaseManager::new()),
        )
    }

    fn with_components(
        clock: Arc<dyn Clock>,
        admission: Arc<dyn AdmissionController>,
        budget: Arc<InMemoryBudgetController>,
        leases: Arc<InMemoryResourceLeaseManager>,
    ) -> Self {
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
            supervisor: Mutex::new(SupervisorTree::new()),
            mailboxes: Arc::new(InProcessMailboxService::new()),
            admission,
            budget,
            leases,
        }
    }

    pub fn clock(&self) -> Arc<dyn Clock> {
        self.clock.clone()
    }

    pub fn admission(&self) -> Arc<dyn AdmissionController> {
        self.admission.clone()
    }

    pub fn mailbox_service(&self) -> Arc<InProcessMailboxService> {
        self.mailboxes.clone()
    }

    pub fn budget_controller(&self) -> Arc<InMemoryBudgetController> {
        self.budget.clone()
    }

    pub fn lease_manager(&self) -> Arc<InMemoryResourceLeaseManager> {
        self.leases.clone()
    }

    pub async fn supervise(&self, process: ProcessId, policy: RestartPolicy) {
        self.supervisor.lock().await.supervise(process, policy);
    }

    pub async fn record_supervised_exit(
        &self,
        process: ProcessId,
        reason: &ExitReason,
    ) -> RestartDecision {
        self.supervisor.lock().await.record_exit(process, reason)
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

    pub async fn set_active_operation(
        &self,
        process: ProcessId,
        operation: Option<OperationId>,
    ) -> anyhow::Result<()> {
        self.processes
            .set_active_operation(process, operation)
            .await
    }

    pub async fn reap_process(&self, id: ProcessId) -> anyhow::Result<fabric::ProcessRecord> {
        self.processes.reap(id).await
    }

    pub async fn submit_operation(
        &self,
        request: OperationRequest,
    ) -> anyhow::Result<OperationHandle> {
        let owner = self.processes.inspect(request.owner).await?;
        anyhow::ensure!(!owner.state.is_terminal(), "operation owner is terminal");
        self.operations.submit(request).await
    }

    pub async fn submit_operation_with_id(
        &self,
        id: OperationId,
        request: OperationRequest,
    ) -> anyhow::Result<OperationHandle> {
        let owner = self.processes.inspect(request.owner).await?;
        anyhow::ensure!(!owner.state.is_terminal(), "operation owner is terminal");
        self.operations.submit_with_id(id, request).await
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

    pub fn upsert_space_binding(&self, id: SpaceId, binding: ContextBinding) {
        self.spaces.upsert_binding(id, binding);
    }

    pub fn set_space_overlay(
        &self,
        id: SpaceId,
        key: impl Into<String>,
        value: serde_json::Value,
    ) -> anyhow::Result<()> {
        self.spaces.set_overlay(id, key, value)
    }

    pub fn release_space(&self, id: SpaceId) -> bool {
        self.spaces.release(id)
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
