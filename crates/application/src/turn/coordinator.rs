//! Single owner of Turn operations and canonical lifecycle settlement.

use std::{future::Future, sync::Arc};

use ::contracts::{
    CancelReason, ItemId, ItemPayload, ItemRecord, MonoDeadline, PrincipalId, SessionId,
    SessionRecord, SessionStatus, ThreadId, TurnId, TurnMetrics, TurnRequest, TurnResult, TurnStop,
    SESSION_SCHEMA_VERSION,
};
use anyhow::{anyhow, Context, Result};
use runtime::turn_pipeline_lifecycle::{TurnPipelineEvent, TurnPipelineLifecycle};
use runtime::{evaluate_cancel, PromptEnvelope, PromptKind, PromptState};
use tokio_util::sync::CancellationToken;

use runtime::durable_write::{
    record_writer_success, write_failed, TurnWriteTracker, WritePhase, WriteResult,
};
use runtime::turn_policy::TurnPolicy;

/// Provider-neutral owner of the capability-preparation -> cognitive-execution
/// order. Concrete adapters remain closures so Application never imports their
/// domain or transport types.
pub async fn execute_capability_cognition_sequence<P, PFut, C, CFut, Prepared, Output>(
    prepare: P,
    cognition: C,
) -> Result<Output>
where
    P: FnOnce() -> PFut,
    PFut: Future<Output = Result<Prepared>>,
    C: FnOnce(Prepared) -> CFut,
    CFut: Future<Output = Result<Output>>,
{
    let prepared = prepare().await?;
    cognition(prepared).await
}

/// Cloneable, synchronously-serial handle to the canonical turn lifecycle
/// reducer. The lock is held only inside the synchronous `apply` transition and
/// never crosses an `.await`; clones share one reducer instance, so the
/// execution envelope and the inner execution closure advance the same state
/// machine without borrowing a `&mut` across a future boundary.
#[derive(Debug, Clone)]
pub struct TurnLifecycleHandle(Arc<std::sync::Mutex<TurnPipelineLifecycle>>);

impl TurnLifecycleHandle {
    pub fn new() -> Self {
        Self(Arc::new(std::sync::Mutex::new(
            TurnPipelineLifecycle::default(),
        )))
    }

    pub fn apply(
        &self,
        event: TurnPipelineEvent,
    ) -> Result<
        runtime::turn_pipeline_lifecycle::TurnPipelineTransition,
        runtime::turn_pipeline_lifecycle::InvalidTurnPipelineTransition,
    > {
        let mut lifecycle = self
            .0
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner());
        lifecycle.apply(event)
    }
}

/// Provider-neutral owner of the turn execution envelope: workspace checkpoint
/// begin, inner execution, fail/cancel terminal application, best-effort abort
/// dispatch, and checkpoint finalize ordering.
///
/// The host supplies closures for every concrete effect; Application decides
/// only the relative order and terminal policy. The fixed order is:
///
/// 1. begin checkpoint (fail-visible);
/// 2. execute the inner turn. The execution closure receives a cloneable
///    lifecycle handle and owns every intermediate lifecycle event;
/// 3. only when execution is `Err`, apply the Fail/Cancel terminal and dispatch
///    the abort notification (best-effort — its error is recorded but never
///    overrides the original pipeline error);
/// 4. finalize the checkpoint for every outcome. Only a genuinely completed
///    turn finalizes success; a Rejected or failed turn aborts the checkpoint.
///    Finalize errors remain fail-visible;
/// 5. return the original execution result.
///
/// `Rejected` arrives as `Ok(TurnPipelineOutcome::Rejected)`: it aborts the
/// checkpoint but must not dispatch an execution-error `on_abort`. Callers must
/// not reorder these steps to work around a borrow — the ordering is the
/// contract.
pub async fn run_turn_execution_envelope<
    CheckpointId,
    Output,
    BeginFut,
    ExecuteFut,
    AbortFut,
    FinalizeFut,
>(
    begin_checkpoint: impl FnOnce() -> BeginFut,
    execute: impl FnOnce(TurnLifecycleHandle) -> ExecuteFut,
    observe_cancellation: impl Fn() -> bool,
    dispatch_abort: impl FnOnce() -> AbortFut,
    finalize_checkpoint: impl FnOnce(CheckpointId, bool) -> FinalizeFut,
    classify_completed: impl Fn(&Output) -> bool,
) -> Result<Output>
where
    BeginFut: Future<Output = Result<Option<CheckpointId>>>,
    ExecuteFut: Future<Output = Result<Output>>,
    AbortFut: Future<Output = Result<()>>,
    FinalizeFut: Future<Output = Result<()>>,
{
    let lifecycle = TurnLifecycleHandle::new();
    let checkpoint_id = begin_checkpoint().await?;
    let result = execute(lifecycle.clone()).await;
    if result.is_err() {
        let terminal = if observe_cancellation() {
            TurnPipelineEvent::Cancel
        } else {
            TurnPipelineEvent::Fail
        };
        // The reducer is non-terminal here (an `Err` cannot surface after the
        // terminal projection event), so Fail/Cancel is always valid.
        lifecycle.apply(terminal)?;
        if let Err(abort_error) = dispatch_abort().await {
            tracing::warn!(
                error = %abort_error,
                "turn abort dispatch failed while preserving original pipeline error"
            );
        }
    }
    if let Some(checkpoint_id) = checkpoint_id {
        let succeeded = result
            .as_ref()
            .is_ok_and(|outcome| classify_completed(outcome));
        finalize_checkpoint(checkpoint_id, succeeded).await?;
    }
    result
}

