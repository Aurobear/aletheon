use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use fabric::ipc::envelope_v2::{DeliveryPattern, EnvelopeV2, SchemaId, Target};
use fabric::ipc::mailbox::{InProcessMailbox, Mailbox};
use fabric::{
    AgentControlError, AgentControlErrorKind, AgentControlMessage, AgentControlPort, AgentHandle,
    AgentId, AgentListRequest, AgentMessageDeliveryState, AgentMessagePayload, AgentRunStatus,
    AgentSendRequest, AgentSnapshot, AgentSpawnRequest, AgentWaitRequest, AgoraVersion,
    CancelReason, Clock, ContextBinding, EventSpine, ExitReason, NamespaceId, OperationExitReason,
    OperationKind, OperationRequest, ProcessId, ProcessSignal, SettlementTerminal, SpawnSpec,
    Timer,
};
use kernel::chronos::SystemTimer;
use kernel::operation::OperationScope;
use kernel::KernelRuntime;
use tokio::sync::{watch, Mutex};
use tokio::task::JoinSet;

pub mod admission;
pub mod candidate_projection;
pub mod cleanup;
pub mod context_fork;
pub mod execution;
pub mod live_runs;
pub mod mailbox;
pub mod memory;
pub mod recovery;
pub mod repository;
pub mod settlement;
pub mod sqlite_repository;

pub use admission::{
    AgentAdmissionLease, AgentAdmissionMetrics, AgentAdmissionPort, AgentAdmissionRequest,
    AgentStorageRequest, BoundedAgentAdmission,
};
pub use candidate_projection::{
    AgentCandidateProjector, AgentCandidateSubmissionPort, ProjectingAgentEventSink,
};
pub use cleanup::{AgentCleanupCoordinator, AgentCleanupReport, MAX_CLEANUP_BATCH};
pub use context_fork::{
    AgentContextItem, AgentContextItemKind, AgentContextProjection, AgentContextProjectionBuilder,
};
pub use execution::{
    AgentEventSink, AgentRecoveryRuntimeInput, AgentRuntimeEvent, AgentRuntimeInput,
    AgentRuntimeLauncher, AgentRuntimeRegistry, BackgroundResourceRegistration,
    CompatibilityRuntimeLauncher, NoopAgentEventSink, SpineAgentEventSink,
};
pub use live_runs::{LiveAgentRun, LiveAgentRuns, ReparentAuthority};
pub use mailbox::{AgentMailboxBridge, AgentRuntimeInbox};
pub use memory::MemoryRecordingAgentEventSink;
pub use recovery::{
    AgentRecoveryCoordinator, AgentRecoveryObservation, AgentRecoveryReport,
    MAX_STARTUP_RECOVERY_ROWS,
};
pub use repository::{
    agent_workspace_id, AgentMessageRecord, AgentResourceLease, AgentResourceLeaseKind,
    AgentRunRecord, AgentRunRepository,
};
pub use settlement::{
    recovery_disposition, settle_admission, terminal_with_memory_flush,
    FailClosedSettlementResourcePort, InMemorySettlementReceiptStore,
    ManagedSettlementResourcePort, NoopSettlementEvidenceSink, RecoveryResourceDisposition,
    RepositorySettlementLeasePort, SettlementEngine, SettlementEvidence, SettlementEvidenceSink,
    SettlementLeasePort, SettlementMetricSnapshot, SettlementMetrics, SettlementReceiptStore,
    SettlementRequest, SettlementResourcePort, SpineSettlementEvidenceSink,
    SqliteSettlementReceiptStore,
};
pub use sqlite_repository::SqliteAgentRunRepository;

const DEFAULT_RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
const MAILBOX_CAPACITY: usize = 64;
const CANCEL_WAIT: Duration = Duration::from_secs(30);

struct ValidatedAgentIdentity {
    agent_id: AgentId,
    root_process_id: Option<ProcessId>,
    root_workspace_id: Option<fabric::AgoraSpaceId>,
    depth: u16,
    parent_profile: Option<fabric::AgentProfileId>,
}

#[async_trait]
pub trait AgentWaitTimer: Send + Sync {
    async fn wait_for_change(
        &self,
        receiver: &mut watch::Receiver<AgentSnapshot>,
        timeout: Duration,
    ) -> bool;
}

#[derive(Debug, Default)]
pub struct SystemAgentWaitTimer;

#[async_trait]
impl AgentWaitTimer for SystemAgentWaitTimer {
    async fn wait_for_change(
        &self,
        receiver: &mut watch::Receiver<AgentSnapshot>,
        timeout: Duration,
    ) -> bool {
        matches!(
            SystemTimer.timeout(timeout, receiver.changed()).await,
            Ok(Ok(()))
        )
    }
}

pub struct AgentControlService {
    kernel: Arc<KernelRuntime>,
    clock: Arc<dyn Clock>,
    repository: Arc<dyn AgentRunRepository>,
    admission: Arc<dyn AgentAdmissionPort>,
    runtimes: Arc<AgentRuntimeRegistry>,
    events: Arc<dyn AgentEventSink>,
    event_spine: Arc<dyn EventSpine>,
    event_projections: Arc<dyn crate::service::event_projection::EventProjectionSink>,
    timer: Arc<dyn AgentWaitTimer>,
    live: Arc<LiveAgentRuns>,
    tasks: Mutex<JoinSet<()>>,
    sibling_routes: parking_lot::RwLock<HashSet<(AgentId, AgentId, AgentId)>>,
    agent_memory_vault: Arc<mnemosyne::AgentMemoryVault>,
    subagent_settlement: bool,
    settlement_generation: String,
    settlement_receipts: Arc<dyn SettlementReceiptStore>,
    settlement_metrics: Arc<SettlementMetrics>,
    budget_controller: Option<Arc<dyn fabric::BudgetController>>,
    lifecycle_hooks: Arc<dyn AgentLifecycleHookSink>,
}

#[async_trait]
pub trait AgentLifecycleHookSink: Send + Sync {
    async fn emit(&self, context: fabric::hook::HookContext);
}

struct NoopAgentLifecycleHookSink;

#[async_trait]
impl AgentLifecycleHookSink for NoopAgentLifecycleHookSink {
    async fn emit(&self, _context: fabric::hook::HookContext) {}
}

pub struct CorpusAgentLifecycleHookSink(pub Arc<dyn corpus::CorpusService>);

#[async_trait]
impl AgentLifecycleHookSink for CorpusAgentLifecycleHookSink {
    async fn emit(&self, context: fabric::hook::HookContext) {
        self.0.execute_hook(&context).await;
    }
}

impl std::fmt::Debug for AgentControlService {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentControlService")
            .finish_non_exhaustive()
    }
}

