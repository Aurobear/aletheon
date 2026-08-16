//! Binary-owned Exec host wiring.
//!
//! This is the CGP-08 Exec cutover: lifecycle, idempotency, terminal event
//! projection, and Core RPC client selection are owned by the `aletheon`
//! binary. Domain/session contracts remain in their owner crates and Runtime.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use crate::wiring::composition::RuntimeIdentityBinder;
use crate::wiring::exec_session::ExecSessionBuilder;
use ::contracts::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, OperationId, PermissionProfileId,
    PrincipalContext, PrincipalId, ThreadId, TurnEvent, TurnEventSink, TurnMetrics, TurnRequest,
};
use anyhow::Result;
use gateway::protocol::exec::{ExecEvent, ExecEventEnvelope, ExecTerminalKind};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::info;

#[derive(Debug, Clone)]
pub struct ExecLaunch {
    pub prompt: String,
    pub model: String,
    pub max_turns: usize,
    pub sandbox: String,
    pub workspace: WorkspaceLaunch,
    pub config: Option<PathBuf>,
    pub idempotency_key: Option<String>,
    pub timeout: Option<Duration>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceLaunch {
    pub cwd: Option<PathBuf>,
    pub add_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecHostOutcome {
    pub success: bool,
    pub exit_code: u8,
    pub terminal: ExecEventEnvelope,
    pub replayed: bool,
}

#[async_trait::async_trait]
pub trait ExecEventWriter: Send + Sync {
    async fn write(&self, event: &ExecEventEnvelope) -> anyhow::Result<()>;
}

pub struct JsonlExecEventWriter {
    output: Mutex<tokio::io::Stdout>,
}

impl Default for JsonlExecEventWriter {
    fn default() -> Self {
        Self {
            output: Mutex::new(tokio::io::stdout()),
        }
    }
}

#[async_trait::async_trait]
impl ExecEventWriter for JsonlExecEventWriter {
    async fn write(&self, event: &ExecEventEnvelope) -> anyhow::Result<()> {
        let mut encoded = serde_json::to_vec(event)?;
        encoded.push(b'\n');
        let mut output = self.output.lock().await;
        output.write_all(&encoded).await?;
        output.flush().await?;
        Ok(())
    }
}

#[derive(Default)]
struct CollectingExecEventWriter {
    events: Mutex<Vec<ExecEventEnvelope>>,
}

#[async_trait::async_trait]
impl ExecEventWriter for CollectingExecEventWriter {
    async fn write(&self, event: &ExecEventEnvelope) -> anyhow::Result<()> {
        self.events.lock().await.push(event.clone());
        Ok(())
    }
}

#[derive(Clone)]
struct ExecEventIdentity {
    session_id: String,
    task_id: String,
    turn_id: String,
    operation_id: OperationId,
}

struct ExecTurnEventWriter {
    writer: Arc<dyn ExecEventWriter>,
    identity: std::sync::Mutex<ExecEventIdentity>,
    next_sequence: AtomicU64,
    output_failed: AtomicBool,
}

impl ExecTurnEventWriter {
    fn new(writer: Arc<dyn ExecEventWriter>, identity: ExecEventIdentity) -> Self {
        Self {
            writer,
            identity: std::sync::Mutex::new(identity),
            next_sequence: AtomicU64::new(1),
            output_failed: AtomicBool::new(false),
        }
    }

    fn envelope(&self, activity_id: Option<String>, event: ExecEvent) -> ExecEventEnvelope {
        let identity = self.identity.lock().unwrap().clone();
        ExecEventEnvelope::v1(
            self.next_sequence.fetch_add(1, Ordering::SeqCst),
            identity.session_id,
            identity.task_id,
            identity.turn_id,
            activity_id,
            identity.operation_id,
            event,
        )
    }

    async fn emit_bounded(&self, event: ExecEventEnvelope) {
        let delivered =
            tokio::time::timeout(Duration::from_millis(250), self.writer.write(&event)).await;
        if !matches!(delivered, Ok(Ok(()))) {
            self.output_failed.store(true, Ordering::SeqCst);
        }
    }
}

impl RuntimeIdentityBinder for ExecTurnEventWriter {
    fn bind_runtime_identity(&self, operation_id: OperationId, turn_id: &::contracts::TurnId) {
        let mut identity = self.identity.lock().unwrap();
        identity.operation_id = operation_id;
        identity.turn_id = turn_id.0.to_string();
    }
}

#[async_trait::async_trait]
impl TurnEventSink for ExecTurnEventWriter {
    async fn emit(&self, event: TurnEvent) {
        if self.output_failed.load(Ordering::SeqCst) {
            return;
        }
        let envelope = match event {
            TurnEvent::Started { .. } => self.envelope(None, ExecEvent::Started),
            TurnEvent::ToolCall { name, .. } => {
                let activity_id = uuid::Uuid::new_v4().to_string();
                self.envelope(Some(activity_id), ExecEvent::ActivityStarted { name })
            }
            TurnEvent::Finished { .. }
            | TurnEvent::EmbodimentProgress { .. }
            | TurnEvent::RobotEpisodeSettled { .. } => return,
        };
        self.emit_bounded(envelope).await;
    }
}

fn terminal_envelope(
    sink: &ExecTurnEventWriter,
    status: ExecTerminalKind,
    output: String,
    metrics: TurnMetrics,
    error_code: Option<String>,
) -> ExecEventEnvelope {
    sink.envelope(
        None,
        ExecEvent::Terminal {
            status,
            output,
            metrics,
            error_code,
        },
    )
}

fn terminal_status(event: &ExecEventEnvelope) -> ExecTerminalKind {
    match event.event {
        ExecEvent::Terminal { status, .. } => status,
        _ => unreachable!("exec outcome must contain a terminal event"),
    }
}

async fn emit_terminal_outcome(
    sink: &ExecTurnEventWriter,
    terminal: ExecEventEnvelope,
    replayed: bool,
) -> ExecHostOutcome {
    sink.emit_bounded(terminal.clone()).await;
    let status = terminal_status(&terminal);
    ExecHostOutcome {
        success: status == ExecTerminalKind::Completed,
        exit_code: status.exit_code(),
        terminal,
        replayed,
    }
}

fn classify_exec_error(error: &anyhow::Error) -> (ExecTerminalKind, &'static str) {
    if let Some(failure) = error.downcast_ref::<cognit::inference::InferenceFailure>() {
        return match failure.code {
            "provider_unavailable" => (
                ExecTerminalKind::ProviderUnavailable,
                "provider_unavailable",
            ),
            "provider_rejected_request" => (
                ExecTerminalKind::ProviderRejected,
                "provider_rejected_request",
            ),
            _ => (ExecTerminalKind::Failed, failure.code),
        };
    }
    (ExecTerminalKind::Failed, "execution_failed")
}

enum ExecExecutionResult {
    Completed(
        ::contracts::TurnResult,
        Arc<crate::wiring::exec_session::ExecSessionFacts>,
    ),
    Failed(anyhow::Error),
    DeadlineExceeded,
    Interrupted,
    CancellationUnconfirmed,
}

fn exec_request_digest(request: &ExecLaunch, workspace: &std::path::Path) -> String {
    let canonical = serde_json::json!({
        "prompt": request.prompt,
        "model": request.model,
        "max_turns": request.max_turns,
        "sandbox": request.sandbox,
        "workspace": workspace,
        "config": request.config,
        "timeout_ms": request.timeout.map(|value| value.as_millis()),
    });
    format!("{:x}", Sha256::digest(canonical.to_string().as_bytes()))
}

pub async fn run_exec(request: ExecLaunch) -> Result<ExecHostOutcome> {
    let writer = Arc::new(CollectingExecEventWriter::default());
    run_exec_streaming(request, writer).await
}

pub async fn run_exec_streaming(
    request: ExecLaunch,
    writer: Arc<dyn ExecEventWriter>,
) -> Result<ExecHostOutcome> {
    let uid = nix::unistd::Uid::effective().as_raw();
    let principal_id = PrincipalId::local_uid(uid);
    let identity = ExecEventIdentity {
        session_id: uuid::Uuid::new_v4().to_string(),
        task_id: uuid::Uuid::new_v4().to_string(),
        // Runtime admission binds the canonical turn before any events are
        // emitted.  Keep the pre-admission envelope explicitly non-canonical.
        turn_id: "runtime-pending-turn".into(),
        operation_id: OperationId::new(),
    };
    let event_sink = Arc::new(ExecTurnEventWriter::new(writer, identity.clone()));
    let process_cwd = match std::env::current_dir() {
        Ok(path) => path,
        Err(source) => {
            let terminal = terminal_envelope(
                &event_sink,
                ExecTerminalKind::Failed,
                format!("cannot resolve process cwd: {source}"),
                TurnMetrics::default(),
                Some("workspace_resolution_failed".into()),
            );
            return Ok(emit_terminal_outcome(&event_sink, terminal, false).await);
        }
    };
    let profile = if request.sandbox == "danger-full-access" {
        PermissionProfileId::danger_full_access()
    } else {
        PermissionProfileId::workspace_write()
    };
    let workspace = match ::contracts::WorkspaceSelection::new(
        request.workspace.cwd.clone(),
        request.workspace.add_dirs.clone(),
    )
    .resolve_with_profile(&process_cwd, &profile)
    {
        Ok(workspace) => workspace,
        Err(error) => {
            let terminal = terminal_envelope(
                &event_sink,
                ExecTerminalKind::Failed,
                error.to_string(),
                TurnMetrics::default(),
                Some("workspace_resolution_failed".into()),
            );
            return Ok(emit_terminal_outcome(&event_sink, terminal, false).await);
        }
    };
    let working_dir = workspace.cwd().to_path_buf();
    let request_digest = exec_request_digest(&request, &working_dir);
    let user_paths = match ::contracts::paths::UserRuntimePaths::resolve(
        &::contracts::paths::ProcessRuntimeEnvironment,
    ) {
        Ok(paths) => paths,
        Err(error) => {
            let terminal = terminal_envelope(
                &event_sink,
                ExecTerminalKind::Failed,
                error.to_string(),
                TurnMetrics::default(),
                Some("runtime_path_resolution_failed".into()),
            );
            return Ok(emit_terminal_outcome(&event_sink, terminal, false).await);
        }
    };
    if let Err(error) = user_paths.prepare() {
        let terminal = terminal_envelope(
            &event_sink,
            ExecTerminalKind::Failed,
            error.to_string(),
            TurnMetrics::default(),
            Some("runtime_path_prepare_failed".into()),
        );
        return Ok(emit_terminal_outcome(&event_sink, terminal, false).await);
    }
    let idempotency_store = match adapters_sqlite::exec_idempotency::ExecIdempotencyStore::open(
        &user_paths.state_root.join("exec-idempotency-v1.db"),
    ) {
        Ok(store) => store,
        Err(error) => {
            let terminal = terminal_envelope(
                &event_sink,
                ExecTerminalKind::Failed,
                error.to_string(),
                TurnMetrics::default(),
                Some("idempotency_store_failed".into()),
            );
            return Ok(emit_terminal_outcome(&event_sink, terminal, false).await);
        }
    };
    if let Some(key) = request.idempotency_key.as_deref() {
        match idempotency_store
            .claim(&principal_id.0, key, &request_digest)
            .await
        {
            Ok(adapters_sqlite::exec_idempotency::ExecClaim::Acquired) => {}
            Ok(adapters_sqlite::exec_idempotency::ExecClaim::Replay(terminal)) => {
                return Ok(emit_terminal_outcome(&event_sink, *terminal, true).await);
            }
            Ok(adapters_sqlite::exec_idempotency::ExecClaim::InProgress) => {
                let terminal = terminal_envelope(
                    &event_sink,
                    ExecTerminalKind::Blocked,
                    "the idempotent execution has no authoritative terminal receipt".into(),
                    TurnMetrics::default(),
                    Some("idempotency_in_progress".into()),
                );
                return Ok(emit_terminal_outcome(&event_sink, terminal, true).await);
            }
            Err(error) => {
                let code = match error {
                    adapters_sqlite::exec_idempotency::ExecIdempotencyError::RequestConflict => {
                        "idempotency_conflict"
                    }
                    adapters_sqlite::exec_idempotency::ExecIdempotencyError::Store(_) => {
                        "idempotency_store_failed"
                    }
                };
                let terminal = terminal_envelope(
                    &event_sink,
                    ExecTerminalKind::Failed,
                    error.to_string(),
                    TurnMetrics::default(),
                    Some(code.into()),
                );
                return Ok(emit_terminal_outcome(&event_sink, terminal, false).await);
            }
        }
    }
    let cancellation = CancellationToken::new();
    let mut builder = ExecSessionBuilder::new(working_dir.clone())
        .with_model(request.model.clone())
        .with_max_turns(request.max_turns)
        .with_sandbox(request.sandbox.clone())
        .with_cancellation(cancellation.clone())
        .with_inference(Arc::new(crate::wiring::core_rpc::CoreRpcClient::new(
            std::env::var_os("ALETHEON_CORE_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/run/aletheon/core.sock")),
        )));
    if let Some(path) = request.config {
        builder = builder.with_config(path);
    }
    let (turn_service, _, _, process_id, facts) = builder.build().await?;
    let execution = async {
        let result = turn_service
            .submit(
                TurnRequest {
                    operation_id: identity.operation_id,
                    process_id,
                    context: {
                        PrincipalContext::new(
                            principal_id.clone(),
                            LocalOsPrincipal {
                                uid,
                                gid: nix::unistd::Gid::effective().as_raw(),
                            },
                            ConnectionId::new(),
                            ThreadId(identity.session_id.clone()),
                            workspace.clone(),
                            profile.clone(),
                            ApprovalPolicy::Never,
                        )
                    },
                    input: request.prompt.clone(),
                    execution_target: ::contracts::ExecutionTargetSelection::default(),
                    model_policy: (!request.model.is_empty()).then_some(request.model.clone()),
                    // Give the TurnCoordinator the same relative deadline as
                    // the CLI. Runtime owns the terminal fence, so timeout
                    // must enter its authoritative abort/settlement protocol
                    // instead of existing only as a host-side future drop.
                    deadline: request.timeout.map(|duration| {
                        ::contracts::MonoDeadlineMillis(duration.as_millis() as u64)
                    }),
                    requirements: Vec::new(),
                    requested_task_kind: None,
                    evaluation_contract: None,
                },
                event_sink.as_ref(),
            )
            .await?;
        Ok::<_, anyhow::Error>((result, facts))
    };
    tokio::pin!(execution);
    let execution_result = if let Some(timeout) = request.timeout {
        tokio::select! {
            // If the Runtime deadline and the host timer become ready in the
            // same poll, prefer the settled Runtime result.
            biased;
            result = &mut execution => match result {
                Ok((result, facts)) => ExecExecutionResult::Completed(result, facts),
                Err(error) => ExecExecutionResult::Failed(error),
            },
            _ = tokio::time::sleep(timeout) => {
                cancellation.cancel();
                turn_service.cancel_active_for_principal(&principal_id).await;
                if tokio::time::timeout(Duration::from_secs(10), &mut execution).await.is_ok() {
                    ExecExecutionResult::DeadlineExceeded
                } else {
                    ExecExecutionResult::CancellationUnconfirmed
                }
            },
            signal = tokio::signal::ctrl_c() => {
                signal?;
                cancellation.cancel();
                turn_service.cancel_active_for_principal(&principal_id).await;
                if tokio::time::timeout(Duration::from_secs(10), &mut execution).await.is_ok() {
                    ExecExecutionResult::Interrupted
                } else {
                    ExecExecutionResult::CancellationUnconfirmed
                }
            }
        }
    } else {
        tokio::select! {
            biased;
            result = &mut execution => match result {
                Ok((result, facts)) => ExecExecutionResult::Completed(result, facts),
                Err(error) => ExecExecutionResult::Failed(error),
            },
            signal = tokio::signal::ctrl_c() => {
                signal?;
                cancellation.cancel();
                turn_service.cancel_active_for_principal(&principal_id).await;
                if tokio::time::timeout(Duration::from_secs(10), &mut execution).await.is_ok() {
                    ExecExecutionResult::Interrupted
                } else {
                    ExecExecutionResult::CancellationUnconfirmed
                }
            }
        }
    };
    let terminal = match execution_result {
        ExecExecutionResult::Completed(result, facts) => {
            let mut status = ExecTerminalKind::from(result.stop.clone());
            let mut error_code = None;
            if facts.approval_unavailable() {
                status = ExecTerminalKind::Blocked;
                error_code = Some("approval_unavailable".into());
            }
            if event_sink.output_failed.load(Ordering::SeqCst) {
                status = ExecTerminalKind::OutputBackpressure;
                error_code = Some("output_backpressure".into());
            }
            info!(
                iterations = result.metrics.iterations,
                tool_calls = result.metrics.tool_calls_made,
                tool_errors = result.metrics.tool_errors,
                provider_retries = result.metrics.provider_retries,
                status = ?status,
                "Execution complete"
            );
            terminal_envelope(
                &event_sink,
                status,
                result.output,
                result.metrics,
                error_code,
            )
        }
        ExecExecutionResult::Failed(error) => {
            let (status, code) = classify_exec_error(&error);
            terminal_envelope(
                &event_sink,
                status,
                error.to_string(),
                TurnMetrics::default(),
                Some(code.into()),
            )
        }
        ExecExecutionResult::DeadlineExceeded => terminal_envelope(
            &event_sink,
            ExecTerminalKind::Cancelled,
            "execution deadline exceeded".into(),
            TurnMetrics::default(),
            Some("deadline_exceeded".into()),
        ),
        ExecExecutionResult::Interrupted => terminal_envelope(
            &event_sink,
            ExecTerminalKind::Cancelled,
            "execution interrupted".into(),
            TurnMetrics::default(),
            Some("interrupted".into()),
        ),
        ExecExecutionResult::CancellationUnconfirmed => terminal_envelope(
            &event_sink,
            ExecTerminalKind::Failed,
            "cancellation was requested but no authoritative terminal was observed".into(),
            TurnMetrics::default(),
            Some("cancellation_unconfirmed".into()),
        ),
    };
    if let Some(key) = request.idempotency_key.as_deref() {
        if let Err(error) = idempotency_store
            .complete(&principal_id.0, key, &request_digest, &terminal)
            .await
        {
            let failed = terminal_envelope(
                &event_sink,
                ExecTerminalKind::Failed,
                error.to_string(),
                TurnMetrics::default(),
                Some("idempotency_receipt_failed".into()),
            );
            return Ok(emit_terminal_outcome(&event_sink, failed, false).await);
        }
    }
    Ok(emit_terminal_outcome(&event_sink, terminal, false).await)
}
