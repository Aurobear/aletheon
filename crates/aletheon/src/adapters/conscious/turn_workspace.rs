//! Agora workspace adapter for Turn clarification and root-task projection.

use std::sync::Arc;

use agora::AgoraService;
use contracts::{AgoraSpaceId, Clock, ProcessId, TurnRequest};

pub struct TurnSpaceSeed {
    pub agora: Option<Arc<dyn AgoraService>>,
    pub agora_version: u64,
    pub main_agent_id: Option<contracts::AgentId>,
}

pub struct ConsciousExecutionResources {
    pub action_loop: Option<Arc<dyn kernel::capability::governed::GovernedActionLoop>>,
    pub batch_planner: Option<Arc<dyn cognit::harness::BatchPlanner>>,
}

pub async fn resolve_execution_resources(
    conscious: Option<&Arc<dyn crate::composition::conscious_workspace::ConsciousTurnPort>>,
    session_id: &str,
    process_id: ProcessId,
) -> anyhow::Result<ConsciousExecutionResources> {
    let Some(conscious) = conscious else {
        return Ok(ConsciousExecutionResources {
            action_loop: None,
            batch_planner: None,
        });
    };
    let space = AgoraSpaceId(session_id.to_string());
    let (action_loop, batch_planner) = tokio::try_join!(
        conscious.resolve(space.clone(), process_id, process_id),
        conscious.batch_planner(space)
    )?;
    Ok(ConsciousExecutionResources {
        action_loop: Some(action_loop),
        batch_planner: Some(batch_planner),
    })
}

pub async fn seed_turn_space(
    agora: Option<&Arc<dyn AgoraService>>,
    kernel: &kernel::KernelRuntime,
    process_id: ProcessId,
    session_id: &str,
    input: &str,
) -> TurnSpaceSeed {
    let agora = agora.cloned();
    let agora_version = if let Some(service) = &agora {
        service
            .view(agora::contract::AgoraViewRequest {
                space: AgoraSpaceId(session_id.to_string()),
            })
            .await
            .map(|view| view.version)
            .unwrap_or(0)
    } else {
        tracing::warn!(target: "agora", "AgoraService is not configured; shared evidence commits disabled for this turn");
        0
    };
    let (agent_space, main_agent_id) = match kernel.inspect_process(process_id).await {
        Ok(snapshot) => (snapshot.space, Some(snapshot.agent_id)),
        Err(error) => {
            tracing::warn!(target: "space", %error, "inspect process failed; using ephemeral space for this turn");
            (contracts::SpaceId::new(), None)
        }
    };
    kernel.upsert_space_binding(
        agent_space,
        contracts::ContextBinding::Session(contracts::SessionId(session_id.to_string())),
    );
    kernel.upsert_space_binding(
        agent_space,
        contracts::ContextBinding::Agora(
            AgoraSpaceId(session_id.to_string()),
            contracts::AgoraVersion(agora_version),
        ),
    );
    if let Err(error) =
        kernel.set_space_overlay(agent_space, "turn_input", serde_json::json!(input))
    {
        tracing::warn!(target: "space", %error, "failed to store turn input overlay");
    }
    TurnSpaceSeed {
        agora,
        agora_version,
        main_agent_id,
    }
}

pub async fn observe_turn_input(
    conscious: Option<&Arc<dyn crate::composition::conscious_workspace::ConsciousTurnPort>>,
    session_id: &str,
    process_id: ProcessId,
    operation_id: contracts::OperationId,
    input: &str,
) -> anyhow::Result<()> {
    let Some(conscious) = conscious else {
        return Ok(());
    };
    conscious
        .observe_turn(application::conscious::ConsciousTurnObservation {
            space: AgoraSpaceId(session_id.to_string()),
            owner: process_id,
            root: process_id,
            operation: operation_id,
            input: input.to_string(),
        })
        .await
        .map(|_| ())
}