impl AgentControlService {
    pub fn new(
        kernel: Arc<KernelRuntime>,
        clock: Arc<dyn Clock>,
        repository: Arc<dyn AgentRunRepository>,
        admission: Arc<dyn AgentAdmissionPort>,
        runtimes: Arc<AgentRuntimeRegistry>,
    ) -> Self {
        let event_spine = Arc::new(
            crate::r#impl::events::SqliteEventSpine::open(":memory:")
                .expect("in-memory Agent event spine"),
        );
        Self {
            kernel,
            clock,
            repository,
            admission,
            runtimes,
            events: Arc::new(NoopAgentEventSink),
            event_spine,
            event_projections: Arc::new(crate::service::event_projection::NoopEventProjectionSink),
            timer: Arc::new(SystemAgentWaitTimer),
            live: Arc::new(LiveAgentRuns::default()),
            tasks: Mutex::new(JoinSet::new()),
            sibling_routes: parking_lot::RwLock::new(HashSet::new()),
            agent_memory_vault: Arc::new(
                mnemosyne::AgentMemoryVault::in_memory().expect("in-memory Agent memory vault"),
            ),
            subagent_settlement: false,
            settlement_generation: "disabled".into(),
            settlement_receipts: Arc::new(InMemorySettlementReceiptStore::default()),
            settlement_metrics: Arc::new(SettlementMetrics::default()),
            budget_controller: None,
            lifecycle_hooks: Arc::new(NoopAgentLifecycleHookSink),
        }
    }

    pub fn with_lifecycle_hooks(mut self, hooks: Arc<dyn AgentLifecycleHookSink>) -> Self {
        self.lifecycle_hooks = hooks;
        self
    }

    pub fn with_event_sink(mut self, events: Arc<dyn AgentEventSink>) -> Self {
        self.events = events;
        self
    }

    pub fn with_event_spine(mut self, event_spine: Arc<dyn EventSpine>) -> Self {
        self.event_spine = event_spine;
        self
    }

    pub fn with_event_projections(
        mut self,
        projections: Arc<dyn crate::service::event_projection::EventProjectionSink>,
    ) -> Self {
        self.event_projections = projections;
        self
    }

    pub fn with_wait_timer(mut self, timer: Arc<dyn AgentWaitTimer>) -> Self {
        self.timer = timer;
        self
    }

    pub fn with_memory_vault(mut self, memory: Arc<mnemosyne::AgentMemoryVault>) -> Self {
        self.agent_memory_vault = memory;
        self
    }

    pub fn with_subagent_settlement(
        mut self,
        enabled: bool,
        generation: impl Into<String>,
        receipts: Arc<dyn SettlementReceiptStore>,
    ) -> Self {
        self.subagent_settlement = enabled;
        self.settlement_generation = generation.into();
        self.settlement_receipts = receipts;
        self
    }

    pub fn with_budget_controller(mut self, budget: Arc<dyn fabric::BudgetController>) -> Self {
        self.budget_controller = Some(budget);
        self
    }

    pub fn live_runs(&self) -> Arc<LiveAgentRuns> {
        self.live.clone()
    }

    pub fn admission_metrics(&self) -> AgentAdmissionMetrics {
        self.admission.metrics()
    }

    pub fn settlement_metrics(&self) -> SettlementMetricSnapshot {
        self.settlement_metrics.snapshot()
    }

    /// Reconcile every bounded open durable row before bootstrap publishes
    /// Agent spawn tools. Native runtimes are `Never` resumable, so absence or
    /// ambiguity always becomes an explicit interruption rather than replay.
    pub async fn reconcile_startup(
        &self,
        daemon_generation: &str,
    ) -> Result<AgentRecoveryReport, AgentControlError> {
        let coordinator = AgentRecoveryCoordinator::new(
            self.repository.clone(),
            daemon_generation,
            self.clock.wall_now().0,
        )?;
        let runs = self.repository.list_open(MAX_STARTUP_RECOVERY_ROWS).await?;
        let mut report = AgentRecoveryReport {
            open_rows: runs.len(),
            ..Default::default()
        };
        for run in runs {
            let process_live = self
                .kernel
                .inspect_process(run.snapshot.handle.process_id)
                .await
                .is_ok();
            let operation_terminal = self
                .kernel
                .inspect_operation(run.snapshot.handle.operation_id)
                .await
                .ok()
                .and_then(|operation| match operation.state {
                    fabric::OperationState::Succeeded => Some(AgentRunStatus::Succeeded),
                    fabric::OperationState::Failed => Some(AgentRunStatus::Failed),
                    fabric::OperationState::Cancelled => Some(AgentRunStatus::Cancelled),
                    _ => None,
                });
            let checkpoint_available = matches!(
                &run.resumability,
                fabric::RuntimeResumability::Checkpointed { reference }
                    if !reference.trim().is_empty()
            );
            let observation = AgentRecoveryObservation {
                process_live,
                operation_terminal,
                checkpoint_available,
            };
            match coordinator.recover_one(&run, observation).await {
                Ok(fabric::AgentRecoveryDecision::Interrupt) => {
                    if self.subagent_settlement {
                        self.recover_settlement_resources(
                            &run,
                            fabric::AgentRecoveryDecision::Interrupt,
                            daemon_generation,
                        )
                        .await?;
                    }
                    report.interrupted += 1;
                }
                Ok(fabric::AgentRecoveryDecision::Resume) => {
                    let checkpoint_reference = match &run.resumability {
                        fabric::RuntimeResumability::Checkpointed { reference } => {
                            reference.clone()
                        }
                        fabric::RuntimeResumability::Never => {
                            unreachable!("resume requires checkpoint")
                        }
                    };
                    match self.runtimes.resolve(&run.snapshot.handle.runtime_id) {
                        Ok(runtime)
                            if runtime.resumability() == run.resumability
                                && runtime
                                    .resume_from_checkpoint(AgentRecoveryRuntimeInput {
                                        handle: run.snapshot.handle.clone(),
                                        request: run.request.clone(),
                                        checkpoint_reference,
                                    })
                                    .await
                                    .is_ok() =>
                        {
                            report.resumed += 1;
                        }
                        _ => report.recovery_failed += 1,
                    }
                }
                Ok(fabric::AgentRecoveryDecision::Finalize) => {
                    if self.subagent_settlement {
                        self.recover_settlement_resources(
                            &run,
                            fabric::AgentRecoveryDecision::Finalize,
                            daemon_generation,
                        )
                        .await?;
                    }
                    report.finalized += 1;
                }
                _ => report.recovery_failed += 1,
            }
        }
        report.unreconciled = self
            .repository
            .list_open(MAX_STARTUP_RECOVERY_ROWS)
            .await?
            .into_iter()
            .filter(|run| {
                !matches!(
                    run.recovery.as_ref().map(|receipt| receipt.decision),
                    Some(fabric::AgentRecoveryDecision::Resume)
                )
            })
            .count();
        Ok(report)
    }