pub use crate::turn::outcome::TurnExecution;

struct CompletedExecution {
    result: TurnResult,
    projection: Option<crate::turn::post_turn::PostTurnDispatch>,
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

fn runtime_terminal_for_stop(result: &TurnResult) -> runtime::TurnTerminal {
    match result.stop {
        TurnStop::Completed if result.metrics.completed_normally => {
            runtime::TurnTerminal::Completed
        }
        TurnStop::Cancelled => runtime::TurnTerminal::Interrupted,
        _ => runtime::TurnTerminal::Failed {
            message: format!("{:?}", result.stop),
        },
    }
}

async fn timeout_with_timer<F>(
    timer: &dyn crate::turn::ports::TurnTimerPort,
    duration: std::time::Duration,
    future: F,
) -> Result<F::Output, ::contracts::Elapsed>
where
    F: Future + Send,
    F::Output: Send,
{
    tokio::pin!(future);
    tokio::select! {
        output = &mut future => Ok(output),
        () = timer.sleep(duration) => Err(::contracts::Elapsed),
    }
}

const CANCEL_DRAIN_GRACE: std::time::Duration = std::time::Duration::from_secs(5);

/// One exact, first-writer-wins abort reason paired with the turn cancellation
/// token. A token alone cannot distinguish user cancellation, deadline,
/// disconnect, or shutdown, and deriving the reason from the mere presence of
/// a deadline mislabels user cancellation on deadline-bearing turns.
/// Guarantees coordinator settlement survives the caller's future being
/// dropped. When a transport connection aborts the request future (disconnect),
/// the `submit_with` await is cancelled before the active-index removal and
/// kernel settlement tail runs. This guard's `Drop` runs in that case and
/// spawns a bounded settlement task so the exact operation reaches terminal
/// state exactly once. The normal path marks it settled first, so the two
/// cannot double-settle.
struct TurnSettlementGuard {
    operations: Arc<dyn crate::turn::ports::TurnOperationPort>,
    active: Arc<runtime::ActiveTurnRegistry>,
    active_key: ActiveTurnKey,
    operation_id: ::contracts::OperationId,
    abort: runtime::TurnCancellation,
    settled: std::sync::Arc<std::sync::atomic::AtomicBool>,
    runtime_turn_writer: Arc<dyn runtime::TurnLifecycleWriter>,
    runtime_session: runtime::SessionId,
    runtime_turn: runtime::TurnId,
}

impl TurnSettlementGuard {
    fn new(
        operations: Arc<dyn crate::turn::ports::TurnOperationPort>,
        active: Arc<runtime::ActiveTurnRegistry>,
        active_key: ActiveTurnKey,
        operation_id: ::contracts::OperationId,
        abort: runtime::TurnCancellation,
        runtime_turn_writer: Arc<dyn runtime::TurnLifecycleWriter>,
        runtime_session: runtime::SessionId,
        runtime_turn: runtime::TurnId,
    ) -> Self {
        Self {
            operations,
            active,
            active_key,
            operation_id,
            abort,
            settled: std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false)),
            runtime_turn_writer,
            runtime_session,
            runtime_turn,
        }
    }

    /// Marks settlement as completed by the normal path; the `Drop` fallback
    /// then becomes a no-op.
    fn mark_settled(&self) {
        self.settled
            .store(true, std::sync::atomic::Ordering::SeqCst);
    }

    fn is_settled(&self) -> bool {
        self.settled.load(std::sync::atomic::Ordering::SeqCst)
    }
}

