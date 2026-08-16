use std::collections::HashMap;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use ::contracts::ipc::envelope_v2::{DeliveryPattern, EnvelopeV2, SchemaId, Target};
use ::contracts::{
    AgentControlError, AgentControlErrorKind, AgentControlMessage, AgentControlPort, AgentHandle,
    AgentId, AgentListRequest, AgentMessageDeliveryState, AgentMessagePayload, AgentRunStatus,
    AgentRuntimeCapability, AgentSendRequest, AgentSnapshot, AgentSpawnIntent, AgentSpawnRequest,
    AgentWaitRequest, AgentWorkspaceMode, AgoraVersion, CancelReason, Clock, ContextBinding,
    ExitReason, NamespaceId, OperationExitReason, OperationKind, OperationRequest, ProcessSignal,
    SettlementTerminal, SpawnSpec,
};
use async_trait::async_trait;
use kernel::operation::OperationScope;
use kernel::KernelRuntime;
use runtime::mailbox::{InProcessMailbox, Mailbox};
use runtime::EventSpine;
use tokio::sync::{watch, Mutex};
use tokio::task::JoinSet;
use tracing::info;

pub mod admission;
pub mod candidate_projection;
pub mod execution;
mod execution_runner;
pub mod memory;
mod runtime_bridge;
mod runtime_projection;
pub mod settlement;
mod spawning;

pub use adapters_sqlite::SqliteSettlementReceiptStore;
pub use admission::{
    AgentAdmissionLease, AgentAdmissionMetrics, AgentAdmissionPort, AgentAdmissionRequest,
    AgentStorageRequest, BoundedAgentAdmission,
};
pub use candidate_projection::{
    AgentCandidateProjector, AgentCandidateSubmissionPort, ProjectingAgentEventSink,
};
pub use execution::{
    AgentEventSink, AgentRecoveryRuntimeInput, AgentRuntimeEvent, AgentRuntimeInput,
    AgentRuntimeLauncher, BackgroundResourceRegistration, CognitiveTaskAdmissionPort,
    CompatibilityRuntimeLauncher, NoopAgentEventSink, SpineAgentEventSink,
};
use execution_runner::run_agent;
pub use memory::MemoryRecordingAgentEventSink;
pub(crate) use runtime::agent_spawn_request_hash;
pub use runtime::AgentLifecycleHookSink;
use runtime::RuntimeAgentStreamAdapter;
use runtime::{agent_lifecycle_hook_context, NoopAgentLifecycleHookSink};
pub use runtime::{
    agent_workspace_id, AgentMessageRecord, AgentResourceLease, AgentResourceLeaseKind,
    AgentRunProjection, AgentRunRecord, AgentTerminalReceipt,
};
pub use runtime::{
    reduce_agent_lifecycle, reduce_agent_status_transition, AgentLifecycleEffect,
    AgentLifecycleEvent, AgentLifecycleTransition, AgentRecoveryCoordinator,
    AgentRecoveryObservation, AgentRecoveryReport, FailClosedRuntimeProcessSupervisor,
    InvalidAgentLifecycleTransition, RuntimeProcessReclaimOutcome, RuntimeProcessSupervisor,
    MAX_STARTUP_RECOVERY_ROWS,
};
pub(crate) use runtime::{runtime_capability, ValidatedAgentIdentity};
pub use runtime::{
    AgentCleanupCoordinator, AgentCleanupReport, AgentWorktreeReclaimer, MAX_CLEANUP_BATCH,
};
pub use runtime::{
    AgentContextItem, AgentContextItemKind, AgentContextProjection, AgentContextProjectionBuilder,
};
pub use runtime::{AgentMailboxBridge, AgentRuntimeInbox};
pub use runtime::{LiveAgentRun, LiveAgentRuns, ReparentAuthority};
pub use runtime_projection::RuntimeAgentRunProjection;
pub use settlement::{
    recovery_disposition, settle_admission, terminal_with_memory_flush,
    FailClosedSettlementResourcePort, InMemorySettlementReceiptStore,
    ManagedSettlementResourcePort, NoopSettlementEvidenceSink, RecoveryResourceDisposition,
    RepositorySettlementLeasePort, SettlementEngine, SettlementEvidence, SettlementEvidenceSink,
    SettlementLeasePort, SettlementMetricSnapshot, SettlementMetrics, SettlementReceiptStore,
    SettlementRequest, SettlementResourcePort, SpineSettlementEvidenceSink,
};

const DEFAULT_RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
const MAILBOX_CAPACITY: usize = 64;
const CANCEL_WAIT: Duration = Duration::from_secs(30);

pub use runtime::{AgentWaitTimer, SystemAgentWaitTimer};