    async fn recover_settlement_resources(
        &self,
        run: &AgentRunRecord,
        decision: fabric::AgentRecoveryDecision,
        daemon_generation: &str,
    ) -> Result<(), AgentControlError> {
        match recovery_disposition(decision) {
            RecoveryResourceDisposition::RetainForResume => return Ok(()),
            RecoveryResourceDisposition::ReplaySettlement
            | RecoveryResourceDisposition::TerminateAndReclaim => {}
        }
        if let Some(budget) = &self.budget_controller {
            let owner = format!("agent:{}", run.agent_id().0);
            if let Some(reservation) = budget.reservation_for_owner(&owner).await {
                // A durable transfer is authoritative and must never be
                // reversed by settlement recovery. Otherwise reclaiming the
                // live child reservation is owner-scoped and idempotent.
                if budget.transfer_for_child(reservation).await.is_none() {
                    match budget.revoke_reservation(reservation).await {
                        Ok(()) | Err(fabric::AdmissionError::AlreadySettled) => {}
                        Err(error) => {
                            return Err(AgentControlError::invalid(format!(
                                "budget recovery failed: {error}"
                            )));
                        }
                    }
                }
            }
        }
        let leases = self
            .repository
            .list_agent_resource_leases(run.agent_id(), MAX_STARTUP_RECOVERY_ROWS)
            .await?;
        let old_owner = leases
            .first()
            .map(|lease| lease.owner.clone())
            .unwrap_or_else(|| format!("process:{}", run.snapshot.handle.process_id.0));
        let terminal = match decision {
            fabric::AgentRecoveryDecision::Finalize => SettlementTerminal::Completed,
            _ => SettlementTerminal::Failed {
                reason: "daemon restart reclaimed child resources".into(),
            },
        };
        let engine = SettlementEngine::with_metrics(
            self.settlement_receipts.clone(),
            Arc::new(FailClosedSettlementResourcePort::new(
                tokio_util::sync::CancellationToken::new(),
            )),
            Arc::new(RepositorySettlementLeasePort::new(self.repository.clone())),
            Arc::new(NoopSettlementEvidenceSink),
            self.settlement_metrics.clone(),
        );
        engine
            .settle(
                SettlementRequest {
                    agent_id: run.agent_id().0.to_string(),
                    attempt_id: run.snapshot.handle.operation_id.0.to_string(),
                    generation: daemon_generation.to_string(),
                    old_owner,
                    parent_owner: None,
                    terminal,
                    lease_keys: leases.into_iter().map(|lease| lease.lease_key).collect(),
                    settled_at_ms: self.clock.wall_now().0,
                },
                run.request.background_decls.clone(),
            )
            .await?;
        Ok(())
    }

    /// Install an explicit parent policy for one directional sibling route.
    /// All identities are revalidated against durable topology before the
    /// policy becomes active.
    pub async fn permit_sibling_route(
        &self,
        caller_root: AgentId,
        parent: AgentId,
        from: AgentId,
        to: AgentId,
    ) -> Result<(), AgentControlError> {
        let parent_run = self.authorize(caller_root, parent).await?;
        let from_run = self.authorize(caller_root, from).await?;
        let to_run = self.authorize(caller_root, to).await?;
        if from_run.snapshot.handle.parent_agent_id != Some(parent)
            || to_run.snapshot.handle.parent_agent_id != Some(parent)
            || parent_run.status().is_terminal()
            || from_run.status().is_terminal()
            || to_run.status().is_terminal()
        {
            return Err(control_error(
                AgentControlErrorKind::Forbidden,
                "sibling route does not match one live parent topology",
            ));
        }
        self.sibling_routes.write().insert((parent, from, to));
        Ok(())
    }

    /// Authorize one visibility-filtered broadcast item against both durable
    /// receipts and the child's current Kernel context-space binding.
    pub async fn authorize_broadcast(
        &self,
        caller_root: AgentId,
        agent: AgentId,
        epoch: fabric::BroadcastEpoch,
        candidate: &fabric::WorkspaceCandidate,
    ) -> Result<(), AgentControlError> {
        candidate
            .validate()
            .map_err(|error| AgentControlError::invalid(error.to_string()))?;
        let run = self.authorize(caller_root, agent).await?;
        let process = self
            .kernel
            .inspect_process(run.snapshot.handle.process_id)
            .await
            .map_err(runtime_error)?;
        let context = self.kernel.inspect_space(process.space).ok_or_else(|| {
            control_error(
                AgentControlErrorKind::Runtime,
                "Agent Kernel context space is unavailable",
            )
        })?;
        let bound = context.bindings.iter().any(|binding| {
            matches!(
                binding,
                ContextBinding::Agora(space, _) if space == &run.workspace_id
            )
        });
        if !bound || !run.can_observe_broadcast(epoch, candidate) {
            return Err(control_error(
                AgentControlErrorKind::Forbidden,
                "broadcast is not permitted by Agent workspace receipt",
            ));
        }
        Ok(())
    }

    pub async fn shutdown(&self) {
        for run in self.live.all().await {
            run.cancellation.cancel();
        }
        let mut tasks = self.tasks.lock().await;
        while tasks.join_next().await.is_some() {}
    }

    async fn authorize_message_sender(
        &self,
        target: &AgentRunRecord,
        request: &AgentSendRequest,
    ) -> Result<AgentId, AgentControlError> {
        let Some(sender) = request.sender_agent_id else {
            return Ok(request.caller_root_agent_id);
        };
        if sender == request.caller_root_agent_id {
            return Ok(sender);
        }
        let sender_run = self.repository.get(sender).await?.ok_or_else(|| {
            control_error(
                AgentControlErrorKind::NotFound,
                "message sender was not found",
            )
        })?;
        if sender_run.root_agent_id() != request.caller_root_agent_id
            || sender_run.status().is_terminal()
        {
            return Err(control_error(
                AgentControlErrorKind::Forbidden,
                "message sender is outside the live Agent tree",
            ));
        }
        let direct_child = target.snapshot.handle.parent_agent_id == Some(sender);
        let direct_parent = sender_run.snapshot.handle.parent_agent_id == Some(target.agent_id());
        if direct_child || direct_parent {
            return Ok(sender);
        }
        if let (Some(sender_parent), Some(target_parent)) = (
            sender_run.snapshot.handle.parent_agent_id,
            target.snapshot.handle.parent_agent_id,
        ) {
            if sender_parent == target_parent
                && self
                    .sibling_routes
                    .read()
                    .contains(&(sender_parent, sender, target.agent_id()))
            {
                return Ok(sender);
            }
        }
        Err(control_error(
            AgentControlErrorKind::Forbidden,
            "sibling or non-adjacent Agent messaging requires explicit parent policy",
        ))
    }

