//! Sub-agent spawning and tracking.
//!
//! Sub-agents are spawned by the LLM via the `agent` tool call. The UI still
//! sees `SubAgentHandle`, but the authoritative lifecycle now lives in the
//! kernel `ProcessTable` and the cancellation token comes from an Operation
//! task group.
//!
//! Supervision: when a sub-agent transitions to `Failed`, the spawner consults
//! a `SupervisorTree`. If the policy permits restart, a replacement is spawned;
//! if the restart limit is reached, the failure is logged and propagated.

#![allow(dead_code)]
//!
//! # Execution
//!
//! When a [`SubAgentRuntime`] is provided (via [`SubAgentSpawner::with_runtime`]),
//! spawned sub-agents execute real LLM + tool work instead of the no-op stub.
//! Without a runtime, the spawned task waits for cancellation (test/dev mode).

use async_trait::async_trait;
use std::future::Future;
use std::pin::Pin;

use aletheon_kernel::chronos::SystemClock;
use aletheon_kernel::operation::{OperationScope, OperationTable};
use aletheon_kernel::process::ProcessTable;
use aletheon_kernel::supervision::{RestartDecision, RestartPolicy, SupervisorTree};
use fabric::ipc::envelope_v2::Target;
use fabric::ipc::mailbox::{InProcessMailbox, InProcessMailboxService, Mailbox, MailboxService};
use fabric::ui_event::{SubAgentHandle, SubAgentStatus};
use fabric::{
    AgentProfileId, CancelReason, ExitReason, NamespaceId, OperationExitReason, OperationKind,
    OperationManager, OperationRequest, ProcessId, ProcessManager, ProcessSignal, ProcessSnapshot,
    ProcessState, SpawnSpec, SubAgentState,
};
use std::collections::HashMap;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Execution runtime for sub-agents — performs the actual LLM + tool work.
///
/// When set on a [`SubAgentSpawner`], spawned sub-agents run real reasoning
/// loops instead of the no-op cancellation stub.
#[async_trait]
pub trait SubAgentRuntime: Send + Sync {
    /// Execute a sub-agent task.
    ///
    /// Receives the task description and a cancellation token. Returns the
    /// final response text or an error.
    async fn run(&self, task: &str, cancel: CancellationToken) -> Result<String, String>;
}

/// Owned, boxed future for spawn() compatibility.
type RuntimeFuture = Pin<Box<dyn Future<Output = Result<String, String>> + Send>>;

/// Erased runtime closure — bridges the trait to the operation scope spawn.
struct RuntimeTask {
    fut: RuntimeFuture,
}

impl RuntimeTask {
    fn new(runtime: Arc<dyn SubAgentRuntime>, task: String, cancel: CancellationToken) -> Self {
        Self {
            fut: Box::pin(async move { runtime.run(&task, cancel).await }),
        }
    }

    async fn run(self) -> Result<String, String> {
        self.fut.await
    }
}

/// Error returned when an illegal lifecycle transition is requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// No agent with the given id is tracked.
    Unknown(String),
    /// The transition `from -> to` is not legal.
    Illegal {
        from: SubAgentState,
        to: SubAgentState,
    },
    /// Kernel process table rejected the transition.
    Kernel(String),
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransitionError::Unknown(id) => write!(f, "unknown sub-agent: {id}"),
            TransitionError::Illegal { from, to } => {
                write!(f, "illegal transition {from:?} -> {to:?}")
            }
            TransitionError::Kernel(message) => write!(f, "kernel transition failed: {message}"),
        }
    }
}
impl std::error::Error for TransitionError {}

/// Internal per-agent record: the UI handle plus process/operation control-plane ids.
#[derive(Debug)]
struct SubAgentEntry {
    handle: SubAgentHandle,
    state: SubAgentState,
    process_id: ProcessId,
    mailbox_target: Target,
    operation_id: fabric::OperationId,
    scope: OperationScope,
    /// Saved spawn parameters so we can restart on failure.
    task: String,
    parent_turn_id: String,
}

/// Spawns and tracks sub-agents.
pub struct SubAgentSpawner {
    agents: HashMap<String, SubAgentEntry>,
    process_table: Arc<ProcessTable>,
    operation_table: Arc<OperationTable>,
    mailbox_service: Arc<InProcessMailboxService>,
    supervisor: SupervisorTree,
    /// Optional execution runtime — when set, spawned sub-agents run real
    /// LLM + tool work. When `None`, the stub waits for cancellation.
    runtime: Option<Arc<dyn SubAgentRuntime>>,
}