/// Aletheon host adapter around the Runtime-owned Agent supervisor.
///
/// This type does not own Agent identity, generation, lifecycle reduction, or
/// terminal settlement. It resolves host policy, performs Kernel/process and
/// mailbox effects, and projects Runtime receipts for the public Agent tools.
/// The compatibility constructor is retained only for isolated fixtures.
pub struct AgentHostAdapter {
    kernel: Arc<KernelRuntime>,
    clock: Arc<dyn Clock>,
    repository: Arc<dyn AgentRunProjection>,
    admission: Arc<dyn AgentAdmissionPort>,
    /// Legacy launcher catalog retained only for isolated compatibility
    /// fixtures. Production composition uses Runtime's DelegateBackendRegistry
    /// and constructs this service without a second catalog.
    runtimes: Option<Arc<dyn CompatibilityRuntimeCatalog>>,
    events: Arc<dyn AgentEventSink>,
    event_spine: Arc<dyn EventSpine>,
    event_projections: Arc<dyn runtime::read_model::EventProjectionSink>,
    timer: Arc<dyn AgentWaitTimer>,
    live: Arc<LiveAgentRuns>,
    tasks: Mutex<JoinSet<()>>,
    topology_routes: Arc<runtime::AgentTopologyRoutes>,
    agent_memory_vault: Arc<mnemosyne::AgentMemoryVault>,
    durable_memory: Option<Arc<dyn mnemosyne::MemoryService>>,
    settlement_generation: String,
    settlement_receipts: Arc<dyn SettlementReceiptStore>,
    settlement_metrics: Arc<SettlementMetrics>,
    budget_controller: Option<Arc<dyn kernel::BudgetController>>,
    lifecycle_hooks: Arc<dyn AgentLifecycleHookSink>,
    selection_policy: runtime::AgentRuntimeSelectionPolicy,
    cognitive_task_admission: Option<Arc<dyn CognitiveTaskAdmissionPort>>,
    runtime_process_supervisor: Arc<dyn RuntimeProcessSupervisor>,
    /// Runtime AgentSupervisor owns child identity and the durable lifecycle
    /// receipt. This adapter retains only policy, host effects, and projection
    /// access around that authority.
    runtime_agent_supervisor: Option<Arc<runtime::RuntimeAgentSupervisor>>,
}

/// Production-facing Agent command/query facade.
///
/// The facade deliberately exposes only the stable `AgentControlPort`. Runtime
/// backend registration receives `AgentHostEffects` from `AgentHostAdapter`
/// directly, so public tools cannot retain or re-enter the concrete host
/// effects object.
pub struct RuntimeAgentControlFacade {
    host: Arc<AgentHostAdapter>,
}

impl RuntimeAgentControlFacade {
    pub fn new(host: Arc<AgentHostAdapter>) -> Self {
        Self { host }
    }
}

/// Composition-facing lifecycle view of the rich host adapter.
///
/// Production bootstrap uses this view for recovery, shutdown, and the live
/// run cancellation tree instead of retaining the concrete host adapter as a
/// general-purpose service locator.
pub struct RuntimeAgentLifecycleFacade {
    host: Arc<AgentHostAdapter>,
}

impl RuntimeAgentLifecycleFacade {
    pub fn new(host: Arc<AgentHostAdapter>) -> Self {
        Self { host }
    }

    pub async fn reconcile_startup(
        &self,
        daemon_generation: &str,
    ) -> Result<AgentRecoveryReport, AgentControlError> {
        self.host.reconcile_startup(daemon_generation).await
    }

    pub fn live_runs(&self) -> Arc<LiveAgentRuns> {
        self.host.live_runs()
    }

    pub async fn shutdown(&self) {
        self.host.shutdown().await;
    }
}

/// The only production projections of the rich host composition object.
pub struct AgentHostFacades {
    pub control: Arc<dyn AgentControlPort>,
    pub effects: Arc<dyn AgentHostEffects>,
    pub lifecycle: Arc<RuntimeAgentLifecycleFacade>,
}

impl AgentHostAdapter {
    /// Consume the production composition object and expose only its narrow
    /// command, host-effect, and lifecycle views.
    ///
    /// The borrowing variant remains available for compatibility fixtures
    /// that inspect adapter-only metrics.
    pub fn into_facades(self) -> AgentHostFacades {
        Arc::new(self).facades()
    }

    pub fn facades(self: &Arc<Self>) -> AgentHostFacades {
        AgentHostFacades {
            control: Arc::new(RuntimeAgentControlFacade::new(self.clone())),
            effects: self.clone(),
            lifecycle: Arc::new(RuntimeAgentLifecycleFacade::new(self.clone())),
        }
    }
}

impl std::fmt::Debug for RuntimeAgentControlFacade {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("RuntimeAgentControlFacade")
            .finish_non_exhaustive()
    }
}

/// Narrow compatibility-only launcher catalog boundary. Production runtime
/// selection and lifecycle use `RuntimeAgentSupervisor`; this port exists only
/// for explicit rollback/fixture construction and keeps the rich host service
/// independent of the concrete legacy registry implementation.
pub trait CompatibilityRuntimeCatalog: Send + Sync {
    fn catalog(&self) -> Vec<runtime::RuntimeManifest>;
    fn select(
        &self,
        request: &runtime::RuntimeSelectionRequest,
    ) -> Result<
        (
            ::contracts::RuntimeId,
            Arc<dyn AgentRuntimeLauncher>,
            runtime::RuntimeSelectionDecision,
        ),
        AgentControlError,
    >;
    fn resolve(
        &self,
        id: &::contracts::RuntimeId,
    ) -> Result<Arc<dyn AgentRuntimeLauncher>, AgentControlError>;
}

pub use runtime_bridge::{
    AgentHostEffects, DurableRuntimeProcessRegistration, RuntimeObservedAgentBackend,
};

impl std::fmt::Debug for AgentHostAdapter {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("AgentHostAdapter")
            .finish_non_exhaustive()
    }
}

/// Aletheon host adapter for the Runtime-owned startup recovery coordinator.
/// It observes Kernel/process state and performs concrete resource/backend
/// actions; Runtime remains responsible for decisions, durable transitions,
/// pagination, and the unreconciled fence.
struct StartupRecoveryHost<'a> {
    service: &'a AgentHostAdapter,
    daemon_generation: &'a str,
    orphan_reclaimed: AtomicUsize,
    orphan_already_exited: AtomicUsize,
    orphan_identity_reused: AtomicUsize,
}