impl Drop for TurnSettlementGuard {
    fn drop(&mut self) {
        if self.is_settled() {
            return;
        }
        // The caller future was dropped before the settlement tail ran (for
        // example the connection disconnected). Perform an idempotent cancel
        // settlement in a bounded task owned by the kernel Arc so the active
        // index and kernel operation still reach terminal state.
        let operations = self.operations.clone();
        let active = self.active.clone();
        let active_key = self.active_key.clone();
        let operation_id = self.operation_id;
        let abort = self.abort.clone();
        let runtime_turn_writer = self.runtime_turn_writer.clone();
        let runtime_session = self.runtime_session.clone();
        let runtime_turn = self.runtime_turn.clone();
        let runner_panicked = std::thread::panicking();
        tokio::spawn(async move {
            active.lock().await.remove(&active_key);
            if runner_panicked {
                abort.request(CancelReason::Other("turn runner panicked".into()));
                let _ = runtime_turn_writer
                    .crash(&runtime_session, &runtime_turn)
                    .await;
                let _ = runtime_turn_writer
                    .settle(
                        &runtime_session,
                        &runtime_turn,
                        runtime::TurnTerminal::Failed {
                            message: "turn runner panicked".into(),
                        },
                    )
                    .await;
                let _ = operations
                    .fail(operation_id, "turn runner panicked".into())
                    .await;
                tracing::error!(
                    operation = %operation_id.0,
                    "turn runner panicked; coordinator settlement completed by guard"
                );
            } else {
                abort.request(CancelReason::Other(
                    "transport request future dropped".into(),
                ));
                let _ = runtime_turn_writer
                    .disconnect(&runtime_session, &runtime_turn)
                    .await;
                let _ = runtime_turn_writer
                    .settle(
                        &runtime_session,
                        &runtime_turn,
                        runtime::TurnTerminal::Interrupted,
                    )
                    .await;
                let _ = operations
                    .cancel(operation_id, abort.reason().unwrap_or(CancelReason::User))
                    .await;
                tracing::warn!(
                    operation = %operation_id.0,
                    "turn request future dropped; coordinator settlement completed by guard"
                );
            }
        });
    }
}

pub use runtime::{ActiveTurn, ActiveTurnKey};

#[derive(Debug, Clone, Default, serde::Serialize)]
pub struct TurnWatchdogSnapshot {
    pub active: usize,
    pub oldest_age_ms: u64,
    pub overdue: usize,
    pub stale_without_deadline: usize,
    /// Process-global scope gauges (typed host facts, never model-reported).
    /// Exposed so installed acceptance can prove the zero-resource invariant.
    pub active_scopes: usize,
    pub active_resources: usize,
    pub drop_fallbacks: usize,
    /// Outstanding external resources whose terminal reclaim was not observed
    /// before their scope dropped. Non-zero means cleanup is still pending.
    pub pending_reclaim: usize,
    /// Dropped scopes rejected by the bounded recovery queue. This is a
    /// fail-visible saturation counter, not a clean resource state.
    pub reclaim_queue_exhausted: usize,
}

pub struct TurnCoordinator {
    operations: Arc<dyn crate::turn::ports::TurnOperationPort>,
    identities: Arc<dyn crate::turn::ports::TurnIdentityPort>,
    clock: Arc<dyn ::contracts::Clock>,
    timer: Arc<dyn crate::turn::ports::TurnTimerPort>,
    session: Arc<dyn crate::turn::ports::TurnSessionPort>,
    active: Arc<runtime::ActiveTurnRegistry>,
    settings: crate::turn::settings::TurnCoordinatorSettings,
    session_input: Arc<crate::session_input::SessionInputCoordinator>,
    evaluation: Option<Arc<crate::evaluation::EvaluationService>>,
    host_acceptance: Option<Arc<dyn crate::evaluation::EvaluationAcceptancePort>>,
    runtime_turn_writer: Arc<dyn runtime::TurnLifecycleWriter>,
    canonical_session_guard: bool,
}

pub struct TurnCoordinatorResources {
    pub clock: Arc<dyn ::contracts::Clock>,
    pub timer: Arc<dyn crate::turn::ports::TurnTimerPort>,
    pub operations: Arc<dyn crate::turn::ports::TurnOperationPort>,
    pub identities: Arc<dyn crate::turn::ports::TurnIdentityPort>,
    pub session: Arc<dyn crate::turn::ports::TurnSessionPort>,
    pub settings: crate::turn::settings::TurnCoordinatorSettings,
    pub runtime_turn_writer: Arc<dyn runtime::TurnLifecycleWriter>,
}

impl TurnCoordinator {
    /// Construct only from consumer ports and immutable settings normalized by
    /// composition. No concrete host runtime crosses this boundary.
    pub fn from_resources(resources: TurnCoordinatorResources) -> Self {
        Self {
            clock: resources.clock,
            timer: resources.timer,
            operations: resources.operations,
            identities: resources.identities,
            session: resources.session,
            active: Arc::new(runtime::ActiveTurnRegistry::new()),
            settings: resources.settings,
            session_input: Arc::new(crate::session_input::SessionInputCoordinator::in_memory()),
            evaluation: None,
            host_acceptance: None,
            runtime_turn_writer: resources.runtime_turn_writer,
            canonical_session_guard: false,
        }
    }

    pub fn with_timer(mut self, timer: Arc<dyn crate::turn::ports::TurnTimerPort>) -> Self {
        self.timer = timer;
        self
    }

    pub fn session_port(&self) -> Arc<dyn crate::turn::ports::TurnSessionPort> {
        self.session.clone()
    }
    pub fn active_index(&self) -> Arc<runtime::ActiveTurnRegistry> {
        self.active.clone()
    }

    /// D2-M5-T2: set backpressure config for overload gating.
    pub fn with_backpressure(
        mut self,
        backpressure: runtime::backpressure::BackpressureConfig,
    ) -> Self {
        self.settings.backpressure = backpressure;
        self
    }

    pub fn with_session_input(
        mut self,
        session_input: Arc<crate::session_input::SessionInputCoordinator>,
    ) -> Self {
        self.session_input = session_input;
        self
    }

