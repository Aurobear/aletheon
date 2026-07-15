//! Opaque, domain-neutral Kernel lifecycle composition.

use crate::admission::budget::InMemoryBudgetController;
use crate::admission::lease::InMemoryResourceLeaseManager;
use crate::admission::ProductionAdmissionController;
use crate::chronos::SystemClock;
use crate::operation::OperationTable;
use crate::process::ProcessTable;
use crate::space::InMemorySpaceManager;
use crate::supervision::{RestartDecision, RestartPolicy, SupervisorTree};
use fabric::ipc::envelope_v2::Target;
use fabric::ipc::mailbox::{InProcessMailboxService, Mailbox, MailboxService};
use fabric::{
    AdmissionController, AdmissionError, AgentId, BudgetRequest, BudgetReservationReceipt,
    BudgetScopeId, BudgetScopeKind, CancelReason, Clock, ContextBinding, ContextSpace, ExitReason,
    ExitStatus, OperationHandle, OperationId, OperationManager, OperationRecord, OperationRequest,
    OperationResult, OsProcessId, PermitId, ProcessHandle, ProcessId, ProcessIdentity,
    ProcessManager, ProcessSignal, ProcessSnapshot, SpaceId, SpawnSpec,
};
use std::collections::HashMap;
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
    production_admission: Option<Arc<ProductionAdmissionController>>,
    budget: Arc<InMemoryBudgetController>,
    leases: Arc<InMemoryResourceLeaseManager>,
    budget_ownership: Mutex<BudgetOwnership>,
    spawn_specs: Mutex<HashMap<ProcessId, SpawnSpec>>,
    process_mailboxes: Mutex<HashMap<ProcessId, Target>>,
    terminal_outcomes: Mutex<HashMap<ProcessId, TerminalOutcome>>,
    terminal_progress: Mutex<HashMap<ProcessId, TerminalProgress>>,
    lifecycle: Mutex<()>,
    identities: Mutex<IdentityRegistry>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TerminalOutcome {
    pub process: ProcessId,
    pub reason: ExitReason,
    pub restart_decision: RestartDecision,
    pub restarted: Vec<ProcessHandle>,
}

#[derive(Debug, Clone)]
struct TerminalProgress {
    reason: ExitReason,
    phase: TerminalPhase,
    decision: Option<RestartDecision>,
    restart_ids: Vec<ProcessId>,
    restarted: Vec<ProcessHandle>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum TerminalPhase {
    Cleanup,
    MarkTerminal,
    Supervise,
    Restart,
}

#[derive(Default)]
struct IdentityRegistry {
    by_agent: HashMap<AgentId, ProcessIdentity>,
    by_process: HashMap<ProcessId, ProcessIdentity>,
    generations: HashMap<AgentId, u64>,
}

#[derive(Default)]
struct BudgetOwnership {
    rollouts: HashMap<String, BudgetScopeId>,
    process_rollouts: HashMap<ProcessId, BudgetScopeId>,
    processes: HashMap<ProcessId, BudgetScopeId>,
    operations: HashMap<OperationId, BudgetScopeId>,
    capabilities: HashMap<PermitId, BudgetScopeId>,
}

impl KernelRuntime {
    pub fn new() -> Self {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let budget = Arc::new(InMemoryBudgetController::new());
        let leases = Arc::new(InMemoryResourceLeaseManager::new());
        let production = Arc::new(
            ProductionAdmissionController::new(clock.clone())
                .with_budget(budget.clone())
                .with_leases(leases.clone())
                .with_sandbox_available(false),
        );
        let admission: Arc<dyn AdmissionController> = production.clone();
        Self::with_components(clock, admission, Some(production), budget, leases)
    }