impl fmt::Debug for SubAgentSpawner {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SubAgentSpawner")
            .field("agents", &self.agents)
            .field("process_table", &self.process_table)
            .field("operation_table", &self.operation_table)
            .field("mailbox_service", &self.mailbox_service)
            .field("supervisor", &self.supervisor)
            .field("runtime", &self.runtime.as_ref().map(|_| "SubAgentRuntime"))
            .finish()
    }
}

impl Default for SubAgentSpawner {
    fn default() -> Self {
        Self::new()
    }
}

impl SubAgentSpawner {
    pub fn new() -> Self {
        let clock = Arc::new(SystemClock::new());
        Self::with_tables(
            Arc::new(ProcessTable::new(clock.clone())),
            Arc::new(OperationTable::new(clock)),
        )
    }

    pub fn with_tables(
        process_table: Arc<ProcessTable>,
        operation_table: Arc<OperationTable>,
    ) -> Self {
        Self {
            agents: HashMap::new(),
            process_table,
            operation_table,
            mailbox_service: Arc::new(InProcessMailboxService::new()),
            supervisor: SupervisorTree::new(),
            runtime: None,
        }
    }

    /// Attach a sub-agent execution runtime.
    ///
    /// When set, spawned sub-agents run real LLM + tool work through the
    /// runtime. Without it, the spawned task waits for cancellation (the
    /// dev/test stub).
    pub fn with_runtime(&mut self, runtime: Arc<dyn SubAgentRuntime>) {
        self.runtime = Some(runtime);
    }

    /// Mailbox registry used by sub-agent process collaboration.
    pub fn mailbox_service(&self) -> Arc<InProcessMailboxService> {
        self.mailbox_service.clone()
    }

    /// Routing target for a live sub-agent's mailbox.
    pub fn mailbox_target(&self, id: &str) -> Option<Target> {
        self.agents
            .get(id)
            .map(|entry| entry.mailbox_target.clone())
    }

    /// Register a new sub-agent process and return the UI handle view.
    ///
    /// `restart_policy` governs whether the supervisor will restart this agent
    /// on failure. Defaults to `RestartPolicy::Never`.
    pub async fn spawn(
        &mut self,
        task: String,
        parent_turn_id: String,
    ) -> anyhow::Result<SubAgentHandle> {
        self.spawn_with_policy(task, parent_turn_id, RestartPolicy::Never)
            .await
    }

    /// Register a new sub-agent process with an explicit restart policy.
    pub async fn spawn_with_policy(
        &mut self,
        task: String,
        parent_turn_id: String,
        restart_policy: RestartPolicy,
    ) -> anyhow::Result<SubAgentHandle> {
        let process = self
            .process_table
            .spawn(SpawnSpec {
                profile: AgentProfileId("sub-agent".into()),
                namespace: NamespaceId(parent_turn_id.clone()),
                initial_operation: Some(OperationKind::SubAgent),
                ..SpawnSpec::default()
            })
            .await?;
        let operation = self
            .operation_table
            .submit(OperationRequest {
                owner: process.id,
                parent: None,
                kind: OperationKind::SubAgent,
                deadline: None,
            })
            .await?;
        self.operation_table.start(operation.id).await?;
        self.process_table
            .set_active_operation(process.id, Some(operation.id))
            .await?;
        let mailbox_target = Self::mailbox_target_for(process.id);
        let mailbox: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::with_capacity(64));
        self.mailbox_service
            .register(mailbox_target.clone(), mailbox)
            .await?;

        let mut scope = OperationScope::new(operation.id);
        let token = scope.token();

        // Spawn the actual execution payload: real LLM+tool work when a
        // runtime is attached, cancellation-wait stub otherwise.
        if let Some(ref runtime) = self.runtime {
            let rt = runtime.clone();
            let task_clone = task.clone();
            let cancel = token.clone();
            scope.spawn("sub-agent-execution", async move {
                info!(task = %task_clone, "Sub-agent execution started");
                match rt.run(&task_clone, cancel).await {
                    Ok(_output) => {
                        info!(output_len = _output.len(), "Sub-agent completed");
                        OperationExitReason::Completed
                    }
                    Err(e) => {
                        warn!(error = %e, "Sub-agent failed");
                        OperationExitReason::Failed(e)
                    }
                }
            });
        } else {
            scope.spawn("sub-agent-execution", async move {
                token.cancelled().await;
                OperationExitReason::Cancelled(CancelReason::User)
            });
        }

