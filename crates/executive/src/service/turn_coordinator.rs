//! Single owner of Turn operations and canonical lifecycle settlement.

use std::{collections::HashMap, future::Future, sync::Arc};

use crate::core::config::{BackpressureConfig, GrokHardeningConfig};
use anyhow::{anyhow, Context, Result};
use fabric::types::prompt_queue::{evaluate_cancel, PromptEnvelope, PromptKind, PromptState};
use fabric::{
    CancelReason, EventSpine, ItemId, ItemPayload, ItemRecord, MonoDeadline, OperationKind,
    OperationManager, OperationRequest, PrincipalId, SessionAppendStore, SessionId, SessionRecord,
    SessionStatus, ThreadId, TurnId, TurnMetrics, TurnRequest, TurnResult, TurnStop,
    SESSION_SCHEMA_VERSION,
};
use kernel::KernelRuntime;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

use super::durable_write::{
    record_writer_success, write_failed, TurnWriteTracker, WritePhase, WriteResult,
};
use super::turn_policy::TurnPolicy;

pub struct TurnExecution {
    pub result: TurnResult,
    pub items: Vec<ItemPayload>,
    pub projection: Option<super::post_turn_projection::PostTurnDispatch>,
    pub context_projection: Option<fabric::ContextProjectionReceipt>,
}

struct CompletedExecution {
    result: TurnResult,
    projection: Option<super::post_turn_projection::PostTurnDispatch>,
}

#[derive(Debug, thiserror::Error)]
#[error("terminal durable write failed: {reason}")]
struct TerminalDurableWriteFailure {
    reason: String,
}

fn is_terminal_durable_write_failure(error: &anyhow::Error) -> bool {
    error
        .downcast_ref::<TerminalDurableWriteFailure>()
        .is_some()
}

#[derive(Clone)]
pub struct ActiveTurn {
    pub operation_id: fabric::OperationId,
    pub turn_id: TurnId,
    pub cancel: CancellationToken,
}

#[derive(Clone, Debug, Eq, Hash, PartialEq)]
pub struct ActiveTurnKey {
    pub principal_id: PrincipalId,
    pub thread_id: ThreadId,
}

impl ActiveTurnKey {
    pub fn from_context(context: &fabric::PrincipalContext) -> Self {
        Self {
            principal_id: context.principal_id.clone(),
            thread_id: context.thread_id.clone(),
        }
    }

    pub fn from_request(request: &TurnRequest) -> Self {
        Self::from_context(&request.context)
    }
}

pub struct TurnCoordinator {
    kernel: Arc<KernelRuntime>,
    clock: Arc<dyn fabric::Clock>,
    read_store: Arc<dyn SessionAppendStore>,
    store: Arc<dyn SessionAppendStore>,
    event_spine: Arc<dyn EventSpine>,
    active: Arc<Mutex<HashMap<ActiveTurnKey, ActiveTurn>>>,
    grok_hardening: GrokHardeningConfig,
    backpressure: BackpressureConfig,
    session_input: Arc<super::session_input::SessionInputCoordinator>,
}

