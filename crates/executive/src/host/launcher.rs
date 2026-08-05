//! Application host launch use cases. The binary selects a mode and delegates here.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fabric::types::exec::{ExecEvent, ExecEventEnvelope, ExecTerminalKind};
use fabric::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, OperationId, PermissionProfileId,
    PrincipalContext, PrincipalId, ThreadId, TurnEvent, TurnEventSink, TurnMetrics, TurnRequest,
};
use sha2::{Digest, Sha256};
use tokio::io::AsyncWriteExt;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;
use tracing::info;

use crate::composition::user_runtime::{UserRuntime, UserRuntimeConfig};
use crate::core::SystemCoreRuntime;
use crate::host::core_rpc::CoreRpcClient;
use crate::host::readiness::{
    acquire_daemon_authority, detect_install_mode, FileStartupLock, ProcessDaemonLifecycleBackend,
};
use crate::ExecSessionBuilder;

#[derive(Debug, Clone)]
pub struct CoreLaunch {
    pub config: Option<PathBuf>,
    pub socket: PathBuf,
}

pub async fn run_core(request: CoreLaunch) -> Result<()> {
    let runtime = SystemCoreRuntime::bootstrap(request.config.as_deref(), request.socket).await?;
    info!(
        socket = %runtime.socket_path().display(),
        providers = runtime.provider_count(),
        "System inference core started"
    );
    runtime.run().await
}

#[derive(Debug, Clone)]
pub struct DaemonLaunch {
    pub config: Option<PathBuf>,
    pub env: Option<PathBuf>,
    pub command_socket: Option<PathBuf>,
    pub parent_socket: Option<PathBuf>,
    pub container: Option<String>,
    pub image: String,
    pub enable_evolution: bool,
    /// Additively enable the isolated execd backend.
    pub enable_execd: bool,
}

fn select_daemon_socket(
    command: Option<PathBuf>,
    parent: Option<PathBuf>,
    environment: Option<PathBuf>,
    default: PathBuf,
) -> PathBuf {
    command.or(parent).or(environment).unwrap_or(default)
}

pub async fn run_daemon(request: DaemonLaunch) -> Result<()> {
    if let Some(env_path) = request.env.as_ref() {
        super::load_dotenv(env_path);
    }
    if request.container.is_some() {
        tracing::warn!(
            image = %request.image,
            "container host selection is ignored by the per-user runtime boundary"
        );
    }
    let paths =
        fabric::paths::UserRuntimePaths::resolve(&fabric::paths::ProcessRuntimeEnvironment)?;
    paths.prepare()?;
    let _authority = acquire_daemon_authority(&paths.runtime_root.join("daemon-authority.lock"))?;
    let socket = select_daemon_socket(
        request.command_socket,
        request.parent_socket,
        std::env::var_os("ALETHEON_SOCKET")
            .filter(|value| !value.is_empty())
            .map(PathBuf::from),
        paths.socket_path(),
    );
    let config = UserRuntimeConfig::load(
        request.config.as_deref(),
        paths,
        socket,
        request.enable_evolution,
        request.enable_execd,
    )?;
    let core_socket = std::env::var_os("ALETHEON_CORE_SOCKET")
        .map(PathBuf::from)
        .unwrap_or_else(|| PathBuf::from("/run/aletheon/core.sock"));
    let inference = Arc::new(CoreRpcClient::new(core_socket));
    UserRuntime::bootstrap(config, inference).await?.run().await
}

#[derive(Debug, Clone)]
pub struct EnsureUserDaemon {
    pub socket: Option<PathBuf>,
    pub startup_timeout: Duration,
}