    pub fn with_clock(clock: Arc<dyn Clock>) -> Self {
        let budget = Arc::new(InMemoryBudgetController::new());
        let leases = Arc::new(InMemoryResourceLeaseManager::new());
        let production = Arc::new(
            ProductionAdmissionController::new(clock.clone())
                .with_budget(budget.clone())
                .with_leases(leases.clone())
                .with_sandbox_available(false),
        );
        let admission: Arc<dyn AdmissionController> = production.clone();
        Self::with_components(clock, admission, Some(production), budget, leases)
    }

    pub fn with_admission(clock: Arc<dyn Clock>, admission: Arc<dyn AdmissionController>) -> Self {
        Self::with_components(
            clock,
            admission,
            None,
            Arc::new(InMemoryBudgetController::new()),
            Arc::new(InMemoryResourceLeaseManager::new()),
        )
    }

    fn with_components(
        clock: Arc<dyn Clock>,
        admission: Arc<dyn AdmissionController>,
        production_admission: Option<Arc<ProductionAdmissionController>>,
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
            production_admission,
            budget,
            leases,
            budget_ownership: Mutex::new(BudgetOwnership::default()),
            spawn_specs: Mutex::new(HashMap::new()),
            process_mailboxes: Mutex::new(HashMap::new()),
            terminal_outcomes: Mutex::new(HashMap::new()),
            terminal_progress: Mutex::new(HashMap::new()),
            lifecycle: Mutex::new(()),
            identities: Mutex::new(IdentityRegistry::default()),
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

    pub async fn register_process_mailbox(
        &self,
        process: ProcessId,
        target: Target,
        mailbox: Arc<dyn Mailbox>,
    ) -> anyhow::Result<()> {
        self.inspect_process(process).await?;
        self.mailboxes.register(target.clone(), mailbox).await?;
        self.process_mailboxes.lock().await.insert(process, target);
        Ok(())
    }

    pub fn budget_controller(&self) -> Arc<InMemoryBudgetController> {
        self.budget.clone()
    }

    pub fn lease_manager(&self) -> Arc<InMemoryResourceLeaseManager> {
        self.leases.clone()
    }

    pub async fn create_rollout_budget(
        &self,
        rollout: impl Into<String>,
        limit: BudgetRequest,
    ) -> BudgetScopeId {
        let rollout = rollout.into();
        let scope = self.budget.create_root(rollout.clone(), limit).await;
        self.budget_ownership
            .lock()
            .await
            .rollouts
            .insert(rollout, scope);
        scope
    }

    pub async fn reserve_process_budget(
        &self,
        rollout_scope: BudgetScopeId,
        process: ProcessId,
        request: BudgetRequest,
    ) -> Result<BudgetReservationReceipt, AdmissionError> {
        self.inspect_process(process)
            .await
            .map_err(|_| AdmissionError::BudgetExceeded)?;
        let receipt = self
            .budget
            .reserve_child(
                rollout_scope,
                BudgetScopeKind::Process,
                format!("process:{}", process.0),
                request,
            )
            .await?;
        let mut ownership = self.budget_ownership.lock().await;
        ownership.process_rollouts.insert(process, rollout_scope);
        ownership.processes.insert(process, receipt.scope_id);
        Ok(receipt)
    }

    pub async fn reserve_operation_budget(
        &self,
        process_scope: BudgetScopeId,
        operation: OperationId,
        request: BudgetRequest,
    ) -> Result<BudgetReservationReceipt, AdmissionError> {
        self.inspect_operation(operation)
            .await
            .map_err(|_| AdmissionError::BudgetExceeded)?;
        let receipt = self
            .budget
            .reserve_child(
                process_scope,
                BudgetScopeKind::Operation,
                format!("operation:{}", operation.0),
                request,
            )
            .await?;
        self.budget
            .bind_operation_scope(operation, receipt.scope_id)
            .await;
        self.budget_ownership
            .lock()
            .await
            .operations
            .insert(operation, receipt.scope_id);
        Ok(receipt)
    }

    pub async fn reserve_capability_budget(
        &self,
        operation_scope: BudgetScopeId,
        permit: PermitId,
        request: BudgetRequest,
    ) -> Result<BudgetReservationReceipt, AdmissionError> {
        let receipt = self
            .budget
            .reserve_child(
                operation_scope,
                BudgetScopeKind::Capability,
                format!("permit:{}", permit.0),
                request,
            )
            .await?;
        self.budget_ownership
            .lock()
            .await
            .capabilities
            .insert(permit, receipt.scope_id);
        Ok(receipt)
    }

    pub async fn supervise(&self, process: ProcessId, policy: RestartPolicy) {
        self.supervisor.lock().await.supervise(process, policy);
    }

    pub async fn spawn_process(&self, spec: SpawnSpec) -> anyhow::Result<ProcessHandle> {
        let retained_spec = spec.clone();
        let agent_id = spec.agent_id;
        let existing = self
            .identities
            .lock()
            .await
            .by_agent
            .get(&agent_id)
            .copied();
        if let Some(existing) = existing {
            if !self
                .processes
                .inspect(existing.process_id)
                .await?
                .state
                .is_terminal()
            {
                anyhow::bail!(
                    "agent {:?} already owns live process {:?}",
                    agent_id,
                    existing.process_id
                );
            }
        }
        let parent = spec.parent;
        let namespace = spec.namespace.0.clone();
        let process = self.processes.spawn(spec).await?;
        let inherited_root = if let Some(parent) = parent {
            self.budget_ownership
                .lock()
                .await
                .process_rollouts
                .get(&parent)
                .copied()
        } else {
            None
        };
        let root = match inherited_root {
            Some(root) => root,
            None => {
                self.budget
                    .create_root(
                        format!("rollout:{namespace}"),
                        BudgetRequest {
                            max_tokens: None,
                            max_cost_micro: None,
                        },
                    )
                    .await
            }
        };
        let process_budget = self
            .budget
            .reserve_child(
                root,
                BudgetScopeKind::Process,
                format!("process:{}", process.id.0),
                BudgetRequest {
                    max_tokens: None,
                    max_cost_micro: None,
                },
            )
            .await
            .map_err(|error| anyhow::anyhow!("process budget allocation failed: {error}"))?;
        let mut ownership = self.budget_ownership.lock().await;
        ownership.process_rollouts.insert(process.id, root);
        ownership
            .processes
            .insert(process.id, process_budget.scope_id);
        drop(ownership);
        self.spawn_specs
            .lock()
            .await
            .insert(process.id, retained_spec);
        let mut identities = self.identities.lock().await;
        let generation = identities
            .generations
            .get(&agent_id)
            .copied()
            .unwrap_or(0)
            .saturating_add(1);
        identities.generations.insert(agent_id, generation);
        let identity = ProcessIdentity {
            agent_id,
            process_id: process.id,
            generation,
            os_pid: None,
        };
        identities.by_agent.insert(agent_id, identity);
        identities.by_process.insert(process.id, identity);
        Ok(process)
    }

    pub async fn bind_os_process_id(
        &self,
        process: ProcessId,
        os_pid: OsProcessId,
    ) -> anyhow::Result<ProcessIdentity> {
        let mut identities = self.identities.lock().await;
        let mut identity = *identities
            .by_process
            .get(&process)
            .ok_or_else(|| anyhow::anyhow!("unknown process identity: {process:?}"))?;
        let current = identities
            .by_agent
            .get(&identity.agent_id)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("agent identity is unbound"))?;
        anyhow::ensure!(
            current.process_id == process,
            "cannot bind OS PID to stale process generation"
        );
        identity.os_pid = Some(os_pid);
        identities.by_process.insert(process, identity);
        identities.by_agent.insert(identity.agent_id, identity);
        Ok(identity)
    }

