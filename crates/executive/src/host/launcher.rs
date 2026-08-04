//! Application host launch use cases. The binary selects a mode and delegates here.

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fabric::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, NoopTurnEventSink, OperationId,
    PermissionProfileId, PrincipalContext, PrincipalId, ThreadId, TurnRequest,
};
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
    pub json: bool,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct WorkspaceLaunch {
    pub cwd: Option<PathBuf>,
    pub add_dirs: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecHostOutcome {
    pub success: bool,
    pub rendered: String,
}

fn render_exec_json(operation_id: OperationId, result: &fabric::TurnResult) -> serde_json::Value {
    serde_json::json!({
        "success": result.metrics.completed_normally,
        "operation_id": operation_id.0,
        "response": result.output,
        "stop": match &result.stop {
            fabric::TurnStop::Completed => "completed",
            fabric::TurnStop::Blocked => "blocked",
            fabric::TurnStop::Cancelled => "cancelled",
            fabric::TurnStop::Failed => "failed",
        },
        "iterations": result.metrics.iterations,
        "tool_calls_made": result.metrics.tool_calls_made,
        "tool_errors": result.metrics.tool_errors,
        "provider_retries": result.metrics.provider_retries,
        "elapsed_ms": result.metrics.elapsed_ms,
    })
}

pub async fn run_exec(request: ExecLaunch) -> Result<ExecHostOutcome> {
    let process_cwd = std::env::current_dir()
        .map_err(|source| anyhow::anyhow!("cannot resolve process cwd: {source}"))?;
    let profile = if request.sandbox == "danger-full-access" {
        PermissionProfileId::danger_full_access()
    } else {
        PermissionProfileId::workspace_write()
    };
    let workspace =
        fabric::WorkspaceSelection::new(request.workspace.cwd, request.workspace.add_dirs)
            .resolve_with_profile(&process_cwd, &profile)?;
    let working_dir = workspace.cwd().to_path_buf();
    let mut builder = ExecSessionBuilder::new(working_dir.clone())
        .with_model(request.model.clone())
        .with_max_turns(request.max_turns)
        .with_sandbox(request.sandbox)
        .with_inference(Arc::new(CoreRpcClient::new(
            std::env::var_os("ALETHEON_CORE_SOCKET")
                .map(PathBuf::from)
                .unwrap_or_else(|| PathBuf::from("/run/aletheon/core.sock")),
        )));
    if let Some(path) = request.config {
        builder = builder.with_config(path);
    }
    let (turn_service, _, _, process_id) = builder.build().await?;
    let operation_id = OperationId::new();
    let result = turn_service
        .submit(
            TurnRequest {
                operation_id,
                process_id,
                context: {
                    let thread_id = uuid::Uuid::new_v4().to_string();
                    let uid = nix::unistd::Uid::effective().as_raw();
                    PrincipalContext::new(
                        PrincipalId::local_uid(uid),
                        LocalOsPrincipal {
                            uid,
                            gid: nix::unistd::Gid::effective().as_raw(),
                        },
                        ConnectionId::new(),
                        ThreadId(thread_id),
                        workspace.clone(),
                        PermissionProfileId::workspace_write(),
                        ApprovalPolicy::OnRequest,
                    )
                },
                input: request.prompt,
                model_policy: (!request.model.is_empty()).then_some(request.model),
                deadline: None,
                requirements: Vec::new(),
                requested_task_kind: None,
                evaluation_contract: None,
            },
            &NoopTurnEventSink,
        )
        .await?;
    let success = result.metrics.completed_normally;
    info!(
        iterations = result.metrics.iterations,
        tool_calls = result.metrics.tool_calls_made,
        tool_errors = result.metrics.tool_errors,
        provider_retries = result.metrics.provider_retries,
        success,
        "Execution complete"
    );
    let rendered = if request.json {
        serde_json::to_string_pretty(&render_exec_json(operation_id, &result))?
    } else {
        result.output
    };
    Ok(ExecHostOutcome { success, rendered })
}

#[cfg(test)]
mod tests {
    use super::{render_exec_json, select_daemon_socket, EnsureUserDaemon};

    #[test]
    fn user_daemon_startup_budget_allows_durable_state_restore() {
        assert_eq!(
            EnsureUserDaemon::default().startup_timeout,
            std::time::Duration::from_secs(30)
        );
    }

    #[test]
    fn exec_json_preserves_authoritative_stop_and_separate_metrics() {
        let result = fabric::TurnResult {
            output: "waiting for approval".into(),
            stop: fabric::TurnStop::Blocked,
            metrics: fabric::TurnMetrics {
                iterations: 2,
                tool_calls_made: 1,
                tool_errors: 0,
                provider_retries: 3,
                elapsed_ms: 40,
                completed_normally: false,
            },
        };

        let value = render_exec_json(fabric::OperationId::new(), &result);

        assert_eq!(value["stop"], "blocked");
        assert_eq!(value["provider_retries"], 3);
        assert_eq!(value["tool_calls_made"], 1);
        assert!(value.get("inference_rounds").is_none());
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