#[async_trait]
impl runtime::AgentRecoveryHost for StartupRecoveryHost<'_> {
    async fn observe(
        &self,
        run: &AgentRunRecord,
    ) -> Result<AgentRecoveryObservation, AgentControlError> {
        if let Some(identity) = self
            .service
            .repository
            .runtime_process(run.agent_id())
            .await?
        {
            let outcome = self
                .service
                .runtime_process_supervisor
                .reclaim(identity)
                .await?;
            match outcome {
                RuntimeProcessReclaimOutcome::Reclaimed => {
                    self.orphan_reclaimed.fetch_add(1, Ordering::Relaxed);
                }
                RuntimeProcessReclaimOutcome::AlreadyExited => {
                    self.orphan_already_exited.fetch_add(1, Ordering::Relaxed);
                }
                RuntimeProcessReclaimOutcome::IdentityReused => {
                    self.orphan_identity_reused.fetch_add(1, Ordering::Relaxed);
                }
            }
            self.service
                .repository
                .clear_runtime_process(&identity)
                .await?;
            tracing::info!(
                agent_id = %identity.agent_id.0,
                process_id = %identity.process_id.0,
                os_pid = identity.os_pid.0,
                ?outcome,
                "reconciled durable external runtime process"
            );
        }
        let process_live = self
            .service
            .kernel
            .inspect_process(run.snapshot.handle.process_id)
            .await
            .is_ok();
        let operation_terminal = self
            .service
            .kernel
            .inspect_operation(run.snapshot.handle.operation_id)
            .await
            .ok()
            .and_then(|operation| match operation.state {
                ::contracts::OperationState::Succeeded => Some(AgentRunStatus::Succeeded),
                ::contracts::OperationState::Failed => Some(AgentRunStatus::Failed),
                ::contracts::OperationState::Cancelled => Some(AgentRunStatus::Cancelled),
                _ => None,
            });
        let checkpoint_available = matches!(
            &run.resumability,
            ::contracts::RuntimeResumability::Checkpointed { reference }
                if !reference.trim().is_empty()
        );
        Ok(AgentRecoveryObservation {
            process_live,
            operation_terminal,
            checkpoint_available,
        })
    }

    async fn apply(
        &self,
        run: &AgentRunRecord,
        decision: ::contracts::AgentRecoveryDecision,
        observation: AgentRecoveryObservation,
    ) -> Result<bool, AgentControlError> {
        match decision {
            ::contracts::AgentRecoveryDecision::Interrupt
            | ::contracts::AgentRecoveryDecision::Finalize => {
                self.service
                    .recover_settlement_resources(
                        run,
                        decision,
                        self.daemon_generation,
                        observation.operation_terminal,
                    )
                    .await?;
                Ok(false)
            }
            ::contracts::AgentRecoveryDecision::Resume => {
                let checkpoint_reference = match &run.resumability {
                    ::contracts::RuntimeResumability::Checkpointed { reference } => {
                        reference.clone()
                    }
                    ::contracts::RuntimeResumability::Never => return Ok(false),
                };
                let resumed = if let Some(supervisor) = &self.service.runtime_agent_supervisor {
                    let agent_run = runtime::AgentRunId(run.snapshot.handle.agent_id.0.to_string());
                    match supervisor.generation(&agent_run) {
                        Ok(generation) => supervisor
                            .resume_from_checkpoint(&agent_run, &generation, checkpoint_reference)
                            .await
                            .is_ok(),
                        Err(_) => false,
                    }
                } else {
                    match self
                        .service
                        .runtimes
                        .as_ref()
                        .and_then(|runtimes| runtimes.resolve(&run.snapshot.handle.runtime_id).ok())
                    {
                        Some(runtime)
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
                            true
                        }
                        _ => false,
                    }
                };
                Ok(resumed)
            }
            ::contracts::AgentRecoveryDecision::Reclaim => Ok(false),
        }
    }

    fn process_reclaim_counts(&self) -> (usize, usize, usize) {
        (
            self.orphan_reclaimed.load(Ordering::Relaxed),
            self.orphan_already_exited.load(Ordering::Relaxed),
            self.orphan_identity_reused.load(Ordering::Relaxed),
        )
    }
}

#[async_trait]
impl runtime::AgentProjectionPresence for StartupRecoveryHost<'_> {
    async fn has_projection(
        &self,
        agent_run: &runtime::AgentRunId,
    ) -> Result<bool, runtime::RuntimeError> {
        let agent_id = uuid::Uuid::parse_str(&agent_run.0)
            .map(AgentId)
            .map_err(|_| runtime::RuntimeError::UnsupportedRequest)?;
        self.service
            .repository
            .get(agent_id)
            .await
            .map(|record| record.is_some())
            .map_err(|_| runtime::RuntimeError::Internal)
    }
}

impl AgentHostAdapter {
    #[doc(hidden)]
    pub async fn spawn_intent(
        &self,
        intent: AgentSpawnIntent,
    ) -> Result<AgentHandle, AgentControlError> {
        self.control_spawn_intent(intent).await
    }

    #[doc(hidden)]
    pub async fn spawn(
        &self,
        request: AgentSpawnRequest,
    ) -> Result<AgentHandle, AgentControlError> {
        self.control_spawn(request).await
    }

    #[doc(hidden)]
    pub async fn wait(
        &self,
        request: AgentWaitRequest,
    ) -> Result<AgentSnapshot, AgentControlError> {
        self.control_wait(request).await
    }

    #[doc(hidden)]
    pub async fn send(
        &self,
        request: AgentSendRequest,
    ) -> Result<AgentControlMessage, AgentControlError> {
        self.control_send(request).await
    }

    #[doc(hidden)]
    pub async fn cancel(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        self.control_cancel(caller_root_agent_id, agent_id).await
    }