    pub async fn identity_for_agent(&self, agent: AgentId) -> Option<ProcessIdentity> {
        self.identities.lock().await.by_agent.get(&agent).copied()
    }

    pub async fn identity_for_process(&self, process: ProcessId) -> Option<ProcessIdentity> {
        self.identities
            .lock()
            .await
            .by_process
            .get(&process)
            .copied()
    }

    pub async fn signal_process(&self, id: ProcessId, signal: ProcessSignal) -> anyhow::Result<()> {
        match signal {
            ProcessSignal::Terminate => {
                self.terminate_process(id, ExitReason::Cancelled("terminated".into()))
                    .await?;
                Ok(())
            }
            ProcessSignal::Kill => {
                self.terminate_process(id, ExitReason::Panic("killed".into()))
                    .await?;
                Ok(())
            }
            other => self.processes.signal(id, other).await,
        }
    }

    pub async fn exit_process(&self, id: ProcessId, reason: ExitReason) -> anyhow::Result<()> {
        self.terminate_process(id, reason).await.map(|_| ())
    }

    /// Validate and complete one process terminal transaction. Cleanup is
    /// serialized, retry-idempotent, and always precedes publishing the
    /// terminal Process snapshot and applying supervision.
    pub async fn terminate_process(
        &self,
        id: ProcessId,
        reason: ExitReason,
    ) -> anyhow::Result<TerminalOutcome> {
        let _lifecycle = self.lifecycle.lock().await;
        if let Some(outcome) = self.terminal_outcomes.lock().await.get(&id).cloned() {
            return Ok(outcome);
        }
        self.terminal_progress
            .lock()
            .await
            .entry(id)
            .or_insert_with(|| TerminalProgress {
                reason,
                phase: TerminalPhase::Cleanup,
                decision: None,
                restart_ids: Vec::new(),
                restarted: Vec::new(),
            });

        loop {
            let progress = self
                .terminal_progress
                .lock()
                .await
                .get(&id)
                .cloned()
                .expect("terminal progress exists");
            match progress.phase {
                TerminalPhase::Cleanup => {
                    self.cleanup_process_resources(id).await?;
                    self.terminal_progress
                        .lock()
                        .await
                        .get_mut(&id)
                        .expect("terminal progress exists")
                        .phase = TerminalPhase::MarkTerminal;
                }
                TerminalPhase::MarkTerminal => {
                    if !self.processes.inspect(id).await?.state.is_terminal() {
                        self.processes
                            .mark_exit(id, progress.reason.clone())
                            .await?;
                    }
                    self.terminal_progress
                        .lock()
                        .await
                        .get_mut(&id)
                        .expect("terminal progress exists")
                        .phase = TerminalPhase::Supervise;
                }
                TerminalPhase::Supervise => {
                    // Hold the progress entry while recording the decision so
                    // cancellation cannot consume a restart attempt without
                    // durably advancing this in-memory transaction.
                    let mut progresses = self.terminal_progress.lock().await;
                    let current = progresses.get_mut(&id).expect("terminal progress exists");
                    if current.phase != TerminalPhase::Supervise {
                        continue;
                    }
                    let mut supervisor = self.supervisor.lock().await;
                    let decision = supervisor.record_exit(id, &current.reason);
                    current.restart_ids = match &decision {
                        RestartDecision::Restart { .. } => vec![id],
                        RestartDecision::RestartGroup { siblings, .. } => {
                            let mut ids = Vec::with_capacity(siblings.len() + 1);
                            ids.push(id);
                            ids.extend(siblings.iter().copied());
                            ids
                        }
                        RestartDecision::DoNotRestart | RestartDecision::FailedLimitReached => {
                            Vec::new()
                        }
                    };
                    current.decision = Some(decision);
                    current.phase = TerminalPhase::Restart;
                }
                TerminalPhase::Restart => break,
            }
        }

        let progress = self
            .terminal_progress
            .lock()
            .await
            .get(&id)
            .cloned()
            .expect("terminal progress exists");
        for restart_id in progress
            .restart_ids
            .iter()
            .copied()
            .skip(progress.restarted.len())
        {
            if restart_id != id
                && !self
                    .processes
                    .inspect(restart_id)
                    .await?
                    .state
                    .is_terminal()
            {
                self.cleanup_process_resources(restart_id).await?;
                self.processes
                    .mark_exit(
                        restart_id,
                        ExitReason::Cancelled("supervisor group restart".into()),
                    )
                    .await?;
            }
            let Some(spec) = self.spawn_specs.lock().await.get(&restart_id).cloned() else {
                continue;
            };
            let replacement = self.spawn_process(spec).await?;
            self.supervisor
                .lock()
                .await
                .inherit_restart_lineage(restart_id, replacement.id);
            self.terminal_progress
                .lock()
                .await
                .get_mut(&id)
                .expect("terminal progress retained")
                .restarted
                .push(replacement);
        }
        let progress = self
            .terminal_progress
            .lock()
            .await
            .remove(&id)
            .expect("terminal progress retained");
        let outcome = TerminalOutcome {
            process: id,
            reason: progress.reason,
            restart_decision: progress.decision.expect("supervision phase completed"),
            restarted: progress.restarted,
        };
        self.terminal_outcomes
            .lock()
            .await
            .insert(id, outcome.clone());
        Ok(outcome)
    }