    pub fn with_evaluation_service(
        mut self,
        evaluation: Arc<crate::evaluation::EvaluationService>,
    ) -> Self {
        self.evaluation = Some(evaluation);
        self
    }

    pub fn with_host_acceptance(
        mut self,
        controller: Arc<dyn crate::evaluation::EvaluationAcceptancePort>,
    ) -> Self {
        self.host_acceptance = Some(controller);
        self
    }

    /// Production composition enables this guard after injecting the shared
    /// canonical SessionInfrastructure. Read-only unit fixtures keep the
    /// legacy in-memory session contract.
    pub fn with_canonical_session_guard(mut self, enabled: bool) -> Self {
        self.canonical_session_guard = enabled;
        self
    }

    pub fn session_input(&self) -> Arc<crate::session_input::SessionInputCoordinator> {
        self.session_input.clone()
    }

    /// Number of currently active turns across all connections.
    pub async fn active_turn_count(&self) -> usize {
        self.active.lock().await.len()
    }

    pub async fn watchdog_snapshot(&self, stale_after_ms: u64) -> TurnWatchdogSnapshot {
        let now = self.clock.mono_now();
        let active = self.active.lock().await;
        let mut snapshot = TurnWatchdogSnapshot {
            active: active.len(),
            ..Default::default()
        };
        for turn in active.values() {
            let age = now.0.saturating_sub(turn.started_at.0);
            snapshot.oldest_age_ms = snapshot.oldest_age_ms.max(age);
            if turn
                .deadline_at
                .is_some_and(|deadline| deadline.is_expired_at(now))
            {
                snapshot.overdue += 1;
            } else if turn.deadline_at.is_none() && age >= stale_after_ms {
                snapshot.stale_without_deadline += 1;
            }
        }
        // Typed process-global scope gauges from the kernel operation layer.
        // These are authoritative host facts, never derived from model output.
        let scope_metrics = self.operations.metrics();
        snapshot.active_scopes = scope_metrics.active_scopes;
        snapshot.active_resources = scope_metrics.active_resources;
        snapshot.drop_fallbacks = scope_metrics.drop_fallbacks;
        snapshot.pending_reclaim = scope_metrics.pending_reclaim;
        snapshot.reclaim_queue_exhausted = scope_metrics.reclaim_queue_exhausted;
        snapshot
    }

    /// D2-M5-T2: check if backpressure limit is exceeded.
    pub async fn check_backpressure(&self) -> Result<(), anyhow::Error> {
        let count = self.active_turn_count().await;
        if self.settings.backpressure.is_exceeded(count) {
            anyhow::bail!(self.settings.backpressure.overload_message());
        }
        Ok(())
    }

    /// Cancel an in-flight turn by operation_id (legacy scan). When
    /// `grok_hardening.prompt_queue` is enabled, callers should use
    /// `cancel_operation_by_key` instead to enforce identity-aware lookup.
    pub async fn cancel_operation(&self, operation_id: ::contracts::OperationId) -> bool {
        let active = self.active.lock().await;
        if let Some(turn) = active
            .values()
            .find(|turn| turn.operation_id == operation_id)
        {
            turn.cancel(CancelReason::User);
            true
        } else {
            false
        }
    }

    /// Cancel every active turn owned by one authenticated principal.
    ///
    /// Compatibility clients can request cancellation before receiving the
    /// versioned turn and operation identifiers.  Scoping this fallback by
    /// principal prevents one connection from cancelling another user's work.
    pub async fn cancel_active_for_principal(&self, principal_id: &PrincipalId) -> usize {
        let active = self.active.lock().await;
        let mut cancelled = 0;
        for (key, turn) in active.iter() {
            if &key.principal_id == principal_id {
                turn.cancel(CancelReason::User);
                cancelled += 1;
            }
        }
        cancelled
    }

    /// Cancel exactly the active turns admitted by one transport connection.
    ///
    /// This is the disconnect path: a dropped connection cancels the turns it
    /// owns without touching turns owned by other connections, even for the
    /// same principal/thread. Each cancelled turn's coordinator settlement is
    /// guaranteed by the settlement guard, so the active index and kernel
    /// operation still reach terminal state after the cancel token fires.
    pub async fn cancel_active_for_connection(
        &self,
        connection_id: &::contracts::ConnectionId,
    ) -> usize {
        let active = self.active.lock().await;
        let mut cancelled = 0;
        for turn in active.values() {
            if &turn.connection_id == connection_id {
                turn.cancel(CancelReason::Other(
                    "transport connection disconnected".into(),
                ));
                cancelled += 1;
            }
        }
        cancelled
    }

    pub async fn cancel_all_active(&self) -> usize {
        let active = self.active.lock().await;
        for turn in active.values() {
            turn.cancel(CancelReason::Shutdown);
        }
        active.len()
    }

