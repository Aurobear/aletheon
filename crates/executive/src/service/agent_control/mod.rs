use std::collections::HashSet;
use std::sync::Arc;
use std::time::{Duration, Instant};

use aletheon_kernel::chronos::SystemTimer;
use aletheon_kernel::operation::OperationScope;
use aletheon_kernel::KernelRuntime;
use async_trait::async_trait;
use fabric::ipc::envelope_v2::{DeliveryPattern, EnvelopeV2, SchemaId, Target};
use fabric::ipc::mailbox::{InProcessMailbox, Mailbox, MailboxService};
use fabric::{
    AgentControlError, AgentControlErrorKind, AgentControlMessage, AgentControlPort, AgentHandle,
    AgentId, AgentListRequest, AgentMessageDeliveryState, AgentMessagePayload, AgentRunStatus,
    AgentSendRequest, AgentSnapshot, AgentSpawnRequest, AgentWaitRequest, AgoraVersion,
    CancelReason, Clock, ContextBinding, EventSpine, ExitReason, NamespaceId, OperationExitReason,
    OperationKind, OperationRequest, ProcessId, ProcessSignal, SpawnSpec, Timer,
};
use tokio::sync::{watch, Mutex};
use tokio::task::JoinSet;

pub mod admission;
pub mod candidate_projection;
pub mod context_fork;
pub mod execution;
pub mod live_runs;
pub mod mailbox;
pub mod memory;
pub mod repository;
pub mod sqlite_repository;

pub use admission::{
    AgentAdmissionLease, AgentAdmissionMetrics, AgentAdmissionPort, AgentAdmissionRequest,
    AgentStorageRequest, BoundedAgentAdmission,
};
pub use candidate_projection::{
    AgentCandidateProjector, AgentCandidateSubmissionPort, ProjectingAgentEventSink,
};
pub use context_fork::{
    AgentContextItem, AgentContextItemKind, AgentContextProjection, AgentContextProjectionBuilder,
};
pub use execution::{
    AgentEventSink, AgentRuntimeEvent, AgentRuntimeInput, AgentRuntimeLauncher,
    AgentRuntimeRegistry, CompatibilityRuntimeLauncher, NoopAgentEventSink, SpineAgentEventSink,
};
pub use live_runs::{LiveAgentRun, LiveAgentRuns};
pub use mailbox::{AgentMailboxBridge, AgentRuntimeInbox};
pub use memory::MemoryRecordingAgentEventSink;
pub use repository::{agent_workspace_id, AgentMessageRecord, AgentRunRecord, AgentRunRepository};
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
        }
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

    pub fn live_runs(&self) -> Arc<LiveAgentRuns> {
        self.live.clone()
    }

    pub fn admission_metrics(&self) -> AgentAdmissionMetrics {
        self.admission.metrics()
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
            .reserve(AgentAdmissionRequest::new(
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

        let scope = OperationScope::new(operation.id);
        let cancellation = scope.token();
        let (mailbox_bridge, inbox) =
            AgentMailboxBridge::bounded(mailbox, MAILBOX_CAPACITY, cancellation.clone())?;
        let (snapshots, _) = watch::channel(queued);
        let inserted = self
            .live
            .insert(
                agent_id,
                LiveAgentRun {
                    snapshots: snapshots.clone(),
                    mailbox_target,
                    cancellation: cancellation.clone(),
                },
            )
            .await;
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
        let runtime_input = AgentRuntimeInput {
            request,
            handle: handle.clone(),
            workspace_id,
            root_workspace_id,
            root_process_id,
            context,
            memory_context: memory_context.clone(),
            inbox,
            cancellation,
        };
        let events: Arc<dyn AgentEventSink> = Arc::new(SpineAgentEventSink::new(
            events,
            event_spine,
            runtime_input.clone(),
            event_projections,
        ));
        let events: Arc<dyn AgentEventSink> = Arc::new(MemoryRecordingAgentEventSink::new(
            events,
            self.agent_memory_vault.clone(),
            memory_context,
        ));
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
) {
    let agent = input.handle.agent_id;
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
    let _ = kernel.terminate_process(process, process_exit).await;
    let settlement = match settlement_usage {
        Some(usage) if next == AgentRunStatus::Succeeded => {
            AgentAdmissionLease::settle(&mut *admission, &usage).await
        }
        _ => admission.revoke().await,
    };
    if let Err(error) = settlement {
        tracing::error!(agent = ?agent, %error, "failed to settle Agent admission lease");
    }
    live.remove(agent).await;
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