    async fn cleanup_process_resources(&self, id: ProcessId) -> anyhow::Result<()> {
        let snapshot = self.processes.inspect(id).await?;
        for operation in self.operations.ids_for_owner(id).await {
            self.operations
                .cancel(operation, CancelReason::Other("process_exit".into()))
                .await?;
        }
        if let Some(admission) = &self.production_admission {
            admission.revoke_process_permits(id).await;
        }
        let process_scope = self
            .budget_ownership
            .lock()
            .await
            .processes
            .get(&id)
            .copied();
        if let Some(scope) = process_scope {
            self.budget
                .revoke_scope_tree(scope)
                .await
                .map_err(|error| anyhow::anyhow!("budget cleanup failed: {error}"))?;
        }
        if let Some(target) = self.process_mailboxes.lock().await.remove(&id) {
            self.mailboxes.unregister(&target).await;
        }
        self.spaces.release(snapshot.space);
        Ok(())
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
        let owner_id = request.owner;
        let operation = self.operations.submit(request).await?;
        self.bind_default_operation_budget(owner_id, operation.id)
            .await?;
        Ok(operation)
    }

    pub async fn submit_operation_with_id(
        &self,
        id: OperationId,
        request: OperationRequest,
    ) -> anyhow::Result<OperationHandle> {
        let owner = self.processes.inspect(request.owner).await?;
        anyhow::ensure!(!owner.state.is_terminal(), "operation owner is terminal");
        let owner_id = request.owner;
        let operation = self.operations.submit_with_id(id, request).await?;
        self.bind_default_operation_budget(owner_id, operation.id)
            .await?;
        Ok(operation)
    }

    async fn bind_default_operation_budget(
        &self,
        owner: ProcessId,
        operation: OperationId,
    ) -> anyhow::Result<()> {
        let process_scope = self
            .budget_ownership
            .lock()
            .await
            .processes
            .get(&owner)
            .copied()
            .ok_or_else(|| anyhow::anyhow!("process budget scope missing"))?;
        let receipt = self
            .budget
            .reserve_child(
                process_scope,
                BudgetScopeKind::Operation,
                format!("operation:{}", operation.0),
                BudgetRequest {
                    max_tokens: None,
                    max_cost_micro: None,
                },
            )
            .await
            .map_err(|error| anyhow::anyhow!("operation budget allocation failed: {error}"))?;
        self.budget
            .bind_operation_scope(operation, receipt.scope_id)
            .await;
        self.budget_ownership
            .lock()
            .await
            .operations
            .insert(operation, receipt.scope_id);
        Ok(())
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