    /// Identity-aware cancel: look up by (principal_id, thread_id), verify
    /// operation_id matches, and optionally validate via `evaluate_cancel`.
    pub async fn cancel_operation_by_key(
        &self,
        principal_id: &PrincipalId,
        thread_id: &ThreadId,
        turn_id: TurnId,
        operation_id: ::contracts::OperationId,
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
        if self.settings.prompt_queue {
            self.validate_cancel_authority(principal_id, thread_id, turn)?;
        }
        turn.cancel(CancelReason::User);
        Ok(())
    }

    pub async fn verify_active_turn(
        &self,
        principal_id: &PrincipalId,
        thread_id: &ThreadId,
        turn_id: TurnId,
        operation_id: ::contracts::OperationId,
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
            prompt_id: runtime::prompt_queue::PromptId::new(),
            version: 0,
            principal_id: principal_id.clone(),
            connection_id: ::contracts::ConnectionId(uuid::Uuid::nil()),
            thread_id: thread_id.clone(),
            kind: PromptKind::Prompt,
            content: String::new(),
            requirements: Vec::new(),
            requested_task_kind: None,
            execution_target: ::contracts::ExecutionTargetSelection::default(),
            created_at_unix: 0,
            updated_at_unix: 0,
            state: PromptState::Queued,
            idempotency_key: String::new(),
        };
        match evaluate_cancel(&synthetic, principal_id, 0) {
            runtime::prompt_queue::QueueOpResult::Ok { .. } => Ok(()),
            runtime::prompt_queue::QueueOpResult::Rejected { reason } => {
                anyhow::bail!("cancel rejected by prompt_queue authority: {reason}")
            }
            runtime::prompt_queue::QueueOpResult::Conflict { .. } => {
                // Version-0 synthetic can't conflict; treat as rejection.
                anyhow::bail!(
                    "cancel rejected by prompt_queue authority: version conflict on synthetic envelope"
                )
            }
        }
    }

    pub async fn submit_with<F, Fut>(
        &self,
        request: TurnRequest,
        _policy: &TurnPolicy,
        runner: F,
    ) -> Result<TurnResult>
    where
        F: FnOnce(TurnRequest, CancellationToken) -> Fut,
        Fut: Future<Output = Result<TurnExecution>> + Send,
    {
        self.submit_with_receipt(request, _policy, None, runner)
            .await
    }

    /// Admit a turn and optionally publish the Runtime-minted receipt before
    /// executing the cognitive pipeline. The receipt channel is transport
    /// correlation only; the Runtime writer remains the sole identity and
    /// terminal authority. This is the async Gateway admission seam.
    pub async fn submit_with_receipt<F, Fut>(
        &self,
        mut request: TurnRequest,
        _policy: &TurnPolicy,
        receipt: Option<tokio::sync::oneshot::Sender<Result<runtime::TurnId, String>>>,
        runner: F,
    ) -> Result<TurnResult>
    where
        F: FnOnce(TurnRequest, CancellationToken) -> Fut,
        Fut: Future<Output = Result<TurnExecution>> + Send,
    {
        // D2-M5-T2: backpressure gate — reject new turns when overloaded.
        self.check_backpressure().await?;

        let operation_id = self
            .operations
            .admit(
                request.process_id,
                request
                    .deadline
                    .map(|d| MonoDeadline::after(self.clock.mono_now(), d.0)),
            )
            .await
            .context("admitting turn operation")?;
        request.operation_id = operation_id;
        let contract_session = self
            .identities
            .contract_session(&request.context.thread_id)?;
        let runtime_session = self.identities.runtime_session(&contract_session)?;
        // A turn may only be admitted for a durable canonical Session. This
        // rejects transport/thread aliases before Runtime mints a TurnId and
        // prevents a caller-provided thread id from becoming a second session
        // authority.
        if self.canonical_session_guard
            && self
                .session
                .load_session(&contract_session)
                .await
                .context("loading canonical session before Runtime turn admission")?
                .is_none()
        {
            let _ = self
                .operations
                .fail(operation_id, "canonical session not found".into())
                .await;
            anyhow::bail!("canonical session not found: {}", runtime_session.0);
        }
        let runtime_turn = self
            .runtime_turn_writer
            .start_turn(&runtime_session)
            .await
            .map_err(|error| {
                anyhow::anyhow!(
                    "Runtime turn start rejected for session {} and input {:?}: {error}",
                    runtime_session.0,
                    request.input
                )
            })?;
        if let Some(receipt) = receipt {
            let _ = receipt.send(Ok(runtime_turn.clone()));
        }
        let fabric_turn = uuid::Uuid::parse_str(&runtime_turn.0)
            .map(::contracts::TurnId)
            .map_err(|error| anyhow::anyhow!("Runtime turn id is not a UUID: {error}"))?;
        request.context.turn_id = Some(fabric_turn);
        let turn_id = request.context.turn_id.unwrap_or_default();
        request.evaluation_contract = if let Some(evaluation) = &self.evaluation {
            match evaluation.issue_contract(&request).await {
                Ok(contract) => contract,
                Err(error) => {
                    let _ = self
                        .runtime_turn_writer
                        .settle(
                            &runtime_session,
                            &runtime_turn,
                            runtime::TurnTerminal::Failed {
                                message: error.to_string(),
                            },
                        )
                        .await;
                    self.operations
                        .fail(operation_id, error.to_string())
                        .await?;
                    return Err(error);
                }
            }
        } else {
            None
        };
        let abort = runtime::TurnCancellation::new();
        let active_key = ActiveTurnKey::from_context(&request.context);
        {
            // Admission and insertion share one lock. The earlier check is a
            // cheap fast path only; this is the authoritative race-free gate.
            let mut active = self.active.lock().await;
            if self.settings.backpressure.is_exceeded(active.len()) {
                drop(active);
                let _ = self
                    .operations
                    .cancel(
                        operation_id,
                        CancelReason::Other("server overloaded".into()),
                    )
                    .await;
                anyhow::bail!(self.settings.backpressure.overload_message());
            }
            if active.contains_key(&active_key) {
                drop(active);
                let _ = self
                    .operations
                    .cancel(
                        operation_id,
                        CancelReason::Other("thread already has an active turn".into()),
                    )
                    .await;
                anyhow::bail!("thread already has an active turn");
            }
            active.insert(
                active_key.clone(),
                ActiveTurn {
                    operation_id,
                    turn_id,
                    canonical_turn_id: runtime_turn.clone(),
                    connection_id: request.context.connection_id.clone(),
                    cancellation: abort.clone(),
                    started_at: self.clock.mono_now(),
                    deadline_at: request
                        .deadline
                        .map(|d| MonoDeadline::after(self.clock.mono_now(), d.0)),
                },
            );
        }

        // Settlement guard: if the caller's future is dropped before the
        // settlement tail below runs (transport disconnect), the guard's Drop
        // performs an idempotent cancel settlement so the active index and
        // kernel operation still reach terminal state.
        let settlement_guard = TurnSettlementGuard::new(
            self.operations.clone(),
            self.active.clone(),
            active_key.clone(),
            operation_id,
            abort.clone(),
            self.runtime_turn_writer.clone(),
            runtime_session.clone(),
            runtime_turn.clone(),
        );

        let outcome = self.run_started_turn(&request, abort.clone(), runner).await;

        // M4-T1: retain only a typed terminal-persistence failure. Model,
        // tool, admission, and other errors must not leak active entries.
        let terminal_write_failed = self.settings.compaction_v2
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
                        self.operations.succeed(operation_id).await
                    }
                    TurnStop::Cancelled => {
                        self.operations
                            .cancel(operation_id, abort.reason().unwrap_or(CancelReason::User))
                            .await
                    }
                    _ => {
                        self.operations
                            .fail(operation_id, format!("{:?}", completed.result.stop))
                            .await
                    }
                };
                // `run_started_turn` (including its cancellation/deadline
                // paths) has already committed the Runtime terminal before
                // writing the compatibility projection. Do not call the
                // terminal writer a second time from this outer facade.
                terminal?;
                if let Some(dispatch) = completed.projection {
                    tokio::spawn(async move {
                        if let Err(error) = dispatch.projector.project(dispatch.outcome).await {
                            tracing::warn!(%error, "post-turn projection failed after settlement");
                        }
                    });
                }
                settlement_guard.mark_settled();
                Ok(completed.result)
            }
            Err(error) => {
                // A terminal projection failure occurs after the Runtime
                // fence has already been committed. Retrying that fence as a
                // Failed terminal would create a false terminal conflict and
                // hide the durable-write recovery boundary from the caller.
                if !is_terminal_durable_write_failure(&error) {
                    self.runtime_turn_writer
                        .settle(
                            &runtime_session,
                            &runtime_turn,
                            runtime::TurnTerminal::Failed {
                                message: error.to_string(),
                            },
                        )
                        .await
                        .map_err(|settle_error| anyhow::anyhow!(settle_error.to_string()))?;
                }
                self.operations
                    .fail(operation_id, error.to_string())
                    .await?;
                settlement_guard.mark_settled();
                Err(error)
            }
        }
    }

    async fn run_started_turn<F, Fut>(
        &self,
        request: &TurnRequest,
        abort: runtime::TurnCancellation,
        runner: F,
    ) -> Result<CompletedExecution>
    where
        F: FnOnce(TurnRequest, CancellationToken) -> Fut,
        Fut: Future<Output = Result<TurnExecution>> + Send,
    {
        let session_id = self
            .identities
            .contract_session(&request.context.thread_id)?;

        // M4-T1: track each durable write so failed terminal flush is
        // observable and leaves the active index entry intact.
        let mut write_tracker = if self.settings.compaction_v2 {
            Some(TurnWriteTracker::new())
        } else {
            None
        };

        // Track session create.
        {
            let result = if self.session.load_session(&session_id).await?.is_none() {
                self.session
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
        // Persist the authenticated owner separately from the Session record
        // so a restart or a picker request cannot widen principal visibility.
        self.session
            .bind_principal(&session_id, &request.context.principal_id)
            .await
            .context("bind session principal")?;

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
                execution_target: request.execution_target.clone(),
            },
            WritePhase::UserMessage,
            &mut write_tracker,
        )
        .await?;

        let runner = runner(request.clone(), abort.token());
        tokio::pin!(runner);
        let execution = if let Some(deadline) = request.deadline {
            let deadline_duration = std::time::Duration::from_millis(deadline.0);
            tokio::select! {
                // Cancellation is authoritative: when the abort token fires it
                // must settle as a typed cancelled result even if the runner
                // surfaces its own error at the same instant. `biased;` with
                // the cancel branch first keeps that outcome deterministic.
                biased;
                () = abort.cancelled() => {
                    let reason = abort.reason().unwrap_or(CancelReason::User);
                    let _ = timeout_with_timer(self.timer.as_ref(), CANCEL_DRAIN_GRACE, &mut runner).await;
                    return self.finish_cancelled_execution(
                        &session_id,
                        turn_id,
                        &mut write_tracker,
                        reason,
                    ).await;
                }
                execution = &mut runner => execution,
                () = self.timer.sleep(deadline_duration) => {
                    abort.request(CancelReason::DeadlineExceeded);
                    let reason = abort
                        .reason()
                        .unwrap_or(CancelReason::DeadlineExceeded);
                    tracing::warn!(
                        deadline_ms = deadline_duration.as_millis(),
                        "turn deadline expired; entering the authoritative abort protocol"
                    );
                    let _ = timeout_with_timer(self.timer.as_ref(), CANCEL_DRAIN_GRACE, &mut runner).await;
                    return self.finish_cancelled_execution(
                        &session_id,
                        turn_id,
                        &mut write_tracker,
                        reason,
                    ).await;
                }
            }
        } else {
            tokio::select! {
                // See the deadline select above: the cancel branch is biased
                // first so a cancelled turn settles as a typed result rather
                // than racing the runner's own error.
                biased;
                () = abort.cancelled() => {
                    let reason = abort.reason().unwrap_or(CancelReason::User);
                    let _ = timeout_with_timer(self.timer.as_ref(), CANCEL_DRAIN_GRACE, &mut runner).await;
                    return self.finish_cancelled_execution(
                        &session_id,
                        turn_id,
                        &mut write_tracker,
                        reason,
                    ).await;
                }
                execution = &mut runner => execution,
            }
        };
        // The runner may persist model-visible lifecycle fragments and the
        // host-authoritative turn-start budget before inference. Continue from
        // the durable tail rather than the sequence cached before the runner.
        sequence = self.next_sequence(&session_id).await?;
        match execution {
            Ok(execution) => {
                let TurnExecution {
                    mut result,
                    items,
                    projection,
                    context_projection,
                    evaluation_artifacts,
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
                if let Some(contract) = &request.evaluation_contract {
                    let evaluation = self.evaluation.as_ref().ok_or_else(|| {
                        anyhow!("evaluation contract exists without an evaluation service")
                    })?;
                    match evaluation
                        .evaluate(
                            contract,
                            &evaluation_artifacts,
                            request.process_id,
                            request.operation_id,
                        )
                        .await
                    {
                        Ok(receipt) => {
                            if let Some(controller) = &self.host_acceptance {
                                controller
                                    .settle(
                                        &evaluation_artifacts.session_id,
                                        &receipt.reference(),
                                        request.process_id,
                                    )
                                    .await?;
                            }
                            result.stop =
                                crate::evaluation::EvaluationSettlementPolicy::settle_stop(
                                    receipt.decision,
                                    result.stop,
                                );
                            if result.stop != TurnStop::Completed {
                                result.metrics.completed_normally = false;
                            }
                            self.append_tracked(
                                &session_id,
                                turn_id,
                                &mut sequence,
                                ItemPayload::EvaluationReceiptRef {
                                    receipt: receipt.reference(),
                                },
                                WritePhase::EvaluationReceipt,
                                &mut write_tracker,
                            )
                            .await?;
                        }
                        Err(error) => {
                            tracing::error!(
                                %error,
                                mode = ?contract.mode,
                                turn = %turn_id.0,
                                "coding evaluation failed without a receipt"
                            );
                            self.append_tracked(
                                &session_id,
                                turn_id,
                                &mut sequence,
                                ItemPayload::SystemNotice {
                                    content: format!("evaluation indeterminate: {error}"),
                                },
                                WritePhase::EvaluationReceipt,
                                &mut write_tracker,
                            )
                            .await?;
                            if contract.mode == ::contracts::EvaluationMode::Enforce {
                                result.stop = TurnStop::Failed;
                                result.metrics.completed_normally = false;
                            }
                        }
                    }
                }
                if result.stop == TurnStop::Cancelled && result.output.trim().is_empty() {
                    result.output = cancelled_result().output;
                }
                // Runtime is the canonical terminal writer. The legacy
                // canonical session item below is only the compatibility
                // projection and is written after this fence succeeds.
                self.settle_runtime_before_projection(
                    &session_id,
                    &turn_id,
                    runtime_terminal_for_stop(&result),
                )
                .await?;
                let terminal = if result.stop == TurnStop::Completed {
                    ItemPayload::AssistantMessage {
                        content: result.output.clone(),
                    }
                } else if result.stop == TurnStop::Cancelled {
                    ItemPayload::TurnSettlement {
                        status: ::contracts::TurnTerminalStatus::Interrupted,
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
                        return Err(TerminalDurableWriteFailure {
                            reason: format!(
                                "durable write failed during turn {}: not all writes succeeded",
                                turn_id.0
                            ),
                        }
                        .into());
                    }
                }

                Ok(CompletedExecution { result, projection })
            }
            Err(error) => {
                self.settle_runtime_before_projection(
                    &session_id,
                    &turn_id,
                    runtime::TurnTerminal::Failed {
                        message: error.to_string(),
                    },
                )
                .await?;
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

    /// Persist the one authoritative cancelled terminal item before allowing
    /// the coordinator to settle the kernel operation. The runner is already
    /// drained (or forcibly dropped after the bounded grace period) when this
    /// method is entered, so no later runner output can race this terminal.
    async fn finish_cancelled_execution(
        &self,
        session_id: &SessionId,
        turn_id: TurnId,
        write_tracker: &mut Option<TurnWriteTracker>,
        reason: CancelReason,
    ) -> Result<CompletedExecution> {
        // The runner may have persisted model-visible lifecycle fragments
        // before observing cancellation. It is fully drained at this point, so
        // refresh the authoritative tail instead of reusing the sequence that
        // preceded runner execution.
        let mut sequence = self.next_sequence(session_id).await?;
        match &reason {
            CancelReason::DeadlineExceeded => self
                .runtime_turn_writer
                .timeout(
                    &self.identities.runtime_session(session_id)?,
                    &self.identities.runtime_turn(turn_id)?,
                )
                .await
                .map_err(|error| anyhow!(error.to_string()))?,
            _ => self
                .runtime_turn_writer
                .cancel(
                    &self.identities.runtime_session(session_id)?,
                    &self.identities.runtime_turn(turn_id)?,
                )
                .await
                .map_err(|error| anyhow!(error.to_string()))?,
        };
        self.settle_runtime_before_projection(
            session_id,
            &turn_id,
            runtime::TurnTerminal::Interrupted,
        )
        .await?;
        self.append_tracked(
            session_id,
            turn_id,
            &mut sequence,
            ItemPayload::TurnSettlement {
                status: ::contracts::TurnTerminalStatus::Interrupted,
                content: format!(
                    "Turn cancelled ({reason:?}). The cancelled turn objective is closed."
                ),
            },
            WritePhase::TerminalFlush,
            write_tracker,
        )
        .await?;

        if write_tracker
            .as_ref()
            .is_some_and(|tracker| !tracker.all_succeeded())
        {
            return Err(TerminalDurableWriteFailure {
                reason: format!("cancel terminal was not durable for turn {}", turn_id.0),
            }
            .into());
        }

        Ok(CompletedExecution {
            result: cancelled_result(),
            projection: None,
        })
    }

    async fn settle_runtime_before_projection(
        &self,
        session_id: &SessionId,
        turn_id: &::contracts::TurnId,
        terminal: runtime::TurnTerminal,
    ) -> Result<()> {
        self.runtime_turn_writer
            .settle(
                &self.identities.runtime_session(session_id)?,
                &self.identities.runtime_turn(*turn_id)?,
                terminal,
            )
            .await
            .map_err(|error| anyhow::anyhow!(error.to_string()))
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
            // Session and Item identities are separate namespaces. Binding the
            // one terminal item to the authoritative Turn UUID gives terminal
            // settlement a stable idempotency key across an ambiguous append
            // acknowledgement while leaving all non-terminal items unique.
            id: if phase == WritePhase::TerminalFlush {
                ItemId(turn_id.0)
            } else {
                ItemId::new()
            },
            session_id: session.clone(),
            turn_id,
            sequence: current,
            created_at_ms: self.now_ms(),
            payload,
        };
        let first_append = self.session.append(session, current, item.clone()).await;
        let append = match first_append {
            // A terminal write can commit durably and still lose its response.
            // Retry the exact same ItemRecord once; TurnSessionPort treats
            // the stable ItemId plus identical payload as AlreadyPresent.
            Err(first_error) if phase == WritePhase::TerminalFlush => self
                .session
                .append(session, current, item)
                .await
                .map_err(|retry_error| {
                    anyhow!(
                        "terminal append retry failed after {first_error}; retry: {retry_error}"
                    )
                }),
            other => other,
        };
        let result = match append {
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
            anyhow::bail!("append failed for phase {phase}: {reason}");
        }
        *sequence += 1;
        Ok(())
    }

    async fn next_sequence(&self, session: &SessionId) -> Result<u64> {
        Ok(self
            .session
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
        output: "Cancelled by user. The cancelled turn objective is closed.".into(),
        stop: TurnStop::Cancelled,
        failure: None,
        usage: Default::default(),
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
    fn cancelled_result_is_typed_and_never_empty() {
        let result = cancelled_result();
        assert_eq!(result.stop, TurnStop::Cancelled);
        assert!(!result.output.trim().is_empty());
        assert!(!result.metrics.completed_normally);
    }

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