    async fn validated_parent(
        &self,
        request: &AgentSpawnRequest,
    ) -> Result<ValidatedAgentIdentity, AgentControlError> {
        match (request.parent_agent_id, request.parent_process_id) {
            (None, None) => Ok(ValidatedAgentIdentity {
                agent_id: request.root_agent_id,
                root_process_id: None,
                root_workspace_id: None,
                depth: 0,
                parent_profile: None,
            }),
            (Some(parent), Some(parent_process)) => {
                let root_process_id;
                let depth;
                let parent_profile;
                if let Some(parent_run) = self.repository.get(parent).await? {
                    if parent_run.root_agent_id() != request.root_agent_id
                        || parent_run.snapshot.handle.process_id != parent_process
                        || parent_run.status().is_terminal()
                    {
                        return Err(control_error(
                            AgentControlErrorKind::Forbidden,
                            "parent Agent does not belong to the requested live root/process",
                        ));
                    }
                    root_process_id = parent_run.root_process_id;
                    depth = self.depth_after(&parent_run).await?;
                    parent_profile = Some(parent_run.snapshot.handle.profile_id.clone());
                } else {
                    let process =
                        self.kernel
                            .inspect_process(parent_process)
                            .await
                            .map_err(|_| {
                                control_error(
                                    AgentControlErrorKind::NotFound,
                                    "parent Agent was not found",
                                )
                            })?;
                    if process.agent_id != parent
                        || request.root_agent_id != parent
                        || process.state.is_terminal()
                    {
                        return Err(control_error(
                            AgentControlErrorKind::Forbidden,
                            "external root parent identity is not live or does not match",
                        ));
                    }
                    root_process_id = parent_process;
                    depth = 1;
                    parent_profile = Some(process.profile.clone());
                }
                let root_process = self
                    .kernel
                    .inspect_process(root_process_id)
                    .await
                    .map_err(runtime_error)?;
                let root_workspace_id = self
                    .kernel
                    .inspect_space(root_process.space)
                    .and_then(|space| {
                        space
                            .bindings
                            .into_iter()
                            .find_map(|binding| match binding {
                                ContextBinding::Agora(id, _) => Some(id),
                                _ => None,
                            })
                    })
                    .or_else(|| {
                        request
                            .broadcast_refs
                            .first()
                            .map(|item| item.space.clone())
                    })
                    // A trusted external root may spawn before its first turn
                    // has materialized a Kernel binding. Turn workspaces use
                    // the durable root/session UUID as their canonical ID.
                    .unwrap_or_else(|| fabric::AgoraSpaceId(request.root_agent_id.0.to_string()));
                Ok(ValidatedAgentIdentity {
                    agent_id: AgentId::new(),
                    root_process_id: Some(root_process_id),
                    root_workspace_id: Some(root_workspace_id),
                    depth,
                    parent_profile,
                })
            }
            _ => Err(AgentControlError::invalid(
                "parent Agent and parent Process must be supplied together",
            )),
        }
    }

    async fn depth_after(&self, parent: &AgentRunRecord) -> Result<u16, AgentControlError> {
        let mut depth = 1u16;
        let mut next = parent.snapshot.handle.parent_agent_id;
        while let Some(agent) = next {
            let Some(run) = self.repository.get(agent).await? else {
                break;
            };
            depth = depth
                .checked_add(1)
                .ok_or_else(|| AgentControlError::invalid("Agent tree depth overflow"))?;
            next = run.snapshot.handle.parent_agent_id;
        }
        Ok(depth)
    }

    async fn authorize(
        &self,
        caller_root: AgentId,
        agent: AgentId,
    ) -> Result<AgentRunRecord, AgentControlError> {
        let run = self.repository.get(agent).await?.ok_or_else(|| {
            control_error(AgentControlErrorKind::NotFound, "Agent run was not found")
        })?;
        if run.root_agent_id() != caller_root {
            return Err(control_error(
                AgentControlErrorKind::Forbidden,
                "Agent does not belong to caller root",
            ));
        }
        Ok(run)
    }

    async fn wait_for_terminal(
        &self,
        caller_root: AgentId,
        agent: AgentId,
        timeout: Duration,
    ) -> Result<AgentSnapshot, AgentControlError> {
        let initial = self.authorize(caller_root, agent).await?;
        if initial.status().is_terminal() {
            return Ok(initial.snapshot);
        }
        let Some(live) = self.live.get(agent).await else {
            return Err(control_error(
                AgentControlErrorKind::Runtime,
                "Agent is nonterminal but has no live runtime",
            ));
        };
        let mut receiver = live.snapshots.subscribe();
        let started = Instant::now();
        loop {
            let snapshot = receiver.borrow().clone();
            if snapshot.status.is_terminal() {
                return Ok(snapshot);
            }
            let remaining = timeout.saturating_sub(started.elapsed());
            if remaining.is_zero() || !self.timer.wait_for_change(&mut receiver, remaining).await {
                return Err(control_error(
                    AgentControlErrorKind::Timeout,
                    "Agent wait timed out",
                ));
            }
        }
    }
}

