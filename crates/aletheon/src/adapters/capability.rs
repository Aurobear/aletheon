use std::sync::Arc;

use async_trait::async_trait;
use kernel::capability::governed::{CapabilityExecutionContext, TurnCapabilityInvoker};
use runtime::turn_tool_projection::{project_initial_tools, AuthorizedToolCatalog};

pub struct PreparedCapabilities {
    pub definitions: Vec<contracts::ToolDefinition>,
    pub invoker: Arc<dyn TurnCapabilityInvoker>,
}

#[async_trait]
pub trait GovernedTurnCapabilityPort: Send + Sync {
    async fn prepare(
        &self,
        context: CapabilityExecutionContext,
        profile: application::turn::settings::ResolvedTurnProfile,
    ) -> anyhow::Result<PreparedCapabilities>;
}

pub struct ProjectedTurnCapabilities {
    pub authorized_catalog: Arc<AuthorizedToolCatalog>,
    pub definitions: Vec<contracts::ToolDefinition>,
    pub invoker: Arc<dyn TurnCapabilityInvoker>,
}

#[derive(Clone)]
pub struct TurnToolExecutor {
    pub invoker: Arc<dyn TurnCapabilityInvoker>,
    pub activation_catalog: Arc<AuthorizedToolCatalog>,
    pub diff_tracker: Arc<tokio::sync::Mutex<runtime::turn_diff_tracker::TurnDiffTracker>>,
    pub session_input: Arc<application::session_input::SessionInputCoordinator>,
    pub principal: contracts::PrincipalId,
    pub connection: contracts::ConnectionId,
    pub thread: contracts::ThreadId,
    pub operation_id: contracts::OperationId,
    pub process_id: contracts::ProcessId,
}

impl TurnToolExecutor {
    pub async fn execute(
        &self,
        tool_id: String,
        name: String,
        input: serde_json::Value,
    ) -> cognit::harness::event_sink::ToolResultEvent {
        let result = self
            .invoker
            .invoke(contracts::CapabilityCall {
                operation_id: self.operation_id,
                process_id: self.process_id,
                name: name.clone(),
                input,
                call_id: tool_id.clone(),
                deadline: None,
            })
            .await;
        if !result.is_error {
            let mut tracker = self.diff_tracker.lock().await;
            let changed = result.patch_delta.as_ref().is_some_and(|delta| {
                tracker.record_patch_delta(delta);
                !delta.files_changed.is_empty()
            });
            let injection = changed.then(|| tracker.to_context_injection());
            drop(tracker);
            if let Some(injection) = injection.filter(|value| !value.is_empty()) {
                if let Err(error) = self
                    .session_input
                    .enqueue(
                        self.principal.clone(),
                        self.connection.clone(),
                        self.thread.clone(),
                        runtime::PromptKind::Interjection,
                        injection,
                        format!("turn-diff:{tool_id}"),
                    )
                    .await
                {
                    tracing::warn!(%error, call_id = %tool_id, "failed to inject turn file delta");
                }
            }
        }
        let activated_tool_definitions = if name == "tool_search" && !result.is_error {
            self.activation_catalog
                .resolve_search_output(&result.output)
        } else {
            Vec::new()
        };
        cognit::harness::event_sink::ToolResultEvent {
            content: result.output,
            is_error: result.is_error,
            execution_time_ms: result.usage.wall_time_ms,
            patch_delta: result.patch_delta,
            activated_tool_definitions,
        }
    }
}

pub struct TurnCapabilityInput<'a> {
    pub main_agent_id: Option<contracts::AgentId>,
    pub process_id: contracts::ProcessId,
    pub operation_id: contracts::OperationId,
    pub principal: &'a contracts::PrincipalId,
    pub request: &'a contracts::TurnRequest,
    pub session_id: &'a str,
    pub working_dir: &'a std::path::Path,
    pub sandbox: contracts::SandboxRequirement,
    pub cancel: tokio_util::sync::CancellationToken,
    pub turn_count: usize,
    pub action_loop: Option<Arc<dyn kernel::capability::governed::GovernedActionLoop>>,
    pub streaming_tools: bool,
    pub turn_event_sender: contracts::ipc::TurnEventSender,
    pub profile: application::turn::settings::ResolvedTurnProfile,
    pub settings: &'a application::turn::settings::TurnRuntimeSettings,
}

pub async fn prepare_turn(
    capabilities: &dyn GovernedTurnCapabilityPort,
    input: TurnCapabilityInput<'_>,
) -> anyhow::Result<ProjectedTurnCapabilities> {
    let turn_id = input
        .request
        .context
        .turn_id
        .ok_or_else(|| anyhow::anyhow!("turn capability authority has no TurnId"))?;
    let delegation = input.main_agent_id.map(|_| {
        application::turn::command::main_delegation_authority(
            input.request.context.workspace.clone(),
            &input.profile,
            input.settings,
        )
    });
    let context = CapabilityExecutionContext {
        agent: input
            .main_agent_id
            .map(|agent_id| contracts::AgentToolContext {
                caller_root_agent_id: agent_id,
                parent_agent_id: agent_id,
                parent_process_id: input.process_id,
                delegator_authority: delegation,
            }),
        process_id: input.process_id,
        operation_id: input.operation_id,
        principal: input.principal.clone(),
        connection_id: input.request.context.connection_id.clone(),
        thread_id: input.request.context.thread_id.clone(),
        turn_id,
        execution_target: input.request.execution_target.clone(),
        workspace: input.request.context.workspace.clone(),
        permission_mode: if input
            .request
            .context
            .permission_profile
            .permits_filesystem_root()
        {
            contracts::permission::HostPermissionMode::Full
        } else {
            contracts::permission::HostPermissionMode::Safe
        },
        session_id: input.session_id.to_string(),
        working_dir: input.working_dir.to_path_buf(),
        sandbox: input.sandbox,
        cancel: input.cancel,
        turn_count: input.turn_count,
        repo_hooks_trusted: input.request.context.repo_hooks_trusted,
        action_loop: input.action_loop,
        streaming_tools: input.streaming_tools,
        turn_event_sender: Some(input.turn_event_sender),
    };
    prepare_and_project(
        capabilities,
        context,
        input.profile,
        input.request.requested_task_kind,
        &input.request.requirements,
    )
    .await
}

pub async fn prepare_and_project(
    capabilities: &dyn GovernedTurnCapabilityPort,
    context: CapabilityExecutionContext,
    profile: application::turn::settings::ResolvedTurnProfile,
    requested_task_kind: Option<contracts::TaskKind>,
    requirements: &[contracts::TurnRequirement],
) -> anyhow::Result<ProjectedTurnCapabilities> {
    let prepared = capabilities.prepare(context, profile).await?;
    let authorized_catalog = Arc::new(AuthorizedToolCatalog::new(&prepared.definitions));
    let projected = project_initial_tools(&prepared.definitions, requested_task_kind, requirements);
    tracing::info!(
        authorized_tool_count = projected.authorized_count,
        projected_tool_count = projected.definitions.len(),
        omitted_tool_count = projected.omitted_count(),
        "projected initial model-visible tool catalog"
    );
    Ok(ProjectedTurnCapabilities {
        authorized_catalog,
        definitions: projected.definitions,
        invoker: prepared.invoker,
    })
}
