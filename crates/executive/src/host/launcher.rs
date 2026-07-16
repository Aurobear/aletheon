//! Application host launch use cases. The binary selects a mode and delegates here.

use std::path::PathBuf;
use std::sync::Arc;

use anyhow::Result;
use fabric::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, NoopTurnEventSink, OperationId,
    PermissionProfileId, PrincipalContext, PrincipalId, ThreadId, TurnRequest, WorkspacePolicy,
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
    pub socket: PathBuf,
    pub container: Option<String>,
    pub image: String,
    pub enable_evolution: bool,
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
    let config = UserRuntimeConfig::load(
        request.config.as_deref(),
        paths,
        request.socket,
        request.enable_evolution,
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
    pub workspace: WorkspacePolicy,
    pub config: Option<PathBuf>,
    pub json: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExecHostOutcome {
    pub success: bool,
    pub rendered: String,
}

pub async fn run_exec(request: ExecLaunch) -> Result<ExecHostOutcome> {
    let working_dir = request.workspace.cwd().to_path_buf();
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
    let result = turn_service
        .submit(
            TurnRequest {
                operation_id: OperationId::new(),
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
                        request.workspace.clone(),
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
            "success": success, "response": result.output, "iterations": result.metrics.iterations,
            "tool_calls_made": result.metrics.tool_calls_made, "tool_errors": result.metrics.tool_errors,
            "elapsed_ms": result.metrics.elapsed_ms,
        }))?
    } else {
        result.output
    };
    Ok(ExecHostOutcome { success, rendered })
}