#[async_trait]
impl AgentControlPort for AgentControlService {
    async fn spawn(&self, request: AgentSpawnRequest) -> Result<AgentHandle, AgentControlError> {
        request.validate()?;
        let launcher = self.runtimes.resolve(&request.runtime_id)?;
        let mut context_builder = AgentContextProjectionBuilder::new().fork(&request.context)?;
        for reference in &request.broadcast_refs {
            context_builder = context_builder.broadcast_ref(reference.content_id);
        }
        let context = context_builder.build()?;
        let identity = self.validated_parent(&request).await?;
        let agent_id = identity.agent_id;
        let workspace_id = agent_workspace_id(agent_id);
        let request_hash = SqliteAgentRunRepository::request_hash(&request)?;
        let storage = if request.runtime_id.0.contains("pi") {
            AgentStorageRequest {
                bytes: 1024 * 1024 * 1024,
                items: 1,
            }
        } else {
            AgentStorageRequest::default()
        };
        let mut admission = self
            .admission
            .reserve(AgentAdmissionRequest::new_for_agent(
                agent_id,
                &request,
                identity.depth,
                identity.parent_profile.as_ref(),
                storage,
            ))
            .await?;

        let deadline = Some(fabric::MonoDeadline::after(
            self.clock.mono_now(),
            request.budget.max_elapsed_ms,
        ));
        let process = match self
            .kernel
            .spawn_process(SpawnSpec {
                agent_id,
                parent: request.parent_process_id,
                profile: request.profile_id.clone(),
                namespace: NamespaceId(request.root_agent_id.0.to_string()),
                initial_operation: None,
                deadline,
                ownership: fabric::ProcessOwnership::ThreadBackground {
                    thread_id: fabric::ThreadId(request.root_agent_id.0.to_string()),
                },
            })
            .await
        {
            Ok(process) => process,
            Err(error) => {
                let _ = admission.revoke().await;
                return Err(runtime_error(error));
            }
        };
        let process_snapshot = match self.kernel.inspect_process(process.id).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                let _ = self
                    .kernel
                    .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                    .await;
                let _ = admission.revoke().await;
                return Err(runtime_error(error));
            }
        };
        let root_process_id = identity.root_process_id.unwrap_or(process.id);
        let root_workspace_id = identity
            .root_workspace_id
            .unwrap_or_else(|| workspace_id.clone());
        self.kernel.upsert_space_binding(
            process_snapshot.space,
            ContextBinding::Agora(workspace_id.clone(), AgoraVersion(0)),
        );
        if let Err(error) = self.kernel.set_space_overlay(
            process_snapshot.space,
            "agent.workspace_receipt",
            serde_json::json!({
                "workspace_id": workspace_id,
                "root_process_id": root_process_id,
                "broadcast_refs": request.broadcast_refs,
            }),
        ) {
            let _ = self
                .kernel
                .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                .await;
            let _ = admission.revoke().await;
            return Err(runtime_error(error));
        }
        let operation = match self
            .kernel
            .submit_operation(OperationRequest {
                owner: process.id,
                parent: None,
                kind: OperationKind::SubAgent,
                deadline,
            })
            .await
        {
            Ok(operation) => operation,
            Err(error) => {
                let _ = self
                    .kernel
                    .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                    .await;
                let _ = admission.revoke().await;
                return Err(runtime_error(error));
            }
        };
        let mailbox_target = Target::from(format!("agent:{}", agent_id.0));
        let mailbox: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::with_capacity(MAILBOX_CAPACITY));
        if let Err(error) = self
            .kernel
            .register_process_mailbox(process.id, mailbox_target.clone(), mailbox.clone())
            .await
        {
            let _ = self
                .kernel
                .cancel_operation(
                    operation.id,
                    CancelReason::Other("mailbox setup failed".into()),
                )
                .await;
            let _ = self
                .kernel
                .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                .await;
            let _ = admission.revoke().await;
            return Err(runtime_error(error));
        }

        let handle = AgentHandle {
            agent_id,
            root_agent_id: request.root_agent_id,
            parent_agent_id: request.parent_agent_id,
            process_id: process.id,
            operation_id: operation.id,
            runtime_id: request.runtime_id.clone(),
            profile_id: request.profile_id.clone(),
        };
        let parent_projection_receipt = memory::context_projection_receipt(&context)?;
        let memory_context = mnemosyne::AgentMemoryContext::verified(
            process.id,
            agent_id,
            fabric::AgentTaskId(format!("task:{request_hash}")),
            parent_projection_receipt,
        )
        .map_err(|error| AgentControlError::invalid(error.to_string()))?;
        self.agent_memory_vault
            .register(&memory_context)
            .map_err(|error| {
                control_error(AgentControlErrorKind::Persistence, error.to_string())
            })?;
        let created_at_ms = self.clock.wall_now().0;
        let queued = AgentSnapshot {
            handle: handle.clone(),
            status: AgentRunStatus::Queued,
            result: None,
            created_at_ms,
            started_at_ms: None,
            ended_at_ms: None,
            last_error: None,
        };
        let record = AgentRunRecord {
            snapshot: queued.clone(),
            request: request.clone(),
            request_hash,
            workspace_id: workspace_id.clone(),
            root_process_id,
            broadcast_refs: request.broadcast_refs.clone(),
            version: 0,
            retain_until_ms: created_at_ms.saturating_add(DEFAULT_RETENTION_MS),
            resumability: launcher.resumability(),
            recovery: None,
        };
        if let Err(error) = self.repository.create(&record).await {
            let _ = self
                .kernel
                .cancel_operation(
                    operation.id,
                    CancelReason::Other("Agent persistence failed".into()),
                )
                .await;
            let _ = self
                .kernel
                .terminate_process(process.id, ExitReason::Failed(error.to_string()))
                .await;
            let _ = admission.revoke().await;
            return Err(error);
        }
        let lease_owner = format!("process:{}", process.id.0);
        let lease_expiry = created_at_ms.saturating_add(request.budget.max_elapsed_ms as i64);
        for (kind, label) in [
            (AgentResourceLeaseKind::Admission, "admission"),
            (AgentResourceLeaseKind::Mailbox, "mailbox"),
            (AgentResourceLeaseKind::Execution, "execution"),
        ] {
            self.repository
                .put_resource_lease(&AgentResourceLease {
                    lease_key: format!("{label}:{}", agent_id.0),
                    agent_id,
                    kind,
                    owner: lease_owner.clone(),
                    expires_at_ms: lease_expiry,
                    worktree_root: None,
                    worktree_path: None,
                    expected_head: None,
                })
                .await?;
        }

        let scope = OperationScope::new(operation.id);
        let cancellation = scope.token();
        let (mailbox_bridge, inbox) =
            AgentMailboxBridge::bounded(mailbox, MAILBOX_CAPACITY, cancellation.clone())?;
        let (snapshots, _) = watch::channel(queued);
        let live_run = LiveAgentRun::new(
            snapshots.clone(),
            mailbox_target,
            cancellation.clone(),
            request.background_decls.clone(),
            ReparentAuthority::new(
                request.trusted_workspace.clone(),
                request.allowed_tools.clone(),
                request.budget.clone(),
            ),
        )?;
        let mut background_cancellations = std::collections::HashMap::new();
        let mut background_registrations = std::collections::HashMap::new();
        let mut background_notification_targets = std::collections::HashMap::new();
        for declaration in &request.background_decls {
            let token = live_run
                .resource_cancellation(&declaration.resource_id)
                .await
                .ok_or_else(|| {
                    control_error(
                        AgentControlErrorKind::Runtime,
                        "reviewed background resource has no managed cancellation token",
                    )
                })?;
            background_cancellations.insert(declaration.resource_id.clone(), token);
            let registration = live_run
                .resource_registration(&declaration.resource_id)
                .ok_or_else(|| {
                    control_error(
                        AgentControlErrorKind::Runtime,
                        "reviewed background resource has no producer registration",
                    )
                })?;
            background_registrations.insert(declaration.resource_id.clone(), registration);
            if let Some(target) = live_run.notification_target(&declaration.resource_id) {
                background_notification_targets.insert(declaration.resource_id.clone(), target);
            }
        }
        let inserted = self.live.insert(agent_id, live_run).await;
        if !inserted {
            let _ = self
                .kernel
                .cancel_operation(
                    operation.id,
                    CancelReason::Other("duplicate live Agent".into()),
                )
                .await;
            let _ = self
                .kernel
                .terminate_process(
                    process.id,
                    ExitReason::Failed("duplicate live Agent".into()),
                )
                .await;
            let _ = admission.revoke().await;
            return Err(control_error(
                AgentControlErrorKind::Conflict,
                "Agent already has a live runtime",
            ));
        }

        let kernel = self.kernel.clone();
        let clock = self.clock.clone();
        let repository = self.repository.clone();
        let live = self.live.clone();
        let events = self.events.clone();
        let event_spine = self.event_spine.clone();
        let event_projections = self.event_projections.clone();
        let settlement_enabled = self.subagent_settlement;
        let settlement_generation = self.settlement_generation.clone();
        let settlement_receipts = self.settlement_receipts.clone();
        let settlement_metrics = self.settlement_metrics.clone();
        let lifecycle_hooks = self.lifecycle_hooks.clone();
        let runtime_input = AgentRuntimeInput {
            workspace: request.trusted_workspace.clone(),
            request,
            handle: handle.clone(),
            workspace_id,
            root_workspace_id,
            root_process_id,
            context,
            memory_context: memory_context.clone(),
            inbox,
            cancellation,
            background_cancellations,
            background_registrations,
            background_notification_targets,
        };
        let events: Arc<dyn AgentEventSink> = Arc::new(SpineAgentEventSink::new(
            events,
            event_spine.clone(),
            runtime_input.clone(),
            event_projections,
        ));
        let memory_events = Arc::new(MemoryRecordingAgentEventSink::new(
            events,
            self.agent_memory_vault.clone(),
            memory_context,
        ));
        let events: Arc<dyn AgentEventSink> = memory_events.clone();
        self.tasks.lock().await.spawn(async move {
            run_agent(
                kernel,
                clock,
                repository,
                live,
                launcher,
                events,
                runtime_input,
                mailbox_bridge,
                snapshots,
                scope,
                admission,
                settlement_enabled,
                settlement_generation,
                settlement_receipts,
                settlement_metrics,
                event_spine,
                memory_events,
                lifecycle_hooks,
            )
            .await;
        });
        Ok(handle)
    }

    async fn wait(&self, request: AgentWaitRequest) -> Result<AgentSnapshot, AgentControlError> {
        request.validate()?;
        self.wait_for_terminal(
            request.caller_root_agent_id,
            request.agent_id,
            Duration::from_millis(request.timeout_ms),
        )
        .await
    }

    async fn send(
        &self,
        request: AgentSendRequest,
    ) -> Result<AgentControlMessage, AgentControlError> {
        request.validate()?;
        let run = self
            .authorize(request.caller_root_agent_id, request.agent_id)
            .await?;
        if run.status().is_terminal() {
            return Err(control_error(
                AgentControlErrorKind::Terminal,
                "terminal Agent rejects new messages",
            ));
        }
        let live = self.live.get(request.agent_id).await.ok_or_else(|| {
            control_error(AgentControlErrorKind::Runtime, "Agent mailbox is not live")
        })?;
        let delivery_id = request.delivery_id.unwrap_or_else(uuid::Uuid::new_v4);
        let from = self.authorize_message_sender(&run, &request).await?;
        let payload = AgentMessagePayload {
            schema_version: fabric::AGENT_MESSAGE_SCHEMA_V1,
            kind: request.kind.clone(),
            content: request.message.clone(),
            start_turn: request.start_turn,
            correlation_id: request.correlation_id,
            deadline_mono_ms: request.deadline_mono_ms,
        };
        let message = self
            .repository
            .append_message(
                request.agent_id,
                from,
                delivery_id,
                &payload,
                self.clock.wall_now().0,
            )
            .await?;
        if message.delivery != AgentMessageDeliveryState::Pending {
            return Ok(AgentControlMessage {
                delivery_id,
                sequence: message.sequence,
                from,
                to: request.agent_id,
                kind: request.kind,
                delivery: message.delivery,
                content: request.message,
            });
        }
        let mut envelope = EnvelopeV2::new(
            SchemaId::from(SchemaId::AGENT_CONTROL_MESSAGE_V1),
            Target::from(format!("agent:{}", from.0)),
            live.mailbox_target,
            DeliveryPattern::Direct,
            NamespaceId(request.caller_root_agent_id.0.to_string()),
            serde_json::json!({
                "sequence": message.sequence,
                "delivery_id": delivery_id,
                "payload": payload,
                "start_turn": request.start_turn,
            }),
        )
        .with_operation_id(run.snapshot.handle.operation_id)
        .with_logical_time(message.sequence);
        envelope.id = fabric::ipc::envelope_v2::MessageId(delivery_id);
        if let Some(correlation) = request.correlation_id {
            envelope =
                envelope.with_correlation_id(fabric::ipc::envelope_v2::MessageId(correlation));
        }
        if let Some(deadline) = request.deadline_mono_ms {
            envelope = envelope.with_deadline(fabric::MonoDeadlineMillis(deadline));
        }
        let envelope = if request.kind == fabric::AgentMessageKind::Signal {
            envelope.with_priority(255)
        } else {
            envelope
        };
        let receipt = self.kernel.mailbox_service().route(envelope).await;
        let delivery = if receipt.is_ok() {
            AgentMessageDeliveryState::Delivered
        } else {
            AgentMessageDeliveryState::Rejected
        };
        let settled = self
            .repository
            .mark_message_delivery(request.agent_id, delivery_id, delivery)
            .await?;
        if !receipt.is_ok() {
            return Err(control_error(
                AgentControlErrorKind::Runtime,
                format!("Agent message delivery failed: {receipt:?}"),
            ));
        }
        Ok(AgentControlMessage {
            delivery_id,
            sequence: message.sequence,
            from,
            to: request.agent_id,
            kind: request.kind,
            delivery: settled.delivery,
            content: request.message,
        })
    }

    async fn cancel(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        let run = self.authorize(caller_root_agent_id, agent_id).await?;
        if run.status().is_terminal() {
            return Ok(run.snapshot);
        }
        let live = self.live.get(agent_id).await.ok_or_else(|| {
            control_error(AgentControlErrorKind::Runtime, "Agent runtime is not live")
        })?;
        // Cancel the authoritative live subtree, not only the requested node.
        // Runtime/model input cannot forge this topology: parent identities
        // come from persisted host-created Agent handles.
        let all_live = self.live.all().await;
        let mut cancelled = std::collections::HashSet::from([agent_id]);
        loop {
            let mut changed = false;
            for descendant in &all_live {
                let snapshot = descendant.snapshots.borrow();
                if snapshot
                    .handle
                    .parent_agent_id
                    .is_some_and(|parent| cancelled.contains(&parent))
                    && cancelled.insert(snapshot.handle.agent_id)
                {
                    changed = true;
                }
            }
            if !changed {
                break;
            }
        }
        for descendant in all_live {
            if cancelled.contains(&descendant.snapshots.borrow().handle.agent_id) {
                descendant.cancellation.cancel();
            }
        }
        live.cancellation.cancel();
        self.kernel
            .cancel_operation(run.snapshot.handle.operation_id, CancelReason::User)
            .await
            .map_err(runtime_error)?;
        self.wait_for_terminal(caller_root_agent_id, agent_id, CANCEL_WAIT)
            .await
    }

    async fn inspect(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        self.authorize(caller_root_agent_id, agent_id)
            .await
            .map(|record| record.snapshot)
    }

    async fn list(
        &self,
        request: AgentListRequest,
    ) -> Result<Vec<AgentSnapshot>, AgentControlError> {
        request.validate()?;
        self.repository
            .list_root(request.caller_root_agent_id, request.status, request.limit)
            .await
            .map(|records| records.into_iter().map(|record| record.snapshot).collect())
    }
}