impl Default for EnsureUserDaemon {
    fn default() -> Self {
        Self {
            socket: None,
            startup_timeout: Duration::from_secs(30),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum EnsureUserDaemonError {
    #[error(transparent)]
    Paths(#[from] fabric::paths::UserPathError),
    #[error("cannot resolve the current Aletheon executable: {0}")]
    CurrentExecutable(#[source] std::io::Error),
    #[error(transparent)]
    Lifecycle(#[from] crate::application::daemon_lifecycle::DaemonLifecycleError),
}

impl EnsureUserDaemonError {
    /// Stable machine-readable category projected by user-facing adapters.
    pub fn diagnostic_code(&self) -> &'static str {
        use crate::application::daemon_lifecycle::DaemonLifecycleError;

        match self {
            Self::Paths(_) => "runtime_path_resolution_failed",
            Self::CurrentExecutable(_) => "current_executable_resolution_failed",
            Self::Lifecycle(error) => match error {
                DaemonLifecycleError::Lock(_) => "startup_lock_failed",
                DaemonLifecycleError::LockTimeout { .. } => "startup_lock_timeout",
                DaemonLifecycleError::Probe(_) => "readiness_probe_failed",
                DaemonLifecycleError::StaleSocketRecovery(_) => "stale_socket_recovery_failed",
                DaemonLifecycleError::Activation(_) => "daemon_activation_failed",
                DaemonLifecycleError::BootstrapFailed(_) => "daemon_bootstrap_failed",
                DaemonLifecycleError::ProtocolMismatch { .. } => "protocol_version_mismatch",
                DaemonLifecycleError::RuntimeVersionMismatch { .. } => "runtime_version_mismatch",
                DaemonLifecycleError::ReadinessTimeout { .. } => "daemon_readiness_timeout",
            },
        }
    }
}

pub async fn ensure_user_daemon(
    request: EnsureUserDaemon,
) -> Result<crate::application::daemon_lifecycle::DaemonReadyReceipt, EnsureUserDaemonError> {
    let paths =
        fabric::paths::UserRuntimePaths::resolve(&fabric::paths::ProcessRuntimeEnvironment)?;
    paths.prepare()?;
    let socket = request.socket.unwrap_or_else(|| paths.socket_path());
    let executable = std::env::current_exe().map_err(EnsureUserDaemonError::CurrentExecutable)?;
    // Unit activation is valid only when the installed socket listens on the
    // exact requested endpoint. Isolated or custom endpoints use foreground
    // development activation instead of waiting on an unrelated unit.
    let mode = detect_install_mode(&socket).await;
    let backend = Arc::new(ProcessDaemonLifecycleBackend::new(executable, mode));
    let startup_lock = Arc::new(FileStartupLock::new(
        paths.runtime_root.join("daemon-startup.lock"),
    ));
    crate::application::daemon_lifecycle::DaemonLifecycleService::new(backend, startup_lock)
        .ensure_running(crate::application::daemon_lifecycle::EnsureDaemonRequest {
            socket,
            mode,
            startup_timeout: request.startup_timeout,
            poll_interval: Duration::from_millis(50),
            expected_protocol_version: fabric::CLIENT_PROTOCOL_VERSION,
            expected_runtime_version: Some(env!("CARGO_PKG_VERSION").into()),
        })
        .await
        .map_err(Into::into)
}

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
    identity: ExecEventIdentity,
    next_sequence: AtomicU64,
    output_failed: AtomicBool,
}

impl ExecTurnEventWriter {
    fn new(writer: Arc<dyn ExecEventWriter>, identity: ExecEventIdentity) -> Self {
        Self {
            writer,
            identity,
            next_sequence: AtomicU64::new(1),
            output_failed: AtomicBool::new(false),
        }
    }

    fn envelope(&self, activity_id: Option<String>, event: ExecEvent) -> ExecEventEnvelope {
        ExecEventEnvelope::v1(
            self.next_sequence.fetch_add(1, Ordering::SeqCst),
            self.identity.session_id.clone(),
            self.identity.task_id.clone(),
            self.identity.turn_id.clone(),
            activity_id,
            self.identity.operation_id,
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
            TurnEvent::Finished { .. } | TurnEvent::EmbodimentProgress { .. } => return,
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
        fabric::TurnResult,
        Arc<crate::composition::exec_session::ExecSessionFacts>,
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
        turn_id: fabric::TurnId::new().0.to_string(),
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
    let workspace = match fabric::WorkspaceSelection::new(
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
    let user_paths =
        match fabric::paths::UserRuntimePaths::resolve(&fabric::paths::ProcessRuntimeEnvironment) {
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
    let idempotency_store = match crate::application::exec::ExecIdempotencyStore::open(
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
            Ok(crate::application::exec::ExecClaim::Acquired) => {}
            Ok(crate::application::exec::ExecClaim::Replay(terminal)) => {
                return Ok(emit_terminal_outcome(&event_sink, *terminal, true).await);
            }
            Ok(crate::application::exec::ExecClaim::InProgress) => {
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
                    crate::application::exec::ExecIdempotencyError::RequestConflict => {
                        "idempotency_conflict"
                    }
                    crate::application::exec::ExecIdempotencyError::Store(_) => {
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
        .with_inference(Arc::new(CoreRpcClient::new(
            std::env::var_os("ALETHEON_CORE_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/run/aletheon/core.sock")),
        )));
    if let Some(path) = request.config {
        builder = builder.with_config(path);
    }
    let execution = async {
        let (turn_service, _, _, process_id, facts) = builder.build().await?;
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
                    model_policy: (!request.model.is_empty()).then_some(request.model.clone()),
                    deadline: None,
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
            result = &mut execution => match result {
                Ok((result, facts)) => ExecExecutionResult::Completed(result, facts),
                Err(error) => ExecExecutionResult::Failed(error),
            },
            _ = tokio::time::sleep(timeout) => {
                cancellation.cancel();
                if tokio::time::timeout(Duration::from_secs(5), &mut execution).await.is_ok() {
                    ExecExecutionResult::DeadlineExceeded
                } else {
                    ExecExecutionResult::CancellationUnconfirmed
                }
            },
            signal = tokio::signal::ctrl_c() => {
                signal?;
                cancellation.cancel();
                if tokio::time::timeout(Duration::from_secs(5), &mut execution).await.is_ok() {
                    ExecExecutionResult::Interrupted
                } else {
                    ExecExecutionResult::CancellationUnconfirmed
                }
            }
        }
    } else {
        tokio::select! {
            result = &mut execution => match result {
                Ok((result, facts)) => ExecExecutionResult::Completed(result, facts),
                Err(error) => ExecExecutionResult::Failed(error),
            },
            signal = tokio::signal::ctrl_c() => {
                signal?;
                cancellation.cancel();
                if tokio::time::timeout(Duration::from_secs(5), &mut execution).await.is_ok() {
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

#[cfg(test)]
mod tests {
    use super::*;

    struct SlowExecWriter;

    #[async_trait::async_trait]
    impl ExecEventWriter for SlowExecWriter {
        async fn write(&self, _event: &ExecEventEnvelope) -> anyhow::Result<()> {
            tokio::time::sleep(Duration::from_secs(1)).await;
            Ok(())
        }
    }

    #[test]
    fn user_daemon_startup_budget_allows_durable_state_restore() {
        assert_eq!(
            EnsureUserDaemon::default().startup_timeout,
            std::time::Duration::from_secs(30)
        );
    }

    #[tokio::test]
    async fn exec_output_backpressure_is_typed_instead_of_silently_dropped() {
        let identity = ExecEventIdentity {
            session_id: "session".into(),
            task_id: "task".into(),
            turn_id: "turn".into(),
            operation_id: OperationId::new(),
        };
        let sink = ExecTurnEventWriter::new(Arc::new(SlowExecWriter), identity);
        let started = std::time::Instant::now();
        sink.emit(TurnEvent::Started {
            operation_id: sink.identity.operation_id,
        })
        .await;
        assert!(started.elapsed() < Duration::from_secs(1));
        assert!(sink.output_failed.load(Ordering::SeqCst));

        let terminal = terminal_envelope(
            &sink,
            ExecTerminalKind::OutputBackpressure,
            "consumer did not drain output".into(),
            TurnMetrics::default(),
            Some("output_backpressure".into()),
        );
        assert_eq!(
            terminal_status(&terminal),
            ExecTerminalKind::OutputBackpressure
        );
        assert_eq!(terminal_status(&terminal).exit_code(), 25);
    }

    #[test]
    fn exec_provider_failures_keep_machine_readable_categories() {
        let unavailable = cognit::inference::InferenceFailure::transient("provider_unavailable");
        assert_eq!(
            classify_exec_error(&unavailable),
            (
                ExecTerminalKind::ProviderUnavailable,
                "provider_unavailable"
            )
        );

        let rejected = cognit::inference::InferenceFailure::terminal("provider_rejected_request");
        assert_eq!(
            classify_exec_error(&rejected),
            (
                ExecTerminalKind::ProviderRejected,
                "provider_rejected_request"
            )
        );
    }

    #[test]
    fn daemon_endpoint_precedence_is_command_parent_environment_default() {
        let path = |value: &str| Some(value.into());
        assert_eq!(
            select_daemon_socket(
                path("/command"),
                path("/parent"),
                path("/environment"),
                "/default".into(),
            ),
            std::path::PathBuf::from("/command")
        );
        assert_eq!(
            select_daemon_socket(
                None,
                path("/parent"),
                path("/environment"),
                "/default".into(),
            ),
            std::path::PathBuf::from("/parent")
        );
        assert_eq!(
            select_daemon_socket(None, None, path("/environment"), "/default".into()),
            std::path::PathBuf::from("/environment")
        );
        assert_eq!(
            select_daemon_socket(None, None, None, "/default".into()),
            std::path::PathBuf::from("/default")
        );
    }
}