/// Project a terminal tool result into the shared cognitive workspace.
///
/// Agora is an optional integration for a Turn, so projection failures remain
/// observable best-effort effects and never replace the authoritative tool
/// result. The returned version is the freshest successfully observed or
/// committed optimistic version.
#[allow(clippy::too_many_arguments)]
pub async fn commit_tool_evidence(
    agora: Option<&Arc<dyn AgoraService>>,
    session_id: &str,
    author: ProcessId,
    current_version: u64,
    call_id: &str,
    tool_name: &str,
    content: &str,
    is_error: bool,
    clock: &dyn Clock,
) -> u64 {
    let Some(agora) = agora else {
        tracing::warn!(target: "agora", "AgoraService missing; skipping shared evidence commit");
        return current_version;
    };
    let space = AgoraSpaceId(session_id.to_owned());
    let mut version = current_version;
    match agora
        .view(agora::contract::AgoraViewRequest {
            space: space.clone(),
        })
        .await
    {
        Ok(view) => version = view.version,
        Err(error) => tracing::warn!(
            target: "agora",
            error = %error,
            "agora view refresh (evidence) failed"
        ),
    }
    let evidence = contracts::Evidence::from_tool_result(
        call_id.to_owned(),
        tool_name.to_owned(),
        content.to_owned(),
        is_error,
    );
    let expires_at_ms = clock.wall_now().0.saturating_add(30_000);
    let proposal = agora::contract::AgoraProposal {
        id: uuid::Uuid::new_v4(),
        space,
        author,
        base_version: version,
        operation: agora::contract::AgoraOperation::AcceptEvidence { evidence },
        evidence: vec![format!("tool-call:{call_id}")],
        confidence: if is_error { 0.5 } else { 1.0 },
        expires_at_ms: Some(expires_at_ms),
    };
    let permit = agora::contract::WorkspaceCommitPermit::issue_for(&proposal, expires_at_ms);
    match (agora.propose(proposal).await, permit) {
        (Ok(id), Ok(permit)) => match agora.commit(id, permit).await {
            Ok(receipt) => receipt.commit.version,
            Err(error) => {
                tracing::warn!(target: "agora", error = %error, "agora commit (evidence) failed");
                version
            }
        },
        (Err(error), _) => {
            tracing::warn!(target: "agora", error = %error, "agora propose (evidence) failed");
            version
        }
        (_, Err(error)) => {
            tracing::warn!(target: "agora", error = %error, "agora permit issue failed");
            version
        }
    }
}

pub async fn resume_single_pending_clarification(
    agora: Option<&Arc<dyn AgoraService>>,
    request: &TurnRequest,
    response: &str,
) -> anyhow::Result<()> {
    let Some(agora) = agora else {
        return Ok(());
    };
    let space = AgoraSpaceId(request.context.thread_id.0.clone());
    let tasks = agora.list_tasks(space.clone()).await?;
    let mut pending = Vec::new();
    for task in tasks
        .tasks
        .iter()
        .filter(|task| task.status == ::contracts::cognitive_workflow::CognitiveTaskStatus::Blocked)
    {
        let projection = agora
            .project_task(::contracts::cognitive_workflow::AgoraProjectionRequest {
                space: space.clone(),
                task_node_id: task.id.clone(),
                role: ::contracts::cognitive_workflow::CognitiveRole::Root,
                max_artifacts: 0,
                include_kinds: Vec::new(),
            })
            .await?;
        if let (Some(clarification), Some(owner)) = (projection.clarification, task.owner) {
            pending.push((task.id.clone(), clarification.id, owner));
        }
    }
    // Never guess which question a response addresses. A single pending
    // question is the only unambiguous automatic resume case.
    if pending.len() != 1 {
        return Ok(());
    }
    let (task_node_id, clarification_id, owner) = pending.remove(0);
    let turn_id = request
        .context
        .turn_id
        .ok_or_else(|| anyhow::anyhow!("clarification response lacks canonical turn id"))?;
    let response_event_id = format!(
        "session:{}:turn:{}:user_message",
        request.context.thread_id.0, turn_id.0
    );
    agora::cognitive_workspace::CognitiveWorkspaceCoordinator::new(agora.clone())
        .resume_from_user_response(
            space,
            task_node_id,
            clarification_id,
            response.to_owned(),
            response_event_id,
            tasks.workspace_version,
            owner,
        )
        .await
        .map_err(anyhow::Error::new)?;
    Ok(())
}

