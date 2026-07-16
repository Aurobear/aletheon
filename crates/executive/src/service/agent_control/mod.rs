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
    AgentId, AgentListRequest, AgentRunStatus, AgentSendRequest, AgentSnapshot, AgentSpawnRequest,
    AgentWaitRequest, CancelReason, Clock, ExitReason, NamespaceId, OperationExitReason,
    OperationKind, OperationRequest, ProcessSignal, SpawnSpec, Timer,
};
use tokio::sync::{watch, Mutex};
use tokio::task::JoinSet;

pub mod admission;
pub mod context_fork;
pub mod execution;
pub mod live_runs;
pub mod repository;
pub mod sqlite_repository;

pub use admission::{AgentAdmissionLease, AgentAdmissionPort, BoundedAgentAdmission};
pub use context_fork::{AgentContextItem, AgentContextItemKind, AgentContextProjection};
pub use execution::{
    AgentEventSink, AgentRuntimeEvent, AgentRuntimeInput, AgentRuntimeLauncher,
    AgentRuntimeRegistry, CompatibilityRuntimeLauncher, NoopAgentEventSink,
};
pub use live_runs::{LiveAgentRun, LiveAgentRuns};
pub use repository::{AgentMessageRecord, AgentRunRecord, AgentRunRepository};
pub use sqlite_repository::SqliteAgentRunRepository;

const DEFAULT_RETENTION_MS: i64 = 7 * 24 * 60 * 60 * 1_000;
const MAILBOX_CAPACITY: usize = 64;
const CANCEL_WAIT: Duration = Duration::from_secs(30);

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
    timer: Arc<dyn AgentWaitTimer>,
    live: Arc<LiveAgentRuns>,
    tasks: Mutex<JoinSet<()>>,
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
        Self {
            kernel,
            clock,
            repository,
            admission,
            runtimes,
            events: Arc::new(NoopAgentEventSink),
            timer: Arc::new(SystemAgentWaitTimer),
            live: Arc::new(LiveAgentRuns::default()),
            tasks: Mutex::new(JoinSet::new()),
        }
    }

    pub fn with_event_sink(mut self, events: Arc<dyn AgentEventSink>) -> Self {
        self.events = events;
        self
    }

    pub fn with_wait_timer(mut self, timer: Arc<dyn AgentWaitTimer>) -> Self {
        self.timer = timer;
        self
    }

    pub fn live_runs(&self) -> Arc<LiveAgentRuns> {
        self.live.clone()
    }

    pub async fn shutdown(&self) {
        for run in self.live.all().await {
            run.cancellation.cancel();
        }
        let mut tasks = self.tasks.lock().await;
        while tasks.join_next().await.is_some() {}
    }

    async fn validated_parent(
        &self,
        request: &AgentSpawnRequest,
    ) -> Result<AgentId, AgentControlError> {
        match (request.parent_agent_id, request.parent_process_id) {
            (None, None) => Ok(request.root_agent_id),
            (Some(parent), Some(parent_process)) => {
                let parent_run = self.repository.get(parent).await?.ok_or_else(|| {
                    control_error(
                        AgentControlErrorKind::NotFound,
                        "parent Agent was not found",
                    )
                })?;
                if parent_run.root_agent_id() != request.root_agent_id
                    || parent_run.snapshot.handle.process_id != parent_process
                    || parent_run.status().is_terminal()
                {
                    return Err(control_error(
                        AgentControlErrorKind::Forbidden,
                        "parent Agent does not belong to the requested live root/process",
                    ));
                }
                Ok(AgentId::new())
            }
            _ => Err(AgentControlError::invalid(
                "parent Agent and parent Process must be supplied together",
            )),
        }
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
        let context = AgentContextProjection::from_fork(&request.context)?;
        let agent_id = self.validated_parent(&request).await?;
        let request_hash = SqliteAgentRunRepository::request_hash(&request)?;
        let mut admission = self.admission.reserve(&request).await?;

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
                admission.revoke();
                return Err(runtime_error(error));
            }
        };
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
                admission.revoke();
                return Err(runtime_error(error));
            }
        };
        let mailbox_target = Target::from(format!("agent:{}", agent_id.0));
        let mailbox: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::with_capacity(MAILBOX_CAPACITY));
        if let Err(error) = self
            .kernel
            .register_process_mailbox(process.id, mailbox_target.clone(), mailbox)
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
            admission.revoke();
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
            admission.revoke();
            return Err(error);
        }

        let scope = OperationScope::new(operation.id);
        let cancellation = scope.token();
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
            admission.revoke();
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
        let runtime_input = AgentRuntimeInput {
            request,
            handle: handle.clone(),
            context,
            cancellation,
        };
        self.tasks.lock().await.spawn(async move {
            run_agent(
                kernel,
                clock,
                repository,
                live,
                launcher,
                events,
                runtime_input,
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
        let message = self
            .repository
            .append_message(
                request.agent_id,
                request.caller_root_agent_id,
                &request.message,
                self.clock.wall_now().0,
            )
            .await?;
        let envelope = EnvelopeV2::new(
            SchemaId::from(SchemaId::AGENT_CONTROL_MESSAGE_V1),
            Target::from(format!("agent:{}", request.caller_root_agent_id.0)),
            live.mailbox_target,
            DeliveryPattern::Direct,
            NamespaceId(request.caller_root_agent_id.0.to_string()),
            serde_json::json!({
                "sequence": message.sequence,
                "content": request.message,
                "start_turn": request.start_turn,
            }),
        )
        .with_operation_id(run.snapshot.handle.operation_id)
        .with_logical_time(message.sequence);
        let receipt = self.kernel.mailbox_service().route(envelope).await;
        if !receipt.is_ok() {
            return Err(control_error(
                AgentControlErrorKind::Runtime,
                format!("Agent message delivery failed: {receipt:?}"),
            ));
        }
        Ok(AgentControlMessage {
            sequence: message.sequence,
            from: request.caller_root_agent_id,
            to: request.agent_id,
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
        admission.revoke();
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
            admission.revoke();
            live.remove(agent).await;
            return;
        }
    };
    snapshots.send_replace(running.snapshot);

    let (outcome_sender, outcome_receiver) = tokio::sync::oneshot::channel();
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
    admission.settle();
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