#[allow(clippy::too_many_arguments)]
async fn run_agent(
    kernel: Arc<KernelRuntime>,
    clock: Arc<dyn Clock>,
    repository: Arc<dyn AgentRunRepository>,
    live: Arc<LiveAgentRuns>,
    launcher: Arc<dyn AgentRuntimeLauncher>,
    events: Arc<dyn AgentEventSink>,
    input: AgentRuntimeInput,
    mailbox_bridge: AgentMailboxBridge,
    snapshots: watch::Sender<AgentSnapshot>,
    mut scope: OperationScope,
    mut admission: Box<dyn AgentAdmissionLease>,
    settlement_enabled: bool,
    settlement_generation: String,
    settlement_receipts: Arc<dyn SettlementReceiptStore>,
    settlement_metrics: Arc<SettlementMetrics>,
    event_spine: Arc<dyn EventSpine>,
    memory_events: Arc<MemoryRecordingAgentEventSink>,
    lifecycle_hooks: Arc<dyn AgentLifecycleHookSink>,
) {
    let agent = input.handle.agent_id;
    let root_agent = input.handle.root_agent_id;
    let parent_agent = input.handle.parent_agent_id;
    let process = input.handle.process_id;
    let operation = input.handle.operation_id;
    let start = async {
        kernel.signal_process(process, ProcessSignal::Start).await?;
        kernel.start_operation(operation).await?;
        kernel
            .set_active_operation(process, Some(operation))
            .await?;
        anyhow::Ok(())
    }
    .await;
    if let Err(error) = start {
        if let Ok(record) = repository
            .transition(
                agent,
                AgentRunStatus::Queued,
                AgentRunStatus::Failed,
                None,
                Some(error.to_string()),
                clock.wall_now().0,
            )
            .await
        {
            snapshots.send_replace(record.snapshot);
        }
        let _ = kernel
            .cancel_operation(operation, CancelReason::Other("Agent start failed".into()))
            .await;
        let _ = kernel
            .terminate_process(process, ExitReason::Failed(error.to_string()))
            .await;
        let _ = admission.revoke().await;
        live.remove(agent).await;
        return;
    }
    let running = match repository
        .transition(
            agent,
            AgentRunStatus::Queued,
            AgentRunStatus::Running,
            None,
            None,
            clock.wall_now().0,
        )
        .await
    {
        Ok(record) => record,
        Err(error) => {
            let _ = kernel
                .cancel_operation(operation, CancelReason::Other("Agent state failed".into()))
                .await;
            let _ = kernel
                .terminate_process(process, ExitReason::Failed(error.to_string()))
                .await;
            let _ = admission.revoke().await;
            live.remove(agent).await;
            return;
        }
    };
    if let Err(error) = admission.mark_running().await {
        let _ = kernel
            .terminate_process(process, ExitReason::Failed(error.to_string()))
            .await;
        let _ = admission.revoke().await;
        live.remove(agent).await;
        return;
    }
    snapshots.send_replace(running.snapshot);
    lifecycle_hooks
        .emit(agent_lifecycle_hook_context(
            fabric::hook::HookPoint::SubagentStart,
            &input,
            "running",
        ))
        .await;
    let stop_hook_input = input.clone();
    let wants_reparent = input
        .request
        .background_decls
        .iter()
        .any(|resource| resource.survive_child);

    let (outcome_sender, outcome_receiver) = tokio::sync::oneshot::channel();
    scope.spawn("agent-mailbox", async move {
        match mailbox_bridge.run().await {
            Ok(()) => OperationExitReason::Completed,
            Err(error) => OperationExitReason::Failed(error.message),
        }
    });
    let task_cancel = scope.token();
    scope.spawn("agent-runtime", async move {
        let outcome = tokio::select! {
            _ = task_cancel.cancelled() => Err(control_error(AgentControlErrorKind::Terminal, "Agent runtime cancelled")),
            outcome = launcher.launch(input, events) => outcome,
        };
        let reason = match &outcome {
            Ok(_) => OperationExitReason::Completed,
            Err(error) if error.kind == AgentControlErrorKind::Terminal => {
                OperationExitReason::Cancelled(CancelReason::User)
            }
            Err(error) => OperationExitReason::Failed(error.message.clone()),
        };
        let _ = outcome_sender.send(outcome);
        reason
    });
    let task_exit = scope.join_next().await;
    let outcome = outcome_receiver.await.unwrap_or_else(|_| {
        Err(control_error(
            AgentControlErrorKind::Runtime,
            "Agent runtime task ended without an outcome",
        ))
    });
    let (next, result, error, process_exit) = match outcome {
        Ok(result) => (
            AgentRunStatus::Succeeded,
            Some(result),
            None,
            ExitReason::Completed,
        ),
        Err(error) if error.kind == AgentControlErrorKind::Terminal => (
            AgentRunStatus::Cancelled,
            None,
            Some(error.message),
            ExitReason::Cancelled("Agent runtime cancelled".into()),
        ),
        Err(error) => (
            AgentRunStatus::Failed,
            None,
            Some(error.message.clone()),
            ExitReason::Failed(error.message),
        ),
    };
    let settlement_usage = result.as_ref().map(|result| result.usage.clone());
    match next {
        AgentRunStatus::Succeeded => {
            let _ = kernel.succeed_operation(operation).await;
        }
        AgentRunStatus::Cancelled => {
            let _ = kernel.cancel_operation(operation, CancelReason::User).await;
        }
        AgentRunStatus::Failed => {
            let message = error
                .clone()
                .unwrap_or_else(|| "Agent runtime failed".into());
            let _ = kernel.fail_operation(operation, message).await;
        }
        _ => {}
    }
    if let Ok(record) = repository
        .transition(
            agent,
            AgentRunStatus::Running,
            next,
            result,
            error,
            clock.wall_now().0,
        )
        .await
    {
        snapshots.send_replace(record.snapshot);
    } else if let Some(exit) = task_exit {
        tracing::error!(agent = ?agent, reason = ?exit.reason, "failed to persist terminal Agent state");
    }
    lifecycle_hooks
        .emit(agent_lifecycle_hook_context(
            fabric::hook::HookPoint::SubagentStop,
            &stop_hook_input,
            match next {
                AgentRunStatus::Succeeded => "succeeded",
                AgentRunStatus::Cancelled => "cancelled",
                AgentRunStatus::Failed => "failed",
                _ => "terminal",
            },
        ))
        .await;
    let _ = kernel.terminate_process(process, process_exit).await;
    let lease_owner = format!("process:{}", process.0);
    if settlement_enabled {
        let terminal = match next {
            AgentRunStatus::Succeeded => fabric::SettlementTerminal::Completed,
            AgentRunStatus::Cancelled => fabric::SettlementTerminal::Cancelled,
            AgentRunStatus::Failed => fabric::SettlementTerminal::Failed {
                reason: "Agent runtime failed".into(),
            },
            _ => fabric::SettlementTerminal::Recoverable,
        };
        if let Some(live_run) = live.get(agent).await {
            let parent_run = match parent_agent {
                Some(parent) => live.get(parent).await,
                None => None,
            };
            // Both sides are host-minted, spawn-time authority envelopes.  A
            // live parent is also the notification/cancellation route; no
            // model-supplied settlement claim participates in this decision.
            let parent_authority_covers = parent_run.as_ref().is_some_and(|parent| {
                parent
                    .reparent_authority()
                    .covers(live_run.reparent_authority())
            });
            // Static maxima coverage is necessary; the authoritative proof is
            // the atomic BudgetController transfer receipt below.
            let _parent_budget_bounds_cover = parent_run.as_ref().is_some_and(|parent| {
                parent
                    .reparent_authority()
                    .accepts_budget(live_run.reparent_authority())
            });
            let parent_cancellation = parent_run.as_ref().map(|run| run.cancellation.clone());
            let parent_mailbox_target = parent_run.as_ref().map(|run| run.mailbox_target.clone());
            let evidence = Arc::new(SpineSettlementEvidenceSink::new(
                event_spine,
                root_agent.0.to_string(),
                agent.0.to_string(),
                operation,
            ));
            let managed_resources = Arc::new(ManagedSettlementResourcePort::new(
                live_run.clone(),
                parent_authority_covers,
                false,
                parent_cancellation,
                parent_mailbox_target,
            ));
            let engine = SettlementEngine::with_metrics(
                settlement_receipts,
                managed_resources.clone(),
                Arc::new(RepositorySettlementLeasePort::new(repository.clone())),
                evidence,
                settlement_metrics,
            );
            match engine.quiesce(&live_run).await {
                Ok(resources) => {
                    // Closing admission and fixing the resource snapshot must
                    // precede the irreversible budget ownership transfer. A
                    // crash before this point therefore leaves the reservation
                    // wholly child-owned and recoverable.
                    let budget_transfer_receipt = match (parent_agent, settlement_usage.as_ref()) {
                        (Some(parent), Some(usage))
                            if parent_authority_covers && wants_reparent =>
                        {
                            match admission.transfer_remaining_to(parent, usage).await {
                                Ok(receipt) => Some(receipt),
                                Err(error) => {
                                    tracing::warn!(agent = ?agent, %error, "parent budget rejected remaining child reservation");
                                    None
                                }
                            }
                        }
                        _ => None,
                    };
                    managed_resources.set_parent_budget_accepts(budget_transfer_receipt.is_some());
                    let mut terminal =
                        terminal_with_memory_flush(terminal, memory_events.take_error());
                    if budget_transfer_receipt.is_none() {
                        if let Err(error) =
                            settle_admission(&mut *admission, &terminal, settlement_usage.as_ref())
                                .await
                        {
                            tracing::error!(agent = ?agent, %error, "failed to settle Agent admission lease");
                            terminal = fabric::SettlementTerminal::Failed {
                                reason: format!(
                                    "Agent admission settlement failed: {}",
                                    error.message
                                ),
                            };
                        }
                    }
                    let request = SettlementRequest {
                        agent_id: agent.0.to_string(),
                        attempt_id: operation.0.to_string(),
                        generation: settlement_generation,
                        old_owner: lease_owner.clone(),
                        parent_owner: parent_agent.map(|parent| format!("agent:{}", parent.0)),
                        terminal,
                        lease_keys: ["admission", "mailbox", "execution"]
                            .into_iter()
                            .map(|label| format!("{label}:{}", agent.0))
                            .collect(),
                        settled_at_ms: clock.wall_now().0,
                    };
                    if let Err(error) = engine.settle(request, resources).await {
                        tracing::error!(agent = ?agent, %error, "Agent settlement state machine failed");
                    }
                }
                Err(error) => {
                    tracing::error!(agent = ?agent, %error, "Agent quiescing failed");
                    for resource in live_run.begin_quiescing().await {
                        let _ = live_run
                            .terminate_managed_resource(
                                &resource.resource_id,
                                &format!("quiesce-failed:{}", resource.resource_id),
                            )
                            .await;
                    }
                    let _ = admission.revoke().await;
                    for label in ["admission", "mailbox", "execution"] {
                        let _ = repository
                            .delete_resource_lease(&format!("{label}:{}", agent.0), &lease_owner)
                            .await;
                    }
                }
            }
        } else {
            let _ = admission.revoke().await;
            for label in ["admission", "mailbox", "execution"] {
                let _ = repository
                    .delete_resource_lease(&format!("{label}:{}", agent.0), &lease_owner)
                    .await;
            }
        }
    } else {
        // Even with the richer receipt/reparent state machine disabled, a
        // concrete registered producer must never outlive its child. Legacy
        // mode has no reparent protocol, so every declaration is cancelled
        // and awaited before releasing admission/leases.
        if let Some(live_run) = live.get(agent).await {
            for resource in live_run.begin_quiescing().await {
                let _ = live_run
                    .terminate_managed_resource(
                        &resource.resource_id,
                        &format!("legacy-terminal:{}", resource.resource_id),
                    )
                    .await;
            }
        }
        let settlement = match settlement_usage {
            Some(usage) if next == AgentRunStatus::Succeeded => {
                AgentAdmissionLease::settle(&mut *admission, &usage).await
            }
            _ => admission.revoke().await,
        };
        if let Err(error) = settlement {
            tracing::error!(agent = ?agent, %error, "failed to settle Agent admission lease");
        }
        for label in ["admission", "mailbox", "execution"] {
            let _ = repository
                .delete_resource_lease(&format!("{label}:{}", agent.0), &lease_owner)
                .await;
        }
    }
    live.remove(agent).await;
}

