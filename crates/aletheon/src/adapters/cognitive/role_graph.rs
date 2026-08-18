//! Cognitive role-graph preparation adapter for one Turn.

use contracts::{AgoraSpaceId, ProcessId, TurnRequest};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

pub type PreparedRoleGraph = (
    ::agora::cognitive_role_workflow::CognitiveRoleWorkflow,
    ::agora::cognitive_role_workflow::CodingWorkflowRequest,
);

#[allow(clippy::too_many_arguments)]
pub async fn prepare_for_turn(
    agora: Option<&Arc<dyn agora::contract::AgoraService>>,
    kernel: &kernel::KernelRuntime,
    role_workflow_factory: Option<&Arc<::agora::cognitive_role_workflow::RoleWorkflowFactory>>,
    request: &TurnRequest,
    objective: &str,
    main_pid: ProcessId,
    cancellation: CancellationToken,
    profile: &application::turn::settings::ResolvedTurnProfile,
    config: &application::turn::settings::TurnRuntimeSettings,
) -> anyhow::Result<Option<PreparedRoleGraph>> {
    crate::adapters::conscious::turn_workspace::resume_single_pending_clarification(
        agora, request, objective,
    )
    .await?;
    let requested = application::turn::command::role_graph_requested(request, config);
    let root = if requested || request.evaluation_contract.is_some() {
        crate::adapters::conscious::turn_workspace::ensure_root_cognitive_task(
            agora, request, objective, main_pid,
        )
        .await?
    } else {
        None
    };
    if !requested {
        return Ok(None);
    }
    prepare_role_graph(
        kernel,
        role_workflow_factory,
        request,
        objective,
        main_pid,
        root,
        cancellation,
        profile,
        config,
    )
    .await
}

pub async fn prepare_role_graph(
    kernel: &kernel::KernelRuntime,
    role_workflow_factory: Option<&Arc<::agora::cognitive_role_workflow::RoleWorkflowFactory>>,
    request: &TurnRequest,
    objective: &str,
    main_pid: ProcessId,
    root: Option<(::contracts::cognitive_workflow::CognitiveTaskNodeId, u64)>,
    cancellation: CancellationToken,
    profile: &application::turn::settings::ResolvedTurnProfile,
    config: &application::turn::settings::TurnRuntimeSettings,
) -> anyhow::Result<Option<PreparedRoleGraph>> {
    use cognit::TaskDecompositionPolicy;
    let process = kernel.inspect_process(main_pid).await?;
    let budget = ::contracts::AgentBudget {
        max_input_tokens: profile.max_input_tokens,
        max_output_tokens: profile.max_output_tokens,
        max_tool_calls: profile.max_tool_calls,
        max_elapsed_ms: profile.max_elapsed_ms,
        max_cost_usd: None,
        max_depth: config.max_agent_depth,
    };
    let mut allowed_tools = profile.delegated_tools.iter().cloned().collect::<Vec<_>>();
    allowed_tools.sort();
    let authority = ::contracts::AgentDelegationAuthority::new(
        Some(request.context.workspace.clone()),
        allowed_tools,
        budget.clone(),
    );
    let prerequisites_available = root.is_some() && role_workflow_factory.is_some();
    let decomposition =
        cognit::DeterministicTaskDecompositionPolicy.decompose(&cognit::DecompositionContext {
            task_kind: request.requested_task_kind,
            requirements: request.requirements.clone(),
            multi_agent_enabled: config.multi_agent_enabled,
            automatic_for_coding: config.automatic_multi_agent_for_coding,
            agora_available: root.is_some(),
            parent_authority: prerequisites_available.then_some(authority.clone()),
            remaining_budget: budget.clone(),
            objective: objective.to_owned(),
        })?;
    let cognit::TaskDecomposition::RoleGraph {
        workspace_scope,
        allowed_capabilities,
        expected_evidence,
        ..
    } = decomposition
    else {
        return Ok(None);
    };
    let (task_node_id, expected_workspace_version) =
        root.ok_or_else(|| anyhow::anyhow!("role graph root task unavailable"))?;
    let factory = role_workflow_factory
        .ok_or_else(|| anyhow::anyhow!("role workflow factory unavailable"))?;
    let workflow = factory.bind(::agora::cognitive_role_workflow::TurnRoleLaunchContext {
        root_agent_id: process.agent_id,
        parent_agent_id: process.agent_id,
        parent_process_id: main_pid,
        workspace: request.context.workspace.clone(),
        delegator_authority: authority,
        remaining_budget: budget,
        cancellation,
    })?;
    let risk_level = ::agora::cognitive_role_workflow::classify_task_risk(&allowed_capabilities);
    Ok(Some((
        workflow,
        ::agora::cognitive_role_workflow::CodingWorkflowRequest {
            space: AgoraSpaceId(request.context.thread_id.0.clone()),
            task_node_id,
            expected_workspace_version,
            current_owner: main_pid,
            workspace_scope,
            project_instructions: Vec::new(),
            allowed_capabilities,
            expected_evidence,
            risk_level,
        },
    )))
}