        let snapshot = self.process_table.inspect(process.id).await?;
        let id = process.id.0.to_string();
        let handle =
            Self::handle_from_snapshot(id.clone(), task.clone(), parent_turn_id.clone(), &snapshot);

        // Register with supervisor for failure-restart tracking.
        self.supervisor.supervise(process.id, restart_policy);

        self.agents.insert(
            id,
            SubAgentEntry {
                handle: handle.clone(),
                state: SubAgentState::Created,
                process_id: process.id,
                mailbox_target,
                operation_id: operation.id,
                scope,
                task,
                parent_turn_id,
            },
        );
        Ok(handle)
    }

    /// Register a tracked sub-agent entry without spawning a runtime task.
    ///
    /// Creates process/operation/mailbox entries so the sub-agent appears in
    /// the process table and can be cancelled or waited on, but does not
    /// execute any LLM work. The caller is responsible for running the actual
    /// task and calling [`transition`](Self::transition) to update state.
    pub async fn spawn_tracked(
        &mut self,
        task: String,
        parent_turn_id: String,
        restart_policy: RestartPolicy,
    ) -> anyhow::Result<SubAgentHandle> {
        let process = self
            .process_table
            .spawn(SpawnSpec {
                profile: AgentProfileId("sub-agent".into()),
                namespace: NamespaceId(parent_turn_id.clone()),
                initial_operation: Some(OperationKind::SubAgent),
                ..SpawnSpec::default()
            })
            .await?;
        let operation = self
            .operation_table
            .submit(OperationRequest {
                owner: process.id,
                parent: None,
                kind: OperationKind::SubAgent,
                deadline: None,
            })
            .await?;
        self.operation_table.start(operation.id).await?;
        self.process_table
            .set_active_operation(process.id, Some(operation.id))
            .await?;
        let mailbox_target = Self::mailbox_target_for(process.id);
        let mailbox: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::with_capacity(64));
        self.mailbox_service
            .register(mailbox_target.clone(), mailbox)
            .await?;

        let mut scope = OperationScope::new(operation.id);
        let token = scope.token();

        // Always use the cancellation-wait stub — no runtime execution.
        scope.spawn("sub-agent-execution", async move {
            token.cancelled().await;
            OperationExitReason::Cancelled(CancelReason::User)
        });

        let snapshot = self.process_table.inspect(process.id).await?;
        let id = process.id.0.to_string();
        let handle =
            Self::handle_from_snapshot(id.clone(), task.clone(), parent_turn_id.clone(), &snapshot);

        // Register with supervisor for failure-restart tracking.
        self.supervisor.supervise(process.id, restart_policy);

        self.agents.insert(
            id,
            SubAgentEntry {
                handle: handle.clone(),
                state: SubAgentState::Created,
                process_id: process.id,
                mailbox_target,
                operation_id: operation.id,
                scope,
                task,
                parent_turn_id,
            },
        );
        Ok(handle)
    }

    /// Update an agent's UI status (unchanged UI-display behavior).
    pub fn update_status(&mut self, id: &str, status: SubAgentStatus) {
        if let Some(entry) = self.agents.get_mut(id) {
            entry.handle.status = status;
        }
    }

    /// Current control-plane state of an agent, if tracked.
    pub fn state(&self, id: &str) -> Option<SubAgentState> {
        self.agents.get(id).map(|e| e.state)
    }

    /// A clone of the agent's cancellation token from its operation task group.
    pub fn cancel_token(&self, id: &str) -> Option<CancellationToken> {
        self.agents.get(id).map(|e| e.scope.token())
    }

    pub async fn snapshot(&self, id: &str) -> anyhow::Result<Option<ProcessSnapshot>> {
        let Some(entry) = self.agents.get(id) else {
            return Ok(None);
        };
        Ok(Some(self.process_table.inspect(entry.process_id).await?))
    }

    /// Attempt a legal-only lifecycle transition and mirror it into the process table.
    ///
    /// When transitioning to `Failed`, the supervisor is consulted. If the
    /// restart policy permits it, a replacement sub-agent is spawned
    /// automatically.
    pub async fn transition(
        &mut self,
        id: &str,
        next: SubAgentState,
    ) -> Result<(), TransitionError> {
        let (from, process_id) = {
            let entry = self
                .agents
                .get(id)
                .ok_or_else(|| TransitionError::Unknown(id.to_string()))?;
            (entry.state, entry.process_id)
        };
        if !from.can_transition_to(&next) {
            return Err(TransitionError::Illegal { from, to: next });
        }
        match next {
            SubAgentState::Running => {
                let current = self
                    .process_table
                    .inspect(process_id)
                    .await
                    .map_err(|e| TransitionError::Kernel(e.to_string()))?
                    .state;
                if current == ProcessState::Created {
                    self.process_table
                        .signal(process_id, ProcessSignal::Start)
                        .await
                        .map_err(|e| TransitionError::Kernel(e.to_string()))?;
                } else if current == ProcessState::Waiting {
                    self.process_table
                        .signal(process_id, ProcessSignal::Resume)
                        .await
                        .map_err(|e| TransitionError::Kernel(e.to_string()))?;
                }
            }
            SubAgentState::Waiting => self
                .process_table
                .signal(process_id, ProcessSignal::Wait)
                .await
                .map_err(|e| TransitionError::Kernel(e.to_string()))?,
            SubAgentState::Completed => self
                .process_table
                .mark_exit(process_id, ExitReason::Completed)
                .await
                .map_err(|e| TransitionError::Kernel(e.to_string()))?,
            SubAgentState::Failed => {
                self.process_table
                    .mark_exit(process_id, ExitReason::Failed("sub-agent failed".into()))
                    .await
                    .map_err(|e| TransitionError::Kernel(e.to_string()))?;

                // Consult supervisor for restart decision.
                let snapshot = self
                    .process_table
                    .inspect(process_id)
                    .await
                    .map_err(|e| TransitionError::Kernel(e.to_string()))?;
                let exit_reason = snapshot
                    .exit
                    .as_ref()
                    .map(|e| e.reason.clone())
                    .unwrap_or(ExitReason::Failed("unknown".into()));
                let decision = self.supervisor.record_exit(process_id, &exit_reason);

                match decision {
                    RestartDecision::Restart { attempt } => {
                        warn!(
                            agent_id = %id,
                            attempt,
                            reason = ?exit_reason,
                            "Sub-agent failed; supervisor restarting",
                        );
                        self.restart_agent(id).await?;
                    }
                    RestartDecision::RestartGroup {
                        attempt,
                        ref siblings,
                    } => {
                        warn!(
                            agent_id = %id,
                            attempt,
                            sibling_count = siblings.len(),
                            reason = ?exit_reason,
                            "Sub-agent failed; supervisor restarting group",
                        );
                        self.restart_agent(id).await?;
                        // Restart siblings: look up their agent-id strings
                        // from the process table.
                        let sibling_ids: Vec<String> = siblings
                            .iter()
                            .filter_map(|&pid| {
                                self.agents.iter().find_map(|(aid, entry)| {
                                    if entry.process_id == pid {
                                        Some(aid.clone())
                                    } else {
                                        None
                                    }
                                })
                            })
                            .collect();
                        for sid in &sibling_ids {
                            if let Err(e) = self.restart_agent(sid).await {
                                warn!(
                                    sibling_agent_id = %sid,
                                    error = %e,
                                    "Failed to restart sibling agent",
                                );
                            }
                        }
                    }
                    RestartDecision::FailedLimitReached => {
                        warn!(
                            agent_id = %id,
                            reason = ?exit_reason,
                            "Sub-agent restart limit reached; not restarting",
                        );
                    }
                    RestartDecision::DoNotRestart => {
                        // RestartPolicy::Never or non-failure exit — no action needed.
                    }
                }
            }
            SubAgentState::Destroyed => self
                .process_table
                .signal(process_id, ProcessSignal::Terminate)
                .await
                .map_err(|e| TransitionError::Kernel(e.to_string()))?,
            SubAgentState::Created => {}
        }
        if let Some(entry) = self.agents.get_mut(id) {
            entry.state = next;
        }
        Ok(())
    }

    /// Spawn a replacement sub-agent when the supervisor permits restart.
    ///
    /// Reads the original `task` / `parent_turn_id` from the failed entry,
    /// spawns a fresh process with the same restart policy, and inserts the
    /// new entry under the new agent id.
    async fn restart_agent(&mut self, failed_id: &str) -> Result<(), TransitionError> {
        let (task, parent_turn_id) = {
            let entry = self
                .agents
                .get(failed_id)
                .ok_or_else(|| TransitionError::Unknown(failed_id.to_string()))?;
            (entry.task.clone(), entry.parent_turn_id.clone())
        };
        // Spawn replacement; use Never for the replacement to avoid infinite
        // restart chains — the supervisor already tracks the restart count on
        // the original ProcessId.
        self.spawn_with_policy(task, parent_turn_id, RestartPolicy::Never)
            .await
            .map_err(|e| TransitionError::Kernel(e.to_string()))?;
        Ok(())
    }

    /// Cancel a sub-agent's operation scope.
    ///
    /// Triggers cancellation of the underlying `CancellationToken`. The state
    /// in the spawner transitions to `Failed` when the cancelled task exits.
    /// Returns `false` if no agent with that id is tracked.
    pub fn cancel(&self, id: &str) -> bool {
        match self.agents.get(id) {
            Some(entry) => {
                entry.scope.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Wait for a sub-agent process to exit, with a timeout.
    ///
    /// Returns a clone of the `SubAgentHandle` on completion, or an error
    /// if the timeout elapses or the process is not tracked.
    pub async fn wait(&self, id: &str, timeout: Duration) -> anyhow::Result<SubAgentHandle> {
        let entry = self
            .agents
            .get(id)
            .ok_or_else(|| anyhow::anyhow!("unknown sub-agent: {id}"))?;
        let pid = entry.process_id;
        tokio::time::timeout(timeout, self.process_table.wait(pid))
            .await
            .map_err(|_| anyhow::anyhow!("wait timed out for sub-agent: {id}"))?
            .map_err(|e| anyhow::anyhow!("process wait error: {e}"))?;
        Ok(entry.handle.clone())
    }

    /// Tear an agent down: cancel its operation scope, signal/wait/reap the process, then free UI slot.
    pub async fn destroy(&mut self, id: &str) -> anyhow::Result<bool> {
        let Some(mut entry) = self.agents.remove(id) else {
            return Ok(false);
        };
        self.mailbox_service.unregister(&entry.mailbox_target).await;
        let _task_exits = entry
            .scope
            .cancel_and_drain(Duration::from_millis(50))
            .await;
        self.operation_table
            .cancel(entry.operation_id, CancelReason::User)
            .await?;
        self.process_table
            .signal(entry.process_id, ProcessSignal::Terminate)
            .await?;
        let _ = self.process_table.wait(entry.process_id).await?;
        let _ = self.process_table.reap(entry.process_id).await?;
        Ok(true)
    }

    /// Remove a completed/failed agent (delegates to `destroy` for teardown).
    pub async fn remove(&mut self, id: &str) -> anyhow::Result<bool> {
        self.destroy(id).await
    }

    /// List all active agents.
    pub fn list(&self) -> Vec<&SubAgentHandle> {
        self.agents.values().map(|e| &e.handle).collect()
    }

    /// Get a specific agent's handle.
    pub fn get(&self, id: &str) -> Option<&SubAgentHandle> {
        self.agents.get(id).map(|e| &e.handle)
    }

    fn handle_from_snapshot(
        id: String,
        task: String,
        parent_turn_id: String,
        snapshot: &ProcessSnapshot,
    ) -> SubAgentHandle {
        SubAgentHandle {
            id,
            task,
            status: match snapshot.state {
                ProcessState::Created | ProcessState::Ready => SubAgentStatus::Planning,
                ProcessState::Running => SubAgentStatus::Executing {
                    current_step: "running".into(),
                },
                ProcessState::Waiting | ProcessState::Stopping => SubAgentStatus::WaitingApproval,
                ProcessState::Exited => SubAgentStatus::Completed {
                    summary: "completed".into(),
                },
                ProcessState::Failed => SubAgentStatus::Failed {
                    error: snapshot
                        .exit
                        .as_ref()
                        .map(|e| format!("{:?}", e.reason))
                        .unwrap_or_else(|| "failed".into()),
                },
            },
            parent_turn_id,
            spawned_at_ms: snapshot.process_id.0.as_u128() as u64,
        }
    }

    fn mailbox_target_for(process_id: ProcessId) -> Target {
        Target::from(format!("process:{}", process_id.0))
    }
}

#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use fabric::ipc::envelope_v2::{DeliveryPattern, EnvelopeV2, SchemaId};
    use fabric::ipc::mailbox::DeliveryReceipt;
    use fabric::SubAgentState;

    #[tokio::test]
    async fn spawn_starts_in_created_and_legal_transitions_advance() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into()).await.unwrap();
        assert_eq!(s.state(&h.id), Some(SubAgentState::Created));
        assert!(s.transition(&h.id, SubAgentState::Running).await.is_ok());
        assert!(s.transition(&h.id, SubAgentState::Waiting).await.is_ok());
        assert_eq!(s.state(&h.id), Some(SubAgentState::Waiting));
    }

    #[tokio::test]
    async fn illegal_transition_is_rejected_and_state_unchanged() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into()).await.unwrap();
        // Created -> Completed is illegal (must Run first).
        assert!(s.transition(&h.id, SubAgentState::Completed).await.is_err());
        assert_eq!(s.state(&h.id), Some(SubAgentState::Created));
    }

    #[tokio::test]
    async fn destroy_cancels_in_flight_work_and_frees_the_slot() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into()).await.unwrap();
        let token = s
            .cancel_token(&h.id)
            .expect("token exists while agent is live");

        // Simulate in-flight work awaiting cancellation.
        let worker = tokio::spawn(async move {
            token.cancelled().await;
            "cancelled"
        });

        assert!(
            s.destroy(&h.id).await.unwrap(),
            "destroy returns true for a live agent"
        );
        assert_eq!(
            worker.await.unwrap(),
            "cancelled",
            "destroy must cancel the token"
        );
        assert!(s.get(&h.id).is_none(), "map slot is freed after destroy");
        assert_eq!(s.state(&h.id), None);
        assert!(
            !s.destroy(&h.id).await.unwrap(),
            "second destroy is a no-op"
        );
    }

    #[tokio::test]
    async fn remove_delegates_to_destroy() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into()).await.unwrap();
        let token = s.cancel_token(&h.id).unwrap();
        assert!(!token.is_cancelled());

        assert!(s.remove(&h.id).await.unwrap());
        assert!(token.is_cancelled(), "remove must cancel the token");
        assert!(s.get(&h.id).is_none());
    }

    #[tokio::test]
    async fn spawn_registers_process_mailbox_for_collaboration() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into()).await.unwrap();
        let target = s.mailbox_target(&h.id).expect("mailbox target exists");
        let env = EnvelopeV2::new(
            SchemaId::from("aletheon.test/v1"),
            Target::from("tester"),
            target,
            DeliveryPattern::Direct,
            NamespaceId("turn-1".into()),
            serde_json::json!({"msg": "hello"}),
        );

        let receipt = s.mailbox_service().route(env).await;
        assert!(
            matches!(receipt, DeliveryReceipt::Delivered { .. }),
            "sub-agent collaboration should route through its process mailbox, got {receipt:?}"
        );
    }

    #[tokio::test]
    async fn destroy_unregisters_process_mailbox() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into()).await.unwrap();
        let target = s.mailbox_target(&h.id).expect("mailbox target exists");

        assert!(s.destroy(&h.id).await.unwrap());

        let env = EnvelopeV2::new(
            SchemaId::from("aletheon.test/v1"),
            Target::from("tester"),
            target.clone(),
            DeliveryPattern::Direct,
            NamespaceId("turn-1".into()),
            serde_json::json!({"msg": "after-destroy"}),
        );
        let receipt = s.mailbox_service().route(env).await;
        assert!(
            matches!(receipt, DeliveryReceipt::NoSuchMailbox { target: ref got } if got == &target),
            "destroyed sub-agent mailbox should be removed, got {receipt:?}"
        );
    }

    #[tokio::test]
    async fn list_and_get_preserved_after_internal_type_change() {
        let mut s = SubAgentSpawner::new();
        let h1 = s.spawn("task1".into(), "t1".into()).await.unwrap();
        let h2 = s.spawn("task2".into(), "t2".into()).await.unwrap();

        let list = s.list();
        assert_eq!(list.len(), 2);
        let ids: Vec<&str> = list.iter().map(|h| h.id.as_str()).collect();
        assert!(ids.contains(&h1.id.as_str()));
        assert!(ids.contains(&h2.id.as_str()));

        let got = s.get(&h1.id).unwrap();
        assert_eq!(got.task, "task1");
    }

    #[tokio::test]
    async fn update_status_still_works() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into()).await.unwrap();
        s.update_status(
            &h.id,
            SubAgentStatus::Executing {
                current_step: "step-1".into(),
            },
        );
        let got = s.get(&h.id).unwrap();
        assert!(matches!(got.status, SubAgentStatus::Executing { .. }));
    }

    #[test]
    fn transition_error_display() {
        let err = TransitionError::Unknown("x".into());
        assert!(err.to_string().contains("x"));

        let err = TransitionError::Illegal {
            from: SubAgentState::Created,
            to: SubAgentState::Completed,
        };
        assert!(err.to_string().contains("Created"));
        assert!(err.to_string().contains("Completed"));
    }
}
