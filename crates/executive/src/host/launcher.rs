//! Application host launch use cases. The binary selects a mode and delegates here.

use std::path::PathBuf;
use std::sync::Arc;

use aletheon_kernel::chronos::SystemClock;
use anyhow::Result;
use fabric::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, NoopTurnEventSink, OperationId,
    PermissionProfileId, PrincipalContext, PrincipalId, ProcessId, ThreadId, TurnRequest,
    WorkspacePolicy,
};
use tracing::info;

use super::RuntimeHost;
use crate::ExecSessionBuilder;

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
    if let Some(runtime_name) = request.container {
        let mut host = super::container::ContainerHost::new(
            request.config,
            request.env,
            runtime_name,
            request.image,
            request.enable_evolution,
        );
        host.init().await?;
        return Box::new(host).serve().await;
    }
    if std::env::var("NOTIFY_SOCKET").is_ok() {
        let mut host = super::systemd::SystemdHost::new(
            request.config,
            request.env,
            request.socket,
            request.enable_evolution,
            Arc::new(SystemClock::new()),
        );
        host.init().await?;
        return Box::new(host).serve().await;
    }
    if std::env::var("CONTAINER").is_ok() || std::path::Path::new("/.dockerenv").exists() {
        let mut host = super::container::ContainerHost::new(
            request.config,
            request.env,
            "docker".into(),
            request.image,
            request.enable_evolution,
        );
        host.init().await?;
        return Box::new(host).serve().await;
    }
    let mut host = super::DaemonHost::new(
        request.config,
        request.env,
        request.socket,
        request.enable_evolution,
    );
    host.init().await?;
    Box::new(host).serve().await
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
        .with_sandbox(request.sandbox);
    if let Some(path) = request.config {
        builder = builder.with_config(path);
    }
    let (turn_service, _, _) = builder.build().await?;
    let result = turn_service
        .submit(
            TurnRequest {
                operation_id: OperationId::new(),
                process_id: ProcessId::new(),
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