pub async fn ensure_root_cognitive_task(
    agora: Option<&Arc<dyn AgoraService>>,
    request: &TurnRequest,
    objective: &str,
    owner: ProcessId,
) -> anyhow::Result<Option<(::contracts::cognitive_workflow::CognitiveTaskNodeId, u64)>> {
    let Some(agora) = agora else {
        return Ok(None);
    };
    use ::contracts::cognitive_workflow::{
        CognitiveRole, CognitiveStage, CognitiveTaskNode, CognitiveTaskNodeId, CognitiveTaskStatus,
    };
    let space = AgoraSpaceId(request.context.thread_id.0.clone());
    let tasks = agora.list_tasks(space.clone()).await?;
    let turn_id = request
        .context
        .turn_id
        .ok_or_else(|| anyhow::anyhow!("cognitive root requires canonical TurnId"))?;
    let root_id = CognitiveTaskNodeId(format!("root:{}", turn_id.0));
    if tasks.tasks.iter().any(|task| task.id == root_id) {
        return Ok(Some((root_id, tasks.workspace_version)));
    }
    let bounded_objective = objective.chars().take(4096).collect::<String>();
    let task = CognitiveTaskNode {
        id: root_id.clone(),
        parent_id: None,
        objective: bounded_objective,
        role: CognitiveRole::Root,
        stage: CognitiveStage::Contract,
        status: CognitiveTaskStatus::Running,
        owner: Some(owner),
        role_profile: ::contracts::cognitive_workflow::CognitiveRoleProfile::canonical(
            CognitiveRole::Root,
        )
        .reference,
        budget: ::contracts::cognitive_workflow::CognitiveRoleProfile::canonical(
            CognitiveRole::Root,
        )
        .budget,
        dependencies: Vec::new(),
        acceptance_criteria: Vec::new(),
        workspace_scope: request
            .context
            .workspace
            .writable_roots()
            .iter()
            .map(|path| path.display().to_string())
            .collect(),
        required_artifact_kinds: vec![
            ::contracts::cognitive_workflow::CognitiveArtifactKind::TaskContract,
        ],
        artifact_refs: Vec::new(),
        unresolved_finding_ids: Vec::new(),
    };
    let workspace = agora::cognitive_workspace::CognitiveWorkspaceCoordinator::new(agora.clone());
    let version = workspace
        .commit_task_at(space.clone(), tasks.workspace_version, task, owner)
        .await
        .map_err(anyhow::Error::new)?;
    let requirement_refs = request
        .evaluation_contract
        .as_ref()
        .map(|contract| {
            contract
                .requirement_refs
                .iter()
                .map(|item| item.0.clone())
                .collect()
        })
        .unwrap_or_else(|| vec![format!("turn:{}:objective", turn_id.0)]);
    let acceptance_criteria = request
        .evaluation_contract
        .as_ref()
        .map(|contract| {
            contract
                .required_gates
                .iter()
                .map(|gate| gate.name.clone())
                .collect()
        })
        .unwrap_or_default();
    let contract = ::contracts::cognitive_workflow::CognitiveArtifactEnvelope::proposed(
        space.clone(),
        root_id.clone(),
        owner,
        vec![format!("turn:{}", turn_id.0)],
        Vec::new(),
        1.0,
        ::contracts::cognitive_workflow::CognitiveArtifact::TaskContract(
            ::contracts::cognitive_workflow::CognitiveTaskContractArtifact {
                objective: objective.chars().take(4096).collect(),
                requirement_refs,
                acceptance_criteria,
                instruction_refs: Vec::new(),
                workspace_scope: request
                    .context
                    .workspace
                    .writable_roots()
                    .iter()
                    .map(|path| path.display().to_string())
                    .collect(),
            },
        ),
    )?;
    let version = workspace
        .commit_artifact_at(space, version, contract, owner)
        .await
        .map_err(anyhow::Error::new)?;
    Ok(Some((root_id, version)))
}