impl TurnCoordinator {
    pub fn new(kernel: Arc<KernelRuntime>, store: Arc<dyn SessionAppendStore>) -> Self {
        let event_spine = Arc::new(
            crate::r#impl::events::SqliteEventSpine::open(":memory:")
                .expect("in-memory event spine"),
        );
        let projections = Arc::new(crate::r#impl::events::DefaultEventProjectionSet::in_memory());
        Self::with_components(kernel, store, event_spine, projections)
    }

    pub fn new_with_grok(
        kernel: Arc<KernelRuntime>,
        store: Arc<dyn SessionAppendStore>,
        grok_hardening: GrokHardeningConfig,
    ) -> Self {
        let event_spine = Arc::new(
            crate::r#impl::events::SqliteEventSpine::open(":memory:")
                .expect("in-memory event spine"),
        );
        let projections = Arc::new(crate::r#impl::events::DefaultEventProjectionSet::in_memory());
        Self::with_components_and_grok(kernel, store, event_spine, projections, grok_hardening)
    }

    pub fn with_event_spine(
        kernel: Arc<KernelRuntime>,
        store: Arc<dyn SessionAppendStore>,
        event_spine: Arc<dyn EventSpine>,
    ) -> Self {
        let projections = Arc::new(crate::r#impl::events::DefaultEventProjectionSet::in_memory());
        Self::with_components(kernel, store, event_spine, projections)
    }

    pub fn with_event_spine_and_grok(
        kernel: Arc<KernelRuntime>,
        store: Arc<dyn SessionAppendStore>,
        event_spine: Arc<dyn EventSpine>,
        grok_hardening: GrokHardeningConfig,
    ) -> Self {
        let projections = Arc::new(crate::r#impl::events::DefaultEventProjectionSet::in_memory());
        Self::with_components_and_grok(kernel, store, event_spine, projections, grok_hardening)
    }

    fn with_components(
        kernel: Arc<KernelRuntime>,
        read_store: Arc<dyn SessionAppendStore>,
        event_spine: Arc<dyn EventSpine>,
        projections: Arc<dyn super::event_projection::EventProjectionSink>,
    ) -> Self {
        Self::with_components_and_grok(
            kernel,
            read_store,
            event_spine,
            projections,
            GrokHardeningConfig::default(),
        )
    }

    fn with_components_and_grok(
        kernel: Arc<KernelRuntime>,
        read_store: Arc<dyn SessionAppendStore>,
        event_spine: Arc<dyn EventSpine>,
        projections: Arc<dyn super::event_projection::EventProjectionSink>,
        grok_hardening: GrokHardeningConfig,
    ) -> Self {
        let store: Arc<dyn SessionAppendStore> = Arc::new(
            crate::r#impl::session::event_sourced_store::EventSourcedSessionStore::new(
                read_store.clone(),
                event_spine.clone(),
                projections,
            ),
        );
        Self {
            clock: kernel.clock(),
            kernel,
            read_store,
            store,
            event_spine,
            active: Arc::new(Mutex::new(HashMap::new())),
            grok_hardening,
            backpressure: BackpressureConfig::default(),
            session_input: Arc::new(super::session_input::SessionInputCoordinator::in_memory()),
        }
    }

    pub fn with_event_projections(
        mut self,
        projections: Arc<dyn super::event_projection::EventProjectionSink>,
    ) -> Self {
        self.store = Arc::new(
            crate::r#impl::session::event_sourced_store::EventSourcedSessionStore::new(
                self.read_store.clone(),
                self.event_spine.clone(),
                projections,
            ),
        );
        self
    }

    pub fn store(&self) -> Arc<dyn SessionAppendStore> {
        self.store.clone()
    }
    pub fn active_index(&self) -> Arc<Mutex<HashMap<ActiveTurnKey, ActiveTurn>>> {
        self.active.clone()
    }

    /// D2-M5-T2: set backpressure config for overload gating.
    pub fn with_backpressure(mut self, backpressure: BackpressureConfig) -> Self {
        self.backpressure = backpressure;
        self
    }

    pub fn with_session_input(
        mut self,
        session_input: Arc<super::session_input::SessionInputCoordinator>,
    ) -> Self {
        self.session_input = session_input;
        self
    }

    pub fn session_input(&self) -> Arc<super::session_input::SessionInputCoordinator> {
        self.session_input.clone()
    }

    /// Number of currently active turns across all connections.
    pub async fn active_turn_count(&self) -> usize {
        self.active.lock().await.len()
    }

    /// D2-M5-T2: check if backpressure limit is exceeded.
    pub async fn check_backpressure(&self) -> Result<(), anyhow::Error> {
        let count = self.active_turn_count().await;
        if self.backpressure.is_exceeded(count) {
            anyhow::bail!(self.backpressure.overload_message());
        }
        Ok(())
    }

    /// Cancel an in-flight turn by operation_id (legacy scan). When
    /// `grok_hardening.prompt_queue` is enabled, callers should use
    /// `cancel_operation_by_key` instead to enforce identity-aware lookup.
    pub async fn cancel_operation(&self, operation_id: fabric::OperationId) -> bool {
        let active = self.active.lock().await;
        if let Some(turn) = active
            .values()
            .find(|turn| turn.operation_id == operation_id)
        {
            turn.cancel.cancel();
            true
        } else {
            false
        }
    }

    /// Identity-aware cancel: look up by (principal_id, thread_id), verify
    /// operation_id matches, and optionally validate via `evaluate_cancel`.
    pub async fn cancel_operation_by_key(
        &self,
        principal_id: &PrincipalId,
        thread_id: &ThreadId,
        turn_id: TurnId,
        operation_id: fabric::OperationId,
    ) -> Result<(), anyhow::Error> {
        let key = ActiveTurnKey {
            principal_id: principal_id.clone(),
            thread_id: thread_id.clone(),
        };
        let active = self.active.lock().await;
        let turn = active.get(&key).ok_or_else(|| {
            anyhow::anyhow!("no active turn for principal {principal_id:?} thread {thread_id:?}")
        })?;
        if turn.operation_id != operation_id {
            anyhow::bail!(
                "operation_id mismatch: expected {:?}, got {operation_id:?}",
                turn.operation_id
            );
        }
        if turn.turn_id != turn_id {
            anyhow::bail!(
                "turn_id mismatch: expected {:?}, got {turn_id:?}",
                turn.turn_id
            );
        }
        // G3 prompt_queue identity validation when flag is enabled.
        if self.grok_hardening.prompt_queue {
            self.validate_cancel_authority(principal_id, thread_id, turn)?;
        }
        turn.cancel.cancel();
        Ok(())
    }

    pub async fn verify_active_turn(
        &self,
        principal_id: &PrincipalId,
        thread_id: &ThreadId,
        turn_id: TurnId,
        operation_id: fabric::OperationId,
    ) -> Result<(), anyhow::Error> {
        let active = self.active.lock().await;
        let turn = active
            .get(&ActiveTurnKey {
                principal_id: principal_id.clone(),
                thread_id: thread_id.clone(),
            })
            .ok_or_else(|| anyhow::anyhow!("identified turn is not active"))?;
        if turn.turn_id != turn_id || turn.operation_id != operation_id {
            anyhow::bail!("turn or operation identity does not match active turn");
        }
        Ok(())
    }

    /// Construct a synthetic PromptEnvelope (version=0) and run
    /// `evaluate_cancel` as a lightweight authority check.
    fn validate_cancel_authority(
        &self,
        principal_id: &PrincipalId,
        thread_id: &ThreadId,
        _turn: &ActiveTurn,
    ) -> Result<(), anyhow::Error> {
        // First cancel on a thread has no persisted envelope yet, so we
        // synthesize a version-0 placeholder.
        let synthetic = PromptEnvelope {
            prompt_id: fabric::types::prompt_queue::PromptId::new(),
            version: 0,
            principal_id: principal_id.clone(),
            connection_id: fabric::ConnectionId(uuid::Uuid::nil()),
            thread_id: thread_id.clone(),
            kind: PromptKind::Prompt,
            content: String::new(),
            created_at_unix: 0,
            updated_at_unix: 0,
            state: PromptState::Queued,
            idempotency_key: String::new(),
        };
        match evaluate_cancel(&synthetic, principal_id, 0) {
            fabric::types::prompt_queue::QueueOpResult::Ok { .. } => Ok(()),
            fabric::types::prompt_queue::QueueOpResult::Rejected { reason } => {
                anyhow::bail!("cancel rejected by prompt_queue authority: {reason}")
            }
            fabric::types::prompt_queue::QueueOpResult::Conflict { .. } => {
                // Version-0 synthetic can't conflict; treat as rejection.
                anyhow::bail!(
                    "cancel rejected by prompt_queue authority: version conflict on synthetic envelope"
                )
            }
        }
    }

    pub async fn submit_with<F, Fut>(
        &self,
        mut request: TurnRequest,
        _policy: &TurnPolicy,
        runner: F,
    ) -> Result<TurnResult>
    where
        F: FnOnce(TurnRequest, CancellationToken) -> Fut,
        Fut: Future<Output = Result<TurnExecution>>,
    {
        // D2-M5-T2: backpressure gate — reject new turns when overloaded.
        self.check_backpressure().await?;

        let operation = self
            .kernel
            .submit(OperationRequest {
                owner: request.process_id,
                parent: None,
                kind: OperationKind::Turn,
                deadline: request
                    .deadline
                    .map(|d| MonoDeadline::after(self.clock.mono_now(), d.0)),
            })
            .await?;
        let has_deadline = request.deadline.is_some();
        self.kernel.start_operation(operation.id).await?;
        request.operation_id = operation.id;
        request.context.turn_id = Some(TurnId::new());
        let turn_id = request.context.turn_id.unwrap_or_default();
        let cancel = CancellationToken::new();
        let active_key = ActiveTurnKey::from_request(&request);
        {
            // Admission and insertion share one lock. The earlier check is a
            // cheap fast path only; this is the authoritative race-free gate.
            let mut active = self.active.lock().await;
            if self.backpressure.is_exceeded(active.len()) {
                drop(active);
                let _ = self
                    .kernel
                    .cancel_operation(
                        operation.id,
                        CancelReason::Other("server overloaded".into()),
                    )
                    .await;
                anyhow::bail!(self.backpressure.overload_message());
            }
            if active.contains_key(&active_key) {
                drop(active);
                let _ = self
                    .kernel
                    .cancel_operation(
                        operation.id,
                        CancelReason::Other("thread already has an active turn".into()),
                    )
                    .await;
                anyhow::bail!("thread already has an active turn");
            }
            active.insert(
                active_key.clone(),
                ActiveTurn {
                    operation_id: operation.id,
                    turn_id,
                    cancel: cancel.clone(),
                },
            );
        }

        let outcome = self
            .run_started_turn(&request, cancel.clone(), runner)
            .await;

        // M4-T1: retain only a typed terminal-persistence failure. Model,
        // tool, admission, and other errors must not leak active entries.
        let terminal_write_failed = self.grok_hardening.compaction_v2
            && outcome
                .as_ref()
                .err()
                .is_some_and(is_terminal_durable_write_failure);
        if !terminal_write_failed {
            self.active.lock().await.remove(&active_key);
        } else {
            tracing::warn!(
                session = %request.context.thread_id.0,
                "turn terminal flush failed; active index entry retained for recovery scan"
            );
        }

        match outcome {
            Ok(completed) => {
                let terminal = match completed.result.stop {
                    TurnStop::Completed if completed.result.metrics.completed_normally => {
                        self.kernel.succeed_operation(operation.id).await
                    }
                    TurnStop::Cancelled => {
                        self.kernel
                            .cancel(
                                operation.id,
                                if has_deadline {
                                    CancelReason::DeadlineExceeded
                                } else {
                                    CancelReason::User
                                },
                            )
                            .await
                    }
                    _ => {
                        self.kernel
                            .fail_operation(operation.id, format!("{:?}", completed.result.stop))
                            .await
                    }
                };
                terminal?;
                if let Some(dispatch) = completed.projection {
                    tokio::spawn(async move {
                        if let Err(error) = dispatch.projector.project(dispatch.outcome).await {
                            tracing::warn!(%error, "post-turn projection failed after settlement");
                        }
                    });
                }
                Ok(completed.result)
            }
            Err(error) => {
                self.kernel
                    .fail_operation(operation.id, error.to_string())
                    .await?;
                Err(error)
            }
        }
    }

    async fn run_started_turn<F, Fut>(
        &self,
        request: &TurnRequest,
        cancel: CancellationToken,
        runner: F,
    ) -> Result<CompletedExecution>
    where
        F: FnOnce(TurnRequest, CancellationToken) -> Fut,
        Fut: Future<Output = Result<TurnExecution>>,
    {
        let session_id = SessionId(request.context.thread_id.0.clone());

        // M4-T1: track each durable write so failed terminal flush is
        // observable and leaves the active index entry intact.
        let mut write_tracker = if self.grok_hardening.compaction_v2 {
            Some(TurnWriteTracker::new())
        } else {
            None
        };

        // Track session create.
        {
            let result = if self.store.load_session(&session_id).await?.is_none() {
                self.store
                    .create(SessionRecord {
                        schema_version: SESSION_SCHEMA_VERSION,
                        id: session_id.clone(),
                        parent: None,
                        created_at_ms: self.now_ms(),
                        status: SessionStatus::Active,
                    })
                    .await
                    .map(|_| {
                        record_writer_success();
                        WriteResult::Succeeded
                    })
                    .unwrap_or_else(|e| write_failed(&e, WritePhase::SessionCreate))
            } else {
                WriteResult::Succeeded
            };
            if let Some(ref mut tracker) = write_tracker {
                tracker.record(result);
            }
        }

        let turn_id = request
            .context
            .turn_id
            .ok_or_else(|| anyhow!("turn context is missing its authoritative turn id"))?;
        let mut sequence = self.next_sequence(&session_id).await?;
        self.append_tracked(
            &session_id,
            turn_id,
            &mut sequence,
            ItemPayload::UserMessage {
                content: request.input.clone(),
            },
            WritePhase::UserMessage,
            &mut write_tracker,
        )
        .await?;

        let execution = runner(request.clone(), cancel).await;
        match execution {
            Ok(execution) => {
                let TurnExecution {
                    result,
                    items,
                    projection,
                    context_projection,
                } = execution;
                if let Some(receipt) = context_projection {
                    receipt.validate()?;
                    self.append_tracked(
                        &session_id,
                        turn_id,
                        &mut sequence,
                        ItemPayload::ContextProjection {
                            space: receipt.space.0,
                            broadcast_epoch: receipt.broadcast_epoch.map(|epoch| epoch.0),
                            workspace_version: receipt.workspace_version,
                            dasein_version: receipt.dasein_version.0,
                            content_ids: receipt
                                .content_ids
                                .into_iter()
                                .map(|id| id.0.to_string())
                                .collect(),
                        },
                        WritePhase::ContextProjection,
                        &mut write_tracker,
                    )
                    .await?;
                }
                for payload in items {
                    let phase = match &payload {
                        ItemPayload::ToolCall { .. } => WritePhase::ToolCall,
                        ItemPayload::ToolResult { .. } => WritePhase::ToolResult,
                        _ => WritePhase::ContextFragment,
                    };
                    self.append_tracked(
                        &session_id,
                        turn_id,
                        &mut sequence,
                        payload,
                        phase,
                        &mut write_tracker,
                    )
                    .await?;
                }
                let terminal = if result.stop == TurnStop::Completed {
                    ItemPayload::AssistantMessage {
                        content: result.output.clone(),
                    }
                } else {
                    ItemPayload::SystemNotice {
                        content: format!("turn stopped: {:?}", result.stop),
                    }
                };
                self.append_tracked(
                    &session_id,
                    turn_id,
                    &mut sequence,
                    terminal,
                    WritePhase::TerminalFlush,
                    &mut write_tracker,
                )
                .await?;

                // M4-T1: verify all writes succeeded.
                if let Some(ref tracker) = write_tracker {
                    if !tracker.all_succeeded() {
                        for result in &tracker.clone().into_results() {
                            if let WriteResult::Failed { reason, phase } = result {
                                tracing::error!(
                                    phase = %phase,
                                    reason = %reason,
                                    session = %session_id.0,
                                    "durable write failed during turn execution"
                                );
                            }
                        }
                        anyhow::bail!(
                            "durable write failed during turn {}: not all writes succeeded",
                            turn_id.0
                        );
                    }
                }

                Ok(CompletedExecution { result, projection })
            }
            Err(error) => {
                self.append_tracked(
                    &session_id,
                    turn_id,
                    &mut sequence,
                    ItemPayload::SystemNotice {
                        content: format!("turn failed: {error}"),
                    },
                    WritePhase::TerminalFlush,
                    &mut write_tracker,
                )
                .await
                .with_context(|| format!("turn failed: {error}; failure notice append failed"))?;

                // Error path: check write durability.
                if let Some(ref tracker) = write_tracker {
                    if !tracker.all_succeeded() {
                        tracing::error!(
                            session = %session_id.0,
                            "durable writes failed during error-path terminal flush"
                        );
                    }
                }
                Err(error)
            }
        }
    }

    /// Append an item and record the write result in the tracker (M4-T1).
    async fn append_tracked(
        &self,
        session: &SessionId,
        turn_id: TurnId,
        sequence: &mut u64,
        payload: ItemPayload,
        phase: WritePhase,
        tracker: &mut Option<TurnWriteTracker>,
    ) -> Result<()> {
        let current = *sequence;
        let item = ItemRecord {
            schema_version: SESSION_SCHEMA_VERSION,
            id: ItemId::new(),
            session_id: session.clone(),
            turn_id,
            sequence: current,
            created_at_ms: self.now_ms(),
            payload,
        };
        let result = match self.store.append(session, current, item).await {
            Ok(_) => {
                record_writer_success();
                WriteResult::Succeeded
            }
            Err(e) => write_failed(&e, phase),
        };
        if let Some(ref mut t) = tracker {
            t.record(result.clone());
        }
        if result.is_failed() {
            let reason = match &result {
                WriteResult::Failed { reason, .. } => reason.clone(),
                _ => unreachable!(),
            };
            if phase == WritePhase::TerminalFlush {
                return Err(TerminalDurableWriteFailure { reason }.into());
            }
            anyhow::bail!("append failed for phase {phase}: {}", reason);
        }
        *sequence += 1;
        Ok(())
    }

    async fn next_sequence(&self, session: &SessionId) -> Result<u64> {
        Ok(self
            .store
            .load_items(session, None)
            .await?
            .last()
            .map_or(1, |item| item.sequence + 1))
    }

    fn now_ms(&self) -> u64 {
        self.clock.wall_now().0.max(0) as u64
    }
}

pub fn cancelled_result() -> TurnResult {
    TurnResult {
        output: String::new(),
        stop: TurnStop::Cancelled,
        metrics: TurnMetrics {
            completed_normally: false,
            ..Default::default()
        },
    }
}

#[cfg(test)]
mod durable_failure_tests {
    use super::*;

    #[test]
    fn active_retention_only_recognizes_terminal_durable_write_failure() {
        let terminal: anyhow::Error = TerminalDurableWriteFailure {
            reason: "disk full".into(),
        }
        .into();
        assert!(is_terminal_durable_write_failure(&terminal));
        let wrapped = terminal.context("turn execution also failed");
        assert!(is_terminal_durable_write_failure(&wrapped));
        assert!(!is_terminal_durable_write_failure(&anyhow!(
            "model execution failed"
        )));
    }
}