    #[doc(hidden)]
    pub async fn inspect(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        self.control_inspect(caller_root_agent_id, agent_id).await
    }

    #[doc(hidden)]
    pub async fn list(
        &self,
        request: AgentListRequest,
    ) -> Result<Vec<AgentSnapshot>, AgentControlError> {
        self.control_list(request).await
    }

    #[doc(hidden)]
    pub fn new_legacy(
        kernel: Arc<KernelRuntime>,
        clock: Arc<dyn Clock>,
        repository: Arc<dyn AgentRunProjection>,
        admission: Arc<dyn AgentAdmissionPort>,
        runtimes: Arc<dyn CompatibilityRuntimeCatalog>,
        event_spine: Arc<dyn EventSpine>,
    ) -> Self {
        let mut service = Self::new_base(kernel, clock, repository, admission, event_spine);
        service.runtimes = Some(runtimes);
        service
    }

    /// Production constructor. Runtime is the only backend selection and
    /// lifecycle authority; no Aletheon launcher registry is composed.
    pub fn new_runtime_only(
        kernel: Arc<KernelRuntime>,
        clock: Arc<dyn Clock>,
        repository: Arc<dyn AgentRunProjection>,
        admission: Arc<dyn AgentAdmissionPort>,
        event_spine: Arc<dyn EventSpine>,
        runtime_agent_supervisor: Arc<runtime::RuntimeAgentSupervisor>,
    ) -> Self {
        let mut service = Self::new_base(kernel, clock, repository, admission, event_spine);
        service.runtime_agent_supervisor = Some(runtime_agent_supervisor);
        service
    }

    fn new_base(
        kernel: Arc<KernelRuntime>,
        clock: Arc<dyn Clock>,
        repository: Arc<dyn AgentRunProjection>,
        admission: Arc<dyn AgentAdmissionPort>,
        event_spine: Arc<dyn EventSpine>,
    ) -> Self {
        Self {
            kernel,
            clock,
            repository,
            admission,
            runtimes: None,
            events: Arc::new(NoopAgentEventSink),
            event_spine,
            event_projections: Arc::new(runtime::read_model::NoopEventProjectionSink),
            timer: Arc::new(SystemAgentWaitTimer),
            live: Arc::new(LiveAgentRuns::default()),
            tasks: Mutex::new(JoinSet::new()),
            topology_routes: Arc::new(runtime::AgentTopologyRoutes::default()),
            agent_memory_vault: Arc::new(
                mnemosyne::AgentMemoryVault::in_memory().expect("in-memory Agent memory vault"),
            ),
            durable_memory: None,
            settlement_generation: "disabled".into(),
            settlement_receipts: Arc::new(InMemorySettlementReceiptStore::default()),
            settlement_metrics: Arc::new(SettlementMetrics::default()),
            budget_controller: None,
            lifecycle_hooks: Arc::new(NoopAgentLifecycleHookSink),
            selection_policy: runtime::AgentRuntimeSelectionPolicy::default(),
            cognitive_task_admission: None,
            runtime_process_supervisor: Arc::new(FailClosedRuntimeProcessSupervisor),
            runtime_agent_supervisor: None,
        }
    }

    pub fn with_runtime_profile_requirements(
        mut self,
        requirements: HashMap<::contracts::AgentProfileId, Vec<AgentRuntimeCapability>>,
    ) -> Self {
        self.selection_policy.set_runtime_requirements(requirements);
        self
    }

    pub fn with_agent_profiles(
        mut self,
        profiles: HashMap<String, ::contracts::AgentProfile>,
    ) -> Self {
        self.selection_policy.set_profiles(profiles);
        self
    }

    pub fn with_agent_profile_resolver(
        mut self,
        resolver: Arc<runtime::AgentProfileResolver>,
    ) -> Self {
        self.selection_policy.set_profile_resolver(resolver);
        self
    }

    fn resolve_agent_profile(
        &self,
        id: &::contracts::AgentProfileId,
    ) -> Option<::contracts::AgentProfile> {
        self.selection_policy.resolve_profile(id)
    }

    pub fn with_cognitive_task_admission(
        mut self,
        admission: Arc<dyn CognitiveTaskAdmissionPort>,
    ) -> Self {
        self.cognitive_task_admission = Some(admission);
        self
    }

    pub fn with_capability_history(
        mut self,
        history: Arc<dyn runtime::RuntimePreferenceHistory>,
    ) -> Self {
        self.selection_policy.set_capability_history(history);
        self
    }

    pub fn with_lifecycle_hooks(mut self, hooks: Arc<dyn AgentLifecycleHookSink>) -> Self {
        self.lifecycle_hooks = hooks;
        self
    }

    pub fn with_runtime_process_supervisor(
        mut self,
        supervisor: Arc<dyn RuntimeProcessSupervisor>,
    ) -> Self {
        self.runtime_process_supervisor = supervisor;
        self
    }

    /// Runtime is the production selection authority. The compatibility registry
    /// is retained only as a launcher/admission adapter for isolated
    /// paths and test fixtures without a composed Runtime supervisor.
    fn runtime_catalog(&self) -> Vec<runtime::RuntimeManifest> {
        self.runtime_agent_supervisor
            .as_ref()
            .map(|supervisor| supervisor.catalog())
            .unwrap_or_else(|| {
                self.runtimes
                    .as_ref()
                    .map(|runtimes| runtimes.catalog())
                    .unwrap_or_default()
            })
    }