fn agent_lifecycle_hook_context(
    point: fabric::hook::HookPoint,
    input: &AgentRuntimeInput,
    status: &str,
) -> fabric::hook::HookContext {
    fabric::hook::HookContext {
        point,
        session_id: input.handle.root_agent_id.0.to_string(),
        turn_count: 0,
        tool_name: None,
        tool_input: None,
        tool_result: None,
        message: Some(input.request.task.clone()),
        metadata: std::collections::HashMap::from([
            ("agent_id".into(), input.handle.agent_id.0.to_string()),
            (
                "parent_agent_id".into(),
                input
                    .handle
                    .parent_agent_id
                    .map(|id| id.0.to_string())
                    .unwrap_or_default(),
            ),
            (
                "operation_id".into(),
                input.handle.operation_id.0.to_string(),
            ),
            ("status".into(), status.into()),
            (
                "workspace_root".into(),
                input
                    .workspace
                    .as_ref()
                    .map(|workspace| workspace.cwd().to_string_lossy().into_owned())
                    .unwrap_or_default(),
            ),
        ]),
    }
}

fn control_error(kind: AgentControlErrorKind, message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind,
        message: message.into(),
    }
}

fn runtime_error(error: impl std::fmt::Display) -> AgentControlError {
    control_error(AgentControlErrorKind::Runtime, error.to_string())
}
