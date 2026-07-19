//! Application host launch use cases. The binary selects a mode and delegates here.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use fabric::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, NoopTurnEventSink, OperationId,
    PermissionProfileId, PrincipalContext, PrincipalId, ThreadId, TurnRequest,
};
use tracing::info;

use crate::core::SystemCoreRuntime;
use crate::r#impl::core_rpc::CoreRpcClient;
use crate::user_runtime::{UserRuntime, UserRuntimeConfig};
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
            },
            &NoopTurnEventSink,
        )
        .await?;
    let success = result.metrics.completed_normally;
    info!(
        iterations = result.metrics.iterations,
        tool_calls = result.metrics.tool_calls_made,
        tool_errors = result.metrics.tool_errors,
        success,
        "Execution complete"
    );
    let rendered = if request.json {
        serde_json::to_string_pretty(&serde_json::json!({
            "success": success, "operation_id": operation_id.0, "response": result.output, "iterations": result.metrics.iterations,
            "tool_calls_made": result.metrics.tool_calls_made, "tool_errors": result.metrics.tool_errors,
            "elapsed_ms": result.metrics.elapsed_ms,
        }))?
    } else {
        result.output
    };
    Ok(ExecHostOutcome { success, rendered })
}

#[cfg(test)]
mod tests {
    use super::select_daemon_socket;

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