    fn select_runtime(
        &self,
        request: &runtime::RuntimeSelectionRequest,
    ) -> Result<(::contracts::RuntimeId, runtime::RuntimeSelectionDecision), AgentControlError>
    {
        if let Some(supervisor) = &self.runtime_agent_supervisor {
            let (id, decision) = supervisor.select(request).map_err(|error| {
                control_error(
                    AgentControlErrorKind::NotFound,
                    format!("Runtime backend selection failed: {error}"),
                )
            })?;
            return Ok((::contracts::RuntimeId(id.0), decision));
        }
        self.runtimes
            .as_ref()
            .ok_or_else(|| {
                control_error(
                    AgentControlErrorKind::NotFound,
                    "Runtime supervisor is not composed",
                )
            })?
            .select(request)
            .map(|(id, _launcher, decision)| (id, decision))
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
        projections: Arc<dyn runtime::read_model::EventProjectionSink>,
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

    pub fn with_durable_memory(mut self, memory: Arc<dyn mnemosyne::MemoryService>) -> Self {
        self.durable_memory = Some(memory);
        self
    }

    pub fn with_subagent_settlement(
        mut self,
        generation: impl Into<String>,
        receipts: Arc<dyn SettlementReceiptStore>,
    ) -> Self {
        self.settlement_generation = generation.into();
        self.settlement_receipts = receipts;
        self
    }

    pub fn with_budget_controller(mut self, budget: Arc<dyn kernel::BudgetController>) -> Self {
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
        let host = StartupRecoveryHost {
            service: self,
            daemon_generation,
            orphan_reclaimed: AtomicUsize::new(0),
            orphan_already_exited: AtomicUsize::new(0),
            orphan_identity_reused: AtomicUsize::new(0),
        };
        let mut report = coordinator.reconcile_with(&host).await?;
        if let Some(supervisor) = &self.runtime_agent_supervisor {
            let orphaned = supervisor
                .reconcile_missing_projections(&host)
                .await
                .map_err(|error| {
                    control_error(
                        AgentControlErrorKind::Persistence,
                        format!("Runtime Agent orphan reconciliation failed: {error}"),
                    )
                })?;
            report.interrupted = report.interrupted.saturating_add(orphaned);
        }
        coordinator.refresh_unreconciled(&mut report).await?;
        Ok(report)
    }

    async fn recover_settlement_resources(
        &self,
        run: &AgentRunRecord,
        decision: ::contracts::AgentRecoveryDecision,
        daemon_generation: &str,
        operation_terminal: Option<AgentRunStatus>,
    ) -> Result<(), AgentControlError> {
        match recovery_disposition(decision) {
            RecoveryResourceDisposition::RetainForResume => return Ok(()),
            RecoveryResourceDisposition::ReplaySettlement
            | RecoveryResourceDisposition::TerminateAndReclaim => {}
        }
        // Runtime owns the restart terminal fence. The SQL repository below
        // is updated only as a compatibility projection after this receipt is
        // durable; an old SQL row without an AgentStream start is imported as
        // a generation-1 legacy observation.
        if let Some(supervisor) = &self.runtime_agent_supervisor {
            let agent_run = runtime::AgentRunId(run.agent_id().0.to_string());
            let existing_terminal = supervisor.terminal(&agent_run);
            let generation = supervisor
                .generation(&agent_run)
                .unwrap_or(runtime::Generation(1));
            if existing_terminal.is_none() && supervisor.generation(&agent_run).is_err() {
                supervisor
                    .record_started(
                        runtime::SessionId(run.root_agent_id().0.to_string()),
                        agent_run.clone(),
                        generation,
                        Some(run.snapshot.handle.runtime_id.0.clone()),
                    )
                    .await
                    .map_err(|error| {
                        control_error(
                            AgentControlErrorKind::Persistence,
                            format!("Runtime Agent recovery admission failed: {error}"),
                        )
                    })?;
            }
            let terminal =
                existing_terminal
                    .clone()
                    .unwrap_or_else(|| match (decision, operation_terminal) {
                        (
                            ::contracts::AgentRecoveryDecision::Finalize,
                            Some(AgentRunStatus::Failed),
                        ) => runtime::TurnTerminal::Failed {
                            message: "Kernel completed Agent operation as failed during recovery"
                                .into(),
                        },
                        (
                            ::contracts::AgentRecoveryDecision::Finalize,
                            Some(AgentRunStatus::Cancelled),
                        ) => runtime::TurnTerminal::Interrupted,
                        (::contracts::AgentRecoveryDecision::Finalize, _) => {
                            runtime::TurnTerminal::Completed
                        }
                        _ => runtime::TurnTerminal::Interrupted,
                    });
            if existing_terminal.is_none() {
                supervisor
                    .record_settled(&agent_run, terminal)
                    .await
                    .map_err(|error| {
                        control_error(
                            AgentControlErrorKind::Persistence,
                            format!("Runtime Agent recovery settlement failed: {error}"),
                        )
                    })?;
            }
        }
        if let Some(budget) = &self.budget_controller {
            let owner = format!("agent:{}", run.agent_id().0);
            if let Some(reservation) = budget.reservation_for_owner(&owner).await {
                // A durable transfer is authoritative and must never be
                // reversed by settlement recovery. Otherwise reclaiming the
                // live child reservation is owner-scoped and idempotent.
                if budget.transfer_for_child(reservation).await.is_none() {
                    match budget.revoke_reservation(reservation).await {
                        Ok(()) | Err(::contracts::AdmissionError::AlreadySettled) => {}
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
        let terminal = match (
            self.runtime_agent_supervisor
                .as_ref()
                .and_then(|supervisor| {
                    supervisor.terminal(&runtime::AgentRunId(run.agent_id().0.to_string()))
                }),
            decision,
            operation_terminal,
        ) {
            (Some(runtime::TurnTerminal::Failed { message }), _, _) => {
                SettlementTerminal::Failed { reason: message }
            }
            (Some(runtime::TurnTerminal::Interrupted), _, _) => SettlementTerminal::Cancelled,
            (Some(runtime::TurnTerminal::Completed), _, _) => SettlementTerminal::Completed,
            (None, ::contracts::AgentRecoveryDecision::Finalize, Some(AgentRunStatus::Failed)) => {
                SettlementTerminal::Failed {
                    reason: "Kernel completed Agent operation as failed during recovery".into(),
                }
            }
            (
                None,
                ::contracts::AgentRecoveryDecision::Finalize,
                Some(AgentRunStatus::Cancelled),
            ) => SettlementTerminal::Cancelled,
            (None, ::contracts::AgentRecoveryDecision::Finalize, _) => {
                SettlementTerminal::Completed
            }
            (None, _, _) => SettlementTerminal::Failed {
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
        )
        .with_generation(daemon_generation);
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
        self.topology_routes.permit(parent, from, to);
        Ok(())
    }

    /// Authorize one visibility-filtered broadcast item against both durable
    /// receipts and the child's current Kernel context-space binding.
    pub async fn authorize_broadcast(
        &self,
        caller_root: AgentId,
        agent: AgentId,
        epoch: ::contracts::BroadcastEpoch,
        candidate: &::contracts::WorkspaceCandidate,
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
        // Freeze Runtime admission and cancel its pinned active children
        // before waiting on host tasks. Waiting first can deadlock graceful
        // daemon shutdown: the connection task is waiting for the Agent
        // terminal while the Runtime child is still running.
        if let Some(supervisor) = &self.runtime_agent_supervisor {
            match supervisor.drain_active().await {
                Ok(drained) => {
                    tracing::info!(
                        count = drained.len(),
                        "Runtime Agent children drained during shutdown"
                    );
                }
                Err(error) => {
                    tracing::error!(%error, "Runtime Agent child drain failed during shutdown");
                }
            }
        }
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
                    .topology_routes
                    .is_permitted(sender_parent, sender, target.agent_id())
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
        runtime_identity: Option<runtime::DelegateReceipt>,
    ) -> Result<ValidatedAgentIdentity, AgentControlError> {
        match (request.parent_agent_id, request.parent_process_id) {
            (None, None) => {
                let (agent_id, runtime_generation) = if let Some(receipt) = runtime_identity {
                    let id = uuid::Uuid::parse_str(&receipt.agent_run.0).map_err(|_| {
                        control_error(
                            AgentControlErrorKind::Runtime,
                            "Runtime Agent identity was not a Fabric UUID",
                        )
                    })?;
                    (AgentId(id), Some(receipt.generation))
                } else {
                    (request.root_agent_id, None)
                };
                Ok(ValidatedAgentIdentity {
                    agent_id,
                    runtime_generation,
                    root_process_id: None,
                    root_workspace_id: None,
                    depth: 0,
                    parent_profile: None,
                })
            }
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
                    .unwrap_or_else(|| {
                        ::contracts::AgoraSpaceId(request.root_agent_id.0.to_string())
                    });
                let runtime_identity = runtime_identity.or_else(|| {
                    self.runtime_agent_supervisor
                        .as_ref()
                        .map(|supervisor| supervisor.mint_legacy_agent_identity())
                });
                let (agent_id, runtime_generation) = match runtime_identity {
                    Some(receipt) => {
                        let uuid = uuid::Uuid::parse_str(&receipt.agent_run.0).map_err(|_| {
                            control_error(
                                AgentControlErrorKind::Runtime,
                                "Runtime Agent identity was not a Fabric UUID",
                            )
                        })?;
                        (AgentId(uuid), Some(receipt.generation))
                    }
                    None => (AgentId(runtime::mint_agent_run_uuid()), None),
                };
                Ok(ValidatedAgentIdentity {
                    // Runtime owns the logical child identity. The fabric
                    // AgentId remains the compatibility wire representation.
                    agent_id,
                    runtime_generation,
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

    /// Compatibility execution path used only by the Runtime observed
    /// adapter. Runtime owns the outer wait fence; this helper reads the rich
    /// Fabric snapshot without re-entering RuntimeAgentSupervisor.
    async fn wait_local(
        &self,
        caller_root: AgentId,
        agent: AgentId,
        timeout: Duration,
    ) -> Result<AgentSnapshot, AgentControlError> {
        self.wait_for_terminal(caller_root, agent, timeout).await
    }

    /// Host execution path used by RuntimeAgentSupervisor's observed backend.
    /// It preserves the existing subtree cancellation semantics while keeping
    /// the public AgentControl entry from recursively calling Runtime.
    async fn cancel_local(
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
    pub(crate) async fn register_agent_mailbox(
        &self,
        process_id: ::contracts::ProcessId,
        target: Target,
        mailbox: Arc<dyn Mailbox>,
    ) -> anyhow::Result<()> {
        self.kernel
            .register_process_mailbox(process_id, target, mailbox)
            .await
    }
}

impl AgentHostAdapter {
    async fn send_local(
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
            schema_version: ::contracts::AGENT_MESSAGE_SCHEMA_V1,
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
        envelope.id = ::contracts::ipc::envelope_v2::MessageId(delivery_id);
        if let Some(correlation) = request.correlation_id {
            envelope =
                envelope.with_correlation_id(::contracts::ipc::envelope_v2::MessageId(correlation));
        }
        if let Some(deadline) = request.deadline_mono_ms {
            envelope = envelope.with_deadline(::contracts::MonoDeadlineMillis(deadline));
        }
        let envelope = if request.kind == ::contracts::AgentMessageKind::Signal {
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
}

impl AgentHostAdapter {
    async fn control_spawn_intent(
        &self,
        mut intent: AgentSpawnIntent,
    ) -> Result<AgentHandle, AgentControlError> {
        intent.validate()?;
        if let Some(profile) = self.resolve_agent_profile(&intent.profile_id) {
            if intent.allowed_tools.is_empty() {
                // The host, not the model, resolves the target profile's
                // callable set. This prevents the common "child started with
                // zero tools" failure while preserving parent attenuation.
                intent.allowed_tools = profile.allowed_tools.clone();
            } else if let Some(tool) = intent
                .allowed_tools
                .iter()
                .find(|tool| !profile.allowed_tools.contains(*tool))
            {
                return Err(control_error(
                    AgentControlErrorKind::Forbidden,
                    format!(
                        "requested child tool '{tool}' is outside profile '{}'",
                        intent.profile_id.0
                    ),
                ));
            }
        }
        intent.validate()?;
        let mut required_capabilities = self
            .selection_policy
            .runtime_requirements(&intent.profile_id);
        required_capabilities.extend(intent.required_capabilities.iter().cloned());
        required_capabilities.sort();
        required_capabilities.dedup();

        let workspace_mode = match intent.trusted_workspace.as_ref() {
            Some(workspace) if !workspace.writable_roots().is_empty() => {
                AgentWorkspaceMode::SharedWritable
            }
            Some(_) => AgentWorkspaceMode::SharedReadOnly,
            None => AgentWorkspaceMode::WorkspaceLess,
        };
        let runtime_catalog = self.runtime_catalog();
        let history_runtime = if intent.runtime_override.is_none() {
            self.selection_policy
                .preferred_runtime(&intent.profile_id, &runtime_catalog)
        } else {
            None
        };
        let selector = intent
            .runtime_override
            .as_ref()
            .map(|value| runtime::RuntimeSelector::Alias(value.clone()))
            .or_else(|| history_runtime.map(runtime::RuntimeSelector::Alias))
            .unwrap_or(runtime::RuntimeSelector::Auto);
        let selection = runtime::RuntimeSelectionRequest {
            selector,
            profile_id: intent.profile_id.0.clone(),
            required_capabilities: required_capabilities
                .iter()
                .map(runtime_capability)
                .collect(),
            interaction_mode: runtime::InteractionMode::Resident,
            workspace_mode: match workspace_mode {
                AgentWorkspaceMode::WorkspaceLess => runtime::WorkspaceMode::WorkspaceLess,
                AgentWorkspaceMode::SharedReadOnly => runtime::WorkspaceMode::SharedReadOnly,
                AgentWorkspaceMode::SharedWritable => runtime::WorkspaceMode::SharedWritable,
                AgentWorkspaceMode::IsolatedWorktree => runtime::WorkspaceMode::IsolatedWorktree,
            },
            task_encoding: if required_capabilities
                .contains(&AgentRuntimeCapability::MemoryProposal)
            {
                runtime::TaskEncoding::StructuredJson
            } else {
                runtime::TaskEncoding::NaturalLanguage
            },
            max_input_tokens: intent.budget.max_input_tokens,
        };
        let runtime_id = match self.select_runtime(&selection) {
            Ok((runtime_id, decision)) => {
                info!(
                    profile_id = %intent.profile_id.0,
                    runtime_id = %runtime_id.0,
                    override_used = decision.override_used,
                    required_capabilities = ?decision.effective_capabilities,
                    reason = %decision.reason,
                    "generic subagent runtime selected"
                );
                runtime_id
            }
            Err(_selection_error)
                if required_capabilities.is_empty()
                    && intent
                        .runtime_override
                        .as_ref()
                        .is_some_and(|runtime_override| {
                            !runtime_catalog.iter().any(|manifest| {
                                manifest.id == *runtime_override
                                    || manifest
                                        .aliases
                                        .iter()
                                        .any(|alias| alias == runtime_override)
                            })
                        }) =>
            {
                let runtime_id =
                    ::contracts::RuntimeId(intent.runtime_override.clone().unwrap_or_default());
                if let Some(supervisor) = &self.runtime_agent_supervisor {
                    supervisor
                        .registry()
                        .resolve(&runtime::DelegateBackendId(runtime_id.0.clone()))
                        .ok_or_else(|| {
                            control_error(
                                AgentControlErrorKind::NotFound,
                                format!("runtime is not registered: {}", runtime_id.0),
                            )
                        })?;
                } else {
                    self.runtimes
                        .as_ref()
                        .ok_or_else(|| {
                            control_error(
                                AgentControlErrorKind::NotFound,
                                "Runtime supervisor is not composed",
                            )
                        })?
                        .resolve(&runtime_id)?;
                }
                info!(
                    profile_id = %intent.profile_id.0,
                    runtime_id = %runtime_id.0,
                    "explicit-only compatibility subagent runtime selected"
                );
                runtime_id
            }
            Err(error) => return Err(error),
        };
        self.control_spawn(AgentSpawnRequest {
            root_agent_id: intent.root_agent_id,
            parent_agent_id: intent.parent_agent_id,
            parent_process_id: intent.parent_process_id,
            profile_id: intent.profile_id,
            runtime_id,
            trusted_workspace: intent.trusted_workspace,
            delegator_authority: intent.delegator_authority,
            cognitive_binding: None,
            task: intent.task,
            context: intent.context,
            broadcast_refs: vec![],
            allowed_tools: intent.allowed_tools,
            budget: intent.budget,
            background_decls: vec![],
        })
        .await
    }

    async fn control_spawn(
        &self,
        request: AgentSpawnRequest,
    ) -> Result<AgentHandle, AgentControlError> {
        self.spawn_via_runtime(request).await
    }

    async fn control_wait(
        &self,
        request: AgentWaitRequest,
    ) -> Result<AgentSnapshot, AgentControlError> {
        request.validate()?;
        if let Some(supervisor) = &self.runtime_agent_supervisor {
            if let Ok(generation) =
                supervisor.generation(&runtime::AgentRunId(request.agent_id.0.to_string()))
            {
                supervisor
                    .wait_with_generation(
                        &runtime::AgentRunId(request.agent_id.0.to_string()),
                        &generation,
                    )
                    .await
                    .map_err(|error| {
                        control_error(
                            AgentControlErrorKind::Runtime,
                            format!("Runtime Agent wait failed: {error}"),
                        )
                    })?;
            }
        }
        self.wait_local(
            request.caller_root_agent_id,
            request.agent_id,
            Duration::from_millis(request.timeout_ms),
        )
        .await
    }

    async fn control_send(
        &self,
        request: AgentSendRequest,
    ) -> Result<AgentControlMessage, AgentControlError> {
        request.validate()?;
        let Some(supervisor) = &self.runtime_agent_supervisor else {
            return self.send_local(request).await;
        };
        let runtime_id = runtime::AgentRunId(request.agent_id.0.to_string());
        let generation = supervisor.generation(&runtime_id).map_err(|error| {
            control_error(
                AgentControlErrorKind::Runtime,
                format!("Runtime Agent send lookup failed: {error}"),
            )
        })?;
        let run = self
            .authorize(request.caller_root_agent_id, request.agent_id)
            .await?;
        if run.status().is_terminal() {
            return Err(control_error(
                AgentControlErrorKind::Terminal,
                "terminal Agent rejects new messages",
            ));
        }
        let from = self.authorize_message_sender(&run, &request).await?;
        let delivery_id = request.delivery_id.unwrap_or_else(uuid::Uuid::new_v4);
        let receipt = supervisor
            .send_with_generation(
                &runtime_id,
                &generation,
                runtime::DelegateMessage {
                    kind: format!("{:?}", request.kind).to_ascii_lowercase(),
                    content: request.message.clone(),
                    correlation: request.correlation_id.map(|value| value.to_string()),
                    delivery_id: Some(delivery_id.to_string()),
                },
            )
            .await
            .map_err(|error| {
                control_error(
                    AgentControlErrorKind::Runtime,
                    format!("Runtime Agent send failed: {error}"),
                )
            })?;
        let receipt_delivery_id = receipt
            .delivery_id
            .as_deref()
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .unwrap_or(delivery_id);
        Ok(AgentControlMessage {
            delivery_id: receipt_delivery_id,
            sequence: receipt.sequence.unwrap_or_default(),
            from,
            to: request.agent_id,
            kind: request.kind,
            delivery: if receipt.delivered {
                AgentMessageDeliveryState::Delivered
            } else {
                AgentMessageDeliveryState::Rejected
            },
            content: request.message,
        })
    }

    async fn control_cancel(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        let run = self.authorize(caller_root_agent_id, agent_id).await?;
        if run.status().is_terminal() {
            return Ok(run.snapshot);
        }
        if let Some(supervisor) = &self.runtime_agent_supervisor {
            let runtime_id = runtime::AgentRunId(agent_id.0.to_string());
            if let Ok(generation) = supervisor.generation(&runtime_id) {
                supervisor
                    .cancel_with_generation(&runtime_id, &generation)
                    .await
                    .map_err(|error| {
                        control_error(
                            AgentControlErrorKind::Runtime,
                            format!("Runtime Agent cancel failed: {error}"),
                        )
                    })?;
                return self
                    .repository
                    .get(agent_id)
                    .await?
                    .map(|record| record.snapshot)
                    .ok_or_else(|| {
                        control_error(
                            AgentControlErrorKind::NotFound,
                            "Agent run disappeared after Runtime cancellation",
                        )
                    });
            }
        }
        self.cancel_local(caller_root_agent_id, agent_id).await
    }

    async fn control_inspect(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        self.authorize(caller_root_agent_id, agent_id)
            .await
            .map(|record| record.snapshot)
    }

    async fn control_list(
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

#[async_trait]
impl AgentControlPort for RuntimeAgentControlFacade {
    async fn spawn_intent(
        &self,
        intent: AgentSpawnIntent,
    ) -> Result<AgentHandle, AgentControlError> {
        self.host.control_spawn_intent(intent).await
    }

    async fn spawn(&self, request: AgentSpawnRequest) -> Result<AgentHandle, AgentControlError> {
        self.host.control_spawn(request).await
    }

    async fn wait(&self, request: AgentWaitRequest) -> Result<AgentSnapshot, AgentControlError> {
        self.host.control_wait(request).await
    }

    async fn send(
        &self,
        request: AgentSendRequest,
    ) -> Result<AgentControlMessage, AgentControlError> {
        self.host.control_send(request).await
    }

    async fn cancel(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        self.host
            .control_cancel(caller_root_agent_id, agent_id)
            .await
    }

    async fn inspect(
        &self,
        caller_root_agent_id: AgentId,
        agent_id: AgentId,
    ) -> Result<AgentSnapshot, AgentControlError> {
        self.host
            .control_inspect(caller_root_agent_id, agent_id)
            .await
    }

    async fn list(
        &self,
        request: AgentListRequest,
    ) -> Result<Vec<AgentSnapshot>, AgentControlError> {
        self.host.control_list(request).await
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
