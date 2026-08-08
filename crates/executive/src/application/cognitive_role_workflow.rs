//! Typed Planner → Explorer → Executor workflow over AgentControl and Agora.
//! Agent runtimes receive bounded task packets; only host-validated artifacts
//! and gate decisions advance the shared task.

mod stages;
mod state_machine;

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
#[cfg(test)]
use fabric::cognitive_workflow::CognitiveArtifact;
use fabric::cognitive_workflow::{
    AgentTaskPacket, CognitiveArtifactEnvelope, CognitiveRole, CognitiveRoleOutput,
    CognitiveRoleProfile, CognitiveTaskNodeId, CognitiveTaskRuntimeBinding, StageDecision,
    StageDecisionKind,
};
use fabric::{
    AgentBudget, AgentContextFork, AgentControlPort, AgentId, AgentProfileId, AgentRunStatus,
    AgentSpawnRequest, AgentWaitRequest, AgoraSpaceId, OperationId, ProcessId, RuntimeId,
    WorkspacePolicy,
};
use tokio_util::sync::CancellationToken;

use super::cognitive_workspace::{CognitiveWorkspaceCoordinator, CognitiveWorkspaceError};
use stages::*;
use state_machine::{acceptance_plan, coding_plan, RepairBudget, TransitionReason};

pub(crate) use state_machine::classify_task_risk;

#[derive(Debug, Clone)]
pub struct RoleLaunchProfile {
    pub profile_id: AgentProfileId,
    pub allowed_tools: Vec<String>,
}

pub struct AgentControlRoleInvoker {
    control: Arc<dyn AgentControlPort>,
    root_agent_id: AgentId,
    parent_agent_id: Option<AgentId>,
    parent_process_id: Option<ProcessId>,
    runtime_id: RuntimeId,
    trusted_workspace: WorkspacePolicy,
    profiles: HashMap<CognitiveRole, RoleLaunchProfile>,
    wait_timeout_ms: u64,
    delegator_authority: fabric::AgentDelegationAuthority,
    remaining_budget: tokio::sync::Mutex<AgentBudget>,
    cancellation: CancellationToken,
}

impl AgentControlRoleInvoker {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        control: Arc<dyn AgentControlPort>,
        root_agent_id: AgentId,
        parent_agent_id: Option<AgentId>,
        parent_process_id: Option<ProcessId>,
        runtime_id: RuntimeId,
        trusted_workspace: WorkspacePolicy,
        profiles: HashMap<CognitiveRole, RoleLaunchProfile>,
        wait_timeout_ms: u64,
        delegator_authority: fabric::AgentDelegationAuthority,
        remaining_budget: AgentBudget,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(wait_timeout_ms > 0, "role wait timeout must be nonzero");
        Ok(Self {
            control,
            root_agent_id,
            parent_agent_id,
            parent_process_id,
            runtime_id,
            trusted_workspace,
            profiles,
            wait_timeout_ms,
            delegator_authority,
            remaining_budget: tokio::sync::Mutex::new(remaining_budget),
            cancellation,
        })
    }
}

#[derive(Clone)]
pub struct RoleWorkflowFactory {
    control: Arc<dyn AgentControlPort>,
    workspace: Arc<CognitiveWorkspaceCoordinator>,
    runtime_id: RuntimeId,
    profiles: HashMap<CognitiveRole, RoleLaunchProfile>,
    wait_timeout_ms: u64,
}

#[derive(Clone)]
pub struct TurnRoleLaunchContext {
    pub root_agent_id: AgentId,
    pub parent_agent_id: AgentId,
    pub parent_process_id: ProcessId,
    pub workspace: WorkspacePolicy,
    pub delegator_authority: fabric::AgentDelegationAuthority,
    pub remaining_budget: AgentBudget,
    pub cancellation: CancellationToken,
}

impl RoleWorkflowFactory {
    pub fn new(
        control: Arc<dyn AgentControlPort>,
        workspace: Arc<CognitiveWorkspaceCoordinator>,
        runtime_id: RuntimeId,
        profiles: HashMap<CognitiveRole, RoleLaunchProfile>,
        wait_timeout_ms: u64,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(wait_timeout_ms > 0, "role wait timeout must be nonzero");
        for role in [
            CognitiveRole::Planner,
            CognitiveRole::Explorer,
            CognitiveRole::Executor,
            CognitiveRole::Tester,
            CognitiveRole::Reviewer,
            CognitiveRole::Fixer,
        ] {
            anyhow::ensure!(
                profiles.contains_key(&role),
                "missing launch profile for {role:?}"
            );
        }
        Ok(Self {
            control,
            workspace,
            runtime_id,
            profiles,
            wait_timeout_ms,
        })
    }

    pub fn bind(&self, context: TurnRoleLaunchContext) -> anyhow::Result<CognitiveRoleWorkflow> {
        let invoker = AgentControlRoleInvoker::new(
            self.control.clone(),
            context.root_agent_id,
            Some(context.parent_agent_id),
            Some(context.parent_process_id),
            self.runtime_id.clone(),
            context.workspace,
            self.profiles.clone(),
            self.wait_timeout_ms,
            context.delegator_authority,
            context.remaining_budget,
            context.cancellation,
        )?;
        Ok(CognitiveRoleWorkflow::new(
            self.workspace.clone(),
            Arc::new(invoker),
        ))
    }
}

#[derive(Debug, Clone)]
pub struct RoleInvocationTerminal {
    pub process_id: ProcessId,
    pub operation_id: OperationId,
    pub output: CognitiveRoleOutput,
}

#[async_trait]
pub trait CognitiveRoleInvoker: Send + Sync {
    async fn invoke(
        &self,
        packet: AgentTaskPacket,
        binding: CognitiveTaskRuntimeBinding,
    ) -> anyhow::Result<RoleInvocationTerminal>;
}

#[async_trait]
impl CognitiveRoleInvoker for AgentControlRoleInvoker {
    async fn invoke(
        &self,
        packet: AgentTaskPacket,
        binding: CognitiveTaskRuntimeBinding,
    ) -> anyhow::Result<RoleInvocationTerminal> {
        packet.validate()?;
        let launch = self
            .profiles
            .get(&packet.role_profile.role)
            .ok_or_else(|| anyhow::anyhow!("no launch profile for cognitive role"))?;
        let packet_json = serde_json::to_string(&packet)?;
        let task = format!(
            "Execute this versioned cognitive task packet. Return only one JSON CognitiveRoleOutput object whose projection_id and workspace_version exactly match the packet; prose cannot advance the stage.\n{packet_json}"
        );
        let budget = &packet.role_profile.budget;
        let mut remaining = self.remaining_budget.lock().await;
        let child_budget = AgentBudget {
            max_input_tokens: budget.max_input_tokens.min(remaining.max_input_tokens),
            max_output_tokens: budget.max_output_tokens.min(remaining.max_output_tokens),
            max_tool_calls: budget.max_tool_calls.min(remaining.max_tool_calls),
            max_elapsed_ms: budget.max_elapsed_ms.min(remaining.max_elapsed_ms),
            max_cost_usd: remaining.max_cost_usd,
            max_depth: 1.min(remaining.max_depth),
        };
        child_budget.validate().map_err(anyhow::Error::new)?;
        // Sequential execution makes this reservation the only outstanding
        // role budget. It remains charged on failure; unused dimensions are
        // released from authoritative terminal usage below.
        remaining.max_input_tokens = remaining
            .max_input_tokens
            .saturating_sub(child_budget.max_input_tokens);
        remaining.max_output_tokens = remaining
            .max_output_tokens
            .saturating_sub(child_budget.max_output_tokens);
        remaining.max_tool_calls = remaining
            .max_tool_calls
            .saturating_sub(child_budget.max_tool_calls);
        remaining.max_elapsed_ms = remaining
            .max_elapsed_ms
            .saturating_sub(child_budget.max_elapsed_ms);
        if remaining.max_cost_usd.is_some() {
            remaining.max_cost_usd = Some(0.0);
        }
        drop(remaining);
        let handle = self
            .control
            .spawn(AgentSpawnRequest {
                root_agent_id: self.root_agent_id,
                parent_agent_id: self.parent_agent_id,
                parent_process_id: self.parent_process_id,
                profile_id: launch.profile_id.clone(),
                runtime_id: self.runtime_id.clone(),
                trusted_workspace: Some(self.trusted_workspace.clone()),
                delegator_authority: Some(self.delegator_authority.clone()),
                cognitive_binding: Some(binding),
                task,
                context: AgentContextFork::None,
                broadcast_refs: Vec::new(),
                allowed_tools: launch.allowed_tools.clone(),
                budget: child_budget.clone(),
                background_decls: Vec::new(),
            })
            .await
            .map_err(anyhow::Error::new)?;
        let wait_request = AgentWaitRequest {
            caller_root_agent_id: self.root_agent_id,
            agent_id: handle.agent_id,
            timeout_ms: self.wait_timeout_ms.min(child_budget.max_elapsed_ms),
        };
        let snapshot = tokio::select! {
            snapshot = self.control.wait(wait_request.clone()) => snapshot.map_err(anyhow::Error::new)?,
            _ = self.cancellation.cancelled() => {
                tracing::debug!(
                    reason_code = TransitionReason::Cancelled.code(),
                    "cognitive role cancelled with parent workflow"
                );
                self.control.cancel(self.root_agent_id, handle.agent_id).await.map_err(anyhow::Error::new)?;
                self.control.wait(wait_request).await.map_err(anyhow::Error::new)?
            }
        };
        anyhow::ensure!(
            snapshot.status.is_terminal(),
            "Agent wait returned a non-terminal snapshot"
        );
        anyhow::ensure!(
            snapshot.status == AgentRunStatus::Succeeded,
            "cognitive role runtime failed: {:?}: {}",
            snapshot.status,
            snapshot.last_error.unwrap_or_default()
        );
        let result = snapshot.result.ok_or_else(|| {
            anyhow::anyhow!("successful cognitive role has no authoritative result")
        })?;
        let mut remaining = self.remaining_budget.lock().await;
        remaining.max_input_tokens = remaining.max_input_tokens.saturating_add(
            child_budget
                .max_input_tokens
                .saturating_sub(result.usage.input_tokens),
        );
        remaining.max_output_tokens = remaining.max_output_tokens.saturating_add(
            child_budget
                .max_output_tokens
                .saturating_sub(result.usage.output_tokens),
        );
        remaining.max_elapsed_ms = remaining.max_elapsed_ms.saturating_add(
            child_budget
                .max_elapsed_ms
                .saturating_sub(result.usage.elapsed_ms),
        );
        let used_tools = result
            .usage
            .observability
            .tool_calls
            .unwrap_or(u64::from(child_budget.max_tool_calls));
        remaining.max_tool_calls = remaining.max_tool_calls.saturating_add(
            child_budget
                .max_tool_calls
                .saturating_sub(used_tools.try_into().unwrap_or(u32::MAX)),
        );
        if let Some(reserved) = child_budget.max_cost_usd {
            remaining.max_cost_usd = Some(
                remaining.max_cost_usd.unwrap_or_default()
                    + (reserved - result.usage.cost_usd.unwrap_or(reserved)).max(0.0),
            );
        }
        drop(remaining);
        let output: CognitiveRoleOutput =
            serde_json::from_str(&result.output).map_err(|error| {
                anyhow::anyhow!("cognitive role output is not strict JSON: {error}")
            })?;
        output.validate_for(&packet)?;
        Ok(RoleInvocationTerminal {
            process_id: handle.process_id,
            operation_id: handle.operation_id,
            output,
        })
    }
}

#[derive(Debug, Clone)]
pub struct CodingWorkflowRequest {
    pub space: AgoraSpaceId,
    pub task_node_id: CognitiveTaskNodeId,
    pub expected_workspace_version: u64,
    pub current_owner: ProcessId,
    pub workspace_scope: Vec<String>,
    pub project_instructions: Vec<String>,
    pub allowed_capabilities: Vec<fabric::AgentRuntimeCapability>,
    pub expected_evidence: Vec<String>,
    /// Typed task risk used by the evidence-driven controller to skip
    /// low-risk planning/exploration stages (M1-AUDIT-003).
    pub risk_level: fabric::types::admission::RiskLevel,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct CodingWorkflowReceipt {
    pub workspace_version: u64,
    pub current_owner: ProcessId,
    pub artifact_ids: Vec<fabric::cognitive_workflow::CognitiveArtifactId>,
    pub operation_ids: Vec<OperationId>,
}

#[derive(Debug, Clone)]
pub struct AcceptanceWorkflowRequest {
    pub space: AgoraSpaceId,
    pub task_node_id: CognitiveTaskNodeId,
    pub expected_workspace_version: u64,
    pub current_owner: ProcessId,
    pub workspace_scope: Vec<String>,
    pub project_instructions: Vec<String>,
    pub allowed_capabilities: Vec<fabric::AgentRuntimeCapability>,
    pub expected_evidence: Vec<String>,
    pub risk_level: fabric::types::admission::RiskLevel,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct AcceptanceWorkflowReceipt {
    pub workspace_version: u64,
    pub current_owner: ProcessId,
    pub validation_artifact_ids: Vec<fabric::cognitive_workflow::CognitiveArtifactId>,
    pub review_artifact_ids: Vec<fabric::cognitive_workflow::CognitiveArtifactId>,
    pub repair_artifact_ids: Vec<fabric::cognitive_workflow::CognitiveArtifactId>,
    pub resolved_finding_ids: Vec<String>,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FullCodingWorkflowReceipt {
    pub coding: CodingWorkflowReceipt,
    pub acceptance: AcceptanceWorkflowReceipt,
}

struct PreparedRole {
    packet: AgentTaskPacket,
    terminal: RoleInvocationTerminal,
    envelope: CognitiveArtifactEnvelope,
    bound_version: u64,
}

pub struct CognitiveRoleWorkflow {
    workspace: Arc<CognitiveWorkspaceCoordinator>,
    invoker: Arc<dyn CognitiveRoleInvoker>,
}

impl CognitiveRoleWorkflow {
    pub fn new(
        workspace: Arc<CognitiveWorkspaceCoordinator>,
        invoker: Arc<dyn CognitiveRoleInvoker>,
    ) -> Self {
        Self { workspace, invoker }
    }

    pub async fn run_planner_explorer_executor(
        &self,
        request: CodingWorkflowRequest,
    ) -> anyhow::Result<CodingWorkflowReceipt> {
        let mut version = request.expected_workspace_version;
        let mut owner = request.current_owner;
        let mut artifact_ids = Vec::new();
        let mut operation_ids = Vec::new();
        let stage_plan = coding_plan(
            request.risk_level,
            &request.workspace_scope,
            &request.expected_evidence,
        );
        tracing::debug!(
            risk = ?request.risk_level,
            uncertainty = ?stage_plan.uncertainty,
            reason_code = stage_plan.reason.code(),
            roles = ?stage_plan.roles,
            "evidence-driven cognitive workflow planned"
        );
        for role in stage_plan.roles {
            let projection = self
                .workspace
                .project_role(
                    request.space.clone(),
                    request.task_node_id.clone(),
                    role,
                    CognitiveRoleProfile::canonical(role)
                        .context_projection
                        .max_artifacts,
                    Vec::new(),
                )
                .await?;
            anyhow::ensure!(
                projection.workspace_version == version,
                "workflow projection became stale before role launch"
            );
            let profile = CognitiveRoleProfile::canonical(role);
            let mut task = projection.task.clone();
            task.role = role;
            task.role_profile = profile.reference.clone();
            task.budget = profile.budget.clone();
            task.workspace_scope = if role.can_write_workspace() {
                request.workspace_scope.clone()
            } else {
                Vec::new()
            };
            let packet = AgentTaskPacket {
                schema_version: 1,
                task,
                role_profile: profile.clone(),
                project_instructions: request.project_instructions.clone(),
                workspace_roots: request.workspace_scope.clone(),
                allowed_capabilities: request.allowed_capabilities.clone(),
                expected_evidence: request.expected_evidence.clone(),
                acceptance_criteria: projection.task.acceptance_criteria.clone(),
                selected_artifacts: projection.artifacts.clone(),
                projection_receipt: projection.receipt.clone(),
            };
            packet.validate()?;
            let binding = CognitiveTaskRuntimeBinding {
                space: request.space.clone(),
                task_node_id: request.task_node_id.clone(),
                expected_workspace_version: version,
                expected_owner: owner,
                role,
                role_profile: profile.reference.clone(),
                budget: profile.budget.clone(),
                workspace_scope: if role.can_write_workspace() {
                    request.workspace_scope.clone()
                } else {
                    Vec::new()
                },
            };
            let terminal = self.invoker.invoke(packet.clone(), binding).await?;
            terminal.output.validate_for(&packet)?;
            version = version.saturating_add(1); // bind-before-launch commit
            let envelope = CognitiveArtifactEnvelope::proposed(
                request.space.clone(),
                request.task_node_id.clone(),
                terminal.process_id,
                terminal.output.source_versions.clone(),
                terminal.output.evidence_refs.clone(),
                terminal.output.confidence,
                terminal.output.artifact.clone(),
            )?;
            if let Err(gate_error) = validate_stage_artifact(role, &packet, &envelope) {
                self.workspace
                    .record_stage_decision_at(
                        request.space.clone(),
                        version,
                        request.task_node_id.clone(),
                        StageDecision {
                            decision: StageDecisionKind::Reject,
                            reason: TransitionReason::StructuredArtifactRejected
                                .detail(&gate_error.to_string()),
                            finding_ids: Vec::new(),
                            evidence_refs: terminal.output.evidence_refs.clone(),
                        },
                        terminal.process_id,
                    )
                    .await
                    .map_err(workspace_error)?;
                return Err(gate_error);
            }
            let artifact_id = envelope.id.clone();
            version = self
                .workspace
                .commit_artifact_at(
                    request.space.clone(),
                    version,
                    envelope,
                    terminal.process_id,
                )
                .await
                .map_err(workspace_error)?;
            version = self
                .workspace
                .record_stage_decision_at(
                    request.space.clone(),
                    version,
                    request.task_node_id.clone(),
                    StageDecision {
                        decision: StageDecisionKind::Accept,
                        reason: TransitionReason::StructuredArtifactAccepted.detail(&format!(
                            "host gate accepted {role:?} artifact at exact version"
                        )),
                        finding_ids: Vec::new(),
                        evidence_refs: vec![artifact_id.0.to_string()],
                    },
                    terminal.process_id,
                )
                .await
                .map_err(workspace_error)?;
            owner = terminal.process_id;
            artifact_ids.push(artifact_id);
            operation_ids.push(terminal.operation_id);
        }
        Ok(CodingWorkflowReceipt {
            workspace_version: version,
            current_owner: owner,
            artifact_ids,
            operation_ids,
        })
    }

    pub async fn run_full_coding_workflow(
        &self,
        coding: CodingWorkflowRequest,
    ) -> anyhow::Result<FullCodingWorkflowReceipt> {
        let acceptance_template = AcceptanceWorkflowRequest {
            space: coding.space.clone(),
            task_node_id: coding.task_node_id.clone(),
            expected_workspace_version: 0,
            current_owner: coding.current_owner,
            workspace_scope: coding.workspace_scope.clone(),
            project_instructions: coding.project_instructions.clone(),
            allowed_capabilities: coding.allowed_capabilities.clone(),
            expected_evidence: coding.expected_evidence.clone(),
            risk_level: coding.risk_level,
        };
        let coding = self.run_planner_explorer_executor(coding).await?;
        let acceptance = self
            .run_reviewer_tester_fixer(AcceptanceWorkflowRequest {
                expected_workspace_version: coding.workspace_version,
                current_owner: coding.current_owner,
                ..acceptance_template
            })
            .await?;
        Ok(FullCodingWorkflowReceipt { coding, acceptance })
    }

    pub async fn run_reviewer_tester_fixer(
        &self,
        request: AcceptanceWorkflowRequest,
    ) -> anyhow::Result<AcceptanceWorkflowReceipt> {
        let mut version = request.expected_workspace_version;
        let mut owner = request.current_owner;
        let mut validation_artifact_ids = Vec::new();
        let mut review_artifact_ids = Vec::new();
        let mut repair_artifact_ids = Vec::new();
        let mut resolved_finding_ids = Vec::new();
        let stage_plan = acceptance_plan(request.risk_level, &request.expected_evidence);
        tracing::debug!(
            tester_required = stage_plan.tester_required,
            reviewer_required = stage_plan.reviewer_required,
            validation_reason = TransitionReason::ValidationRequired.code(),
            review_reason = TransitionReason::ReviewRequired.code(),
            "evidence-driven acceptance stages planned"
        );
        let mut repair_budget = RepairBudget::new(stage_plan.max_fix_attempts);
        if !stage_plan.tester_required && !stage_plan.reviewer_required {
            return Ok(AcceptanceWorkflowReceipt {
                workspace_version: version,
                current_owner: owner,
                validation_artifact_ids,
                review_artifact_ids,
                repair_artifact_ids,
                resolved_finding_ids,
            });
        }

        // M1: Tester stage – evidence-driven, not fixed-count.
        // Only invoked when validation evidence is expected/missing or
        // risk demands terminal validation.
        if stage_plan.tester_required {
            let mut validation = self
                .invoke_acceptance_role(&request, version, owner, CognitiveRole::Tester, None)
                .await?;
            version = validation.bound_version;
            let validation_record =
                validate_validation_artifact(&validation.packet, &validation.envelope)?;
            let validation_passed = validation_record.passed;
            let validation_id = validation.envelope.id.clone();
            version = self
                .workspace
                .commit_artifact_at(
                    request.space.clone(),
                    version,
                    validation.envelope,
                    validation.terminal.process_id,
                )
                .await
                .map_err(workspace_error)?;
            owner = validation.terminal.process_id;
            validation_artifact_ids.push(validation_id.clone());

            if !validation_passed {
                repair_budget.claim().map_err(|reason| {
                    anyhow::anyhow!(reason.detail("validation repair limit reached"))
                })?;
                let repair_scope = latest_change_set(&validation.packet)?.changed_paths.clone();
                let finding = format!("validation:{}", validation_id.0);
                version = self
                    .set_unresolved_findings(&request, version, owner, vec![finding.clone()])
                    .await?;
                version = self
                    .record_decision(
                        &request,
                        version,
                        owner,
                        StageDecisionKind::Repair,
                        &TransitionReason::ValidationFailed.detail("terminal validation failed"),
                        vec![finding.clone()],
                        vec![validation_id.0.to_string()],
                    )
                    .await?;
                let repair = self
                    .invoke_acceptance_role(
                        &request,
                        version,
                        owner,
                        CognitiveRole::Fixer,
                        Some(repair_scope),
                    )
                    .await?;
                version = repair.bound_version;
                validate_fixer_artifact(
                    &repair.packet,
                    &repair.envelope,
                    std::slice::from_ref(&finding),
                )?;
                let repair_id = repair.envelope.id.clone();
                version = self
                    .workspace
                    .commit_artifact_at(
                        request.space.clone(),
                        version,
                        repair.envelope,
                        repair.terminal.process_id,
                    )
                    .await
                    .map_err(workspace_error)?;
                owner = repair.terminal.process_id;
                repair_artifact_ids.push(repair_id.clone());
                version = self
                    .record_decision(
                        &request,
                        version,
                        owner,
                        StageDecisionKind::Accept,
                        &TransitionReason::StructuredFailureRepair
                            .detail("bounded validation repair committed"),
                        vec![finding.clone()],
                        vec![repair_id.0.to_string()],
                    )
                    .await?;
                validation = self
                    .invoke_acceptance_role(&request, version, owner, CognitiveRole::Tester, None)
                    .await?;
                version = validation.bound_version;
                let record =
                    validate_validation_artifact(&validation.packet, &validation.envelope)?;
                anyhow::ensure!(
                    record.passed,
                    "validation still fails after the bounded repair"
                );
                let id = validation.envelope.id.clone();
                version = self
                    .workspace
                    .commit_artifact_at(
                        request.space.clone(),
                        version,
                        validation.envelope,
                        validation.terminal.process_id,
                    )
                    .await
                    .map_err(workspace_error)?;
                owner = validation.terminal.process_id;
                validation_artifact_ids.push(id.clone());
                version = self
                    .record_decision(
                        &request,
                        version,
                        owner,
                        StageDecisionKind::Accept,
                        &TransitionReason::RepairValidated
                            .detail("terminal validation passed after repair"),
                        vec![finding.clone()],
                        vec![id.0.to_string()],
                    )
                    .await?;
                resolved_finding_ids.push(finding);
            } else {
                version = self
                    .record_decision(
                        &request,
                        version,
                        owner,
                        StageDecisionKind::Accept,
                        &TransitionReason::ValidationPassed.detail("terminal validation passed"),
                        Vec::new(),
                        vec![validation_id.0.to_string()],
                    )
                    .await?;
            }
        }

        // M1: Reviewer stage – evidence-driven, not fixed-count.
        // Only invoked when risk demands independent human/automated review.
        if stage_plan.reviewer_required {
            let review = self
                .invoke_acceptance_role(&request, version, owner, CognitiveRole::Reviewer, None)
                .await?;
            version = review.bound_version;
            let findings = validate_review_artifact(
                &review.packet,
                &review.envelope,
                &request.workspace_scope,
                None,
            )?;
            let unresolved_findings = findings
                .iter()
                .filter(|finding| !finding.resolved)
                .cloned()
                .collect::<Vec<_>>();
            let unresolved = unresolved_findings
                .iter()
                .map(|finding| finding.id.clone())
                .collect::<Vec<_>>();
            let review_id = review.envelope.id.clone();
            version = self
                .workspace
                .commit_artifact_at(
                    request.space.clone(),
                    version,
                    review.envelope,
                    review.terminal.process_id,
                )
                .await
                .map_err(workspace_error)?;
            owner = review.terminal.process_id;
            review_artifact_ids.push(review_id.clone());
            if unresolved.is_empty() {
                version = self
                    .set_unresolved_findings(&request, version, owner, Vec::new())
                    .await?;
                version = self
                    .record_decision(
                        &request,
                        version,
                        owner,
                        StageDecisionKind::Accept,
                        &TransitionReason::ReviewPassed
                            .detail("independent review accepted the exact validated change"),
                        Vec::new(),
                        vec![review_id.0.to_string()],
                    )
                    .await?;
            } else {
                repair_budget.claim().map_err(|reason| {
                    anyhow::anyhow!(reason.detail("review repair limit reached"))
                })?;
                let repair_scope =
                    unresolved_finding_scope(&unresolved_findings, &request.workspace_scope)?;
                version = self
                    .set_unresolved_findings(&request, version, owner, unresolved.clone())
                    .await?;
                version = self
                    .record_decision(
                        &request,
                        version,
                        owner,
                        StageDecisionKind::Repair,
                        &TransitionReason::ReviewFailed
                            .detail("independent review rejected the change"),
                        unresolved.clone(),
                        vec![review_id.0.to_string()],
                    )
                    .await?;
                let repair = self
                    .invoke_acceptance_role(
                        &request,
                        version,
                        owner,
                        CognitiveRole::Fixer,
                        Some(repair_scope),
                    )
                    .await?;
                version = repair.bound_version;
                validate_fixer_artifact(&repair.packet, &repair.envelope, &unresolved)?;
                let repair_id = repair.envelope.id.clone();
                version = self
                    .workspace
                    .commit_artifact_at(
                        request.space.clone(),
                        version,
                        repair.envelope,
                        repair.terminal.process_id,
                    )
                    .await
                    .map_err(workspace_error)?;
                owner = repair.terminal.process_id;
                repair_artifact_ids.push(repair_id.clone());
                version = self
                    .record_decision(
                        &request,
                        version,
                        owner,
                        StageDecisionKind::Accept,
                        &TransitionReason::StructuredFailureRepair.detail(
                            "bounded review repair committed with preserved finding identities",
                        ),
                        unresolved.clone(),
                        vec![repair_id.0.to_string()],
                    )
                    .await?;

                // M1: re-validation after review repair is also evidence-driven.
                if stage_plan.tester_required {
                    let validation = self
                        .invoke_acceptance_role(
                            &request,
                            version,
                            owner,
                            CognitiveRole::Tester,
                            None,
                        )
                        .await?;
                    version = validation.bound_version;
                    let validation_record =
                        validate_validation_artifact(&validation.packet, &validation.envelope)?;
                    anyhow::ensure!(
                        validation_record.passed,
                        "repaired change failed terminal validation"
                    );
                    let validation_id = validation.envelope.id.clone();
                    version = self
                        .workspace
                        .commit_artifact_at(
                            request.space.clone(),
                            version,
                            validation.envelope,
                            validation.terminal.process_id,
                        )
                        .await
                        .map_err(workspace_error)?;
                    owner = validation.terminal.process_id;
                    validation_artifact_ids.push(validation_id.clone());
                    version = self
                        .record_decision(
                            &request,
                            version,
                            owner,
                            StageDecisionKind::Accept,
                            &TransitionReason::RepairValidated
                                .detail("repaired change passed terminal validation"),
                            unresolved.clone(),
                            vec![validation_id.0.to_string()],
                        )
                        .await?;
                }

                let rereview = self
                    .invoke_acceptance_role(&request, version, owner, CognitiveRole::Reviewer, None)
                    .await?;
                version = rereview.bound_version;
                let rereview_findings = validate_review_artifact(
                    &rereview.packet,
                    &rereview.envelope,
                    &request.workspace_scope,
                    Some(&unresolved_findings),
                )?;
                anyhow::ensure!(
                    rereview_findings.iter().all(|finding| finding.resolved),
                    "re-review did not resolve every preserved finding"
                );
                let rereview_id = rereview.envelope.id.clone();
                version = self
                    .workspace
                    .commit_artifact_at(
                        request.space.clone(),
                        version,
                        rereview.envelope,
                        rereview.terminal.process_id,
                    )
                    .await
                    .map_err(workspace_error)?;
                owner = rereview.terminal.process_id;
                review_artifact_ids.push(rereview_id.clone());
                version = self
                    .set_unresolved_findings(&request, version, owner, Vec::new())
                    .await?;
                version = self
                    .record_decision(
                        &request,
                        version,
                        owner,
                        StageDecisionKind::Accept,
                        &TransitionReason::ReviewPassed
                            .detail("independent re-review resolved every preserved finding"),
                        unresolved.clone(),
                        vec![rereview_id.0.to_string()],
                    )
                    .await?;
                resolved_finding_ids.extend(unresolved);
            }
        }

        Ok(AcceptanceWorkflowReceipt {
            workspace_version: version,
            current_owner: owner,
            validation_artifact_ids,
            review_artifact_ids,
            repair_artifact_ids,
            resolved_finding_ids,
        })
    }

    async fn invoke_acceptance_role(
        &self,
        request: &AcceptanceWorkflowRequest,
        version: u64,
        owner: ProcessId,
        role: CognitiveRole,
        write_scope: Option<Vec<String>>,
    ) -> anyhow::Result<PreparedRole> {
        let mut role_scope = match (role.can_write_workspace(), write_scope) {
            (false, None) => Vec::new(),
            (false, Some(_)) => anyhow::bail!("read-only role received a write scope"),
            (true, Some(scope)) if !scope.is_empty() => scope,
            (true, _) => anyhow::bail!("writable role received no bounded write scope"),
        };
        anyhow::ensure!(
            role_scope
                .iter()
                .all(|path| path_is_within_roots(path, &request.workspace_scope)),
            "role write scope exceeds the owned task scope"
        );
        role_scope.sort();
        role_scope.dedup();
        let profile = CognitiveRoleProfile::canonical(role);
        let projection = self
            .workspace
            .project_role(
                request.space.clone(),
                request.task_node_id.clone(),
                role,
                profile.context_projection.max_artifacts,
                Vec::new(),
            )
            .await?;
        anyhow::ensure!(
            projection.workspace_version == version,
            "acceptance projection became stale before role launch"
        );
        let mut task = projection.task.clone();
        task.role = role;
        task.role_profile = profile.reference.clone();
        task.budget = profile.budget.clone();
        task.workspace_scope = role_scope.clone();
        let packet = AgentTaskPacket {
            schema_version: 1,
            task,
            role_profile: profile.clone(),
            project_instructions: request.project_instructions.clone(),
            workspace_roots: role_scope.clone(),
            allowed_capabilities: request.allowed_capabilities.clone(),
            expected_evidence: request.expected_evidence.clone(),
            acceptance_criteria: projection.task.acceptance_criteria.clone(),
            selected_artifacts: projection.artifacts,
            projection_receipt: projection.receipt,
        };
        packet.validate()?;
        let terminal = self
            .invoker
            .invoke(
                packet.clone(),
                CognitiveTaskRuntimeBinding {
                    space: request.space.clone(),
                    task_node_id: request.task_node_id.clone(),
                    expected_workspace_version: version,
                    expected_owner: owner,
                    role,
                    role_profile: profile.reference,
                    budget: profile.budget,
                    workspace_scope: role_scope,
                },
            )
            .await?;
        terminal.output.validate_for(&packet)?;
        let envelope = CognitiveArtifactEnvelope::proposed(
            request.space.clone(),
            request.task_node_id.clone(),
            terminal.process_id,
            terminal.output.source_versions.clone(),
            terminal.output.evidence_refs.clone(),
            terminal.output.confidence,
            terminal.output.artifact.clone(),
        )?;
        Ok(PreparedRole {
            packet,
            terminal,
            envelope,
            bound_version: version.saturating_add(1),
        })
    }

    async fn set_unresolved_findings(
        &self,
        request: &AcceptanceWorkflowRequest,
        version: u64,
        owner: ProcessId,
        finding_ids: Vec<String>,
    ) -> anyhow::Result<u64> {
        let projection = self
            .workspace
            .project_role(
                request.space.clone(),
                request.task_node_id.clone(),
                CognitiveRole::Root,
                0,
                Vec::new(),
            )
            .await?;
        anyhow::ensure!(
            projection.workspace_version == version,
            "finding update projection is stale"
        );
        let mut task = projection.task;
        task.unresolved_finding_ids = finding_ids;
        self.workspace
            .commit_task_at(request.space.clone(), version, task, owner)
            .await
            .map_err(workspace_error)
    }

    #[allow(clippy::too_many_arguments)]
    async fn record_decision(
        &self,
        request: &AcceptanceWorkflowRequest,
        version: u64,
        owner: ProcessId,
        decision: StageDecisionKind,
        reason: &str,
        finding_ids: Vec<String>,
        evidence_refs: Vec<String>,
    ) -> anyhow::Result<u64> {
        self.workspace
            .record_stage_decision_at(
                request.space.clone(),
                version,
                request.task_node_id.clone(),
                StageDecision {
                    decision,
                    reason: reason.into(),
                    finding_ids,
                    evidence_refs,
                },
                owner,
            )
            .await
            .map_err(workspace_error)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::cognitive_workflow::*;
    use tokio::sync::Mutex;

    struct ScriptedInvoker {
        workspace: Arc<CognitiveWorkspaceCoordinator>,
        roles: Mutex<Vec<CognitiveRole>>,
        incomplete_plan: bool,
    }

    struct AcceptanceInvoker {
        workspace: Arc<CognitiveWorkspaceCoordinator>,
        roles: Mutex<Vec<CognitiveRole>>,
        bindings: Mutex<Vec<CognitiveTaskRuntimeBinding>>,
        reject_first_review: bool,
        fail_first_validation: bool,
        review_finding_paths: Vec<String>,
        rereview_finding_paths: Option<Vec<String>>,
        fixer_changed_paths: Vec<String>,
        review_calls: Mutex<usize>,
        validation_calls: Mutex<usize>,
        repair_calls: Mutex<usize>,
    }

    #[async_trait]
    impl CognitiveRoleInvoker for AcceptanceInvoker {
        async fn invoke(
            &self,
            packet: AgentTaskPacket,
            binding: CognitiveTaskRuntimeBinding,
        ) -> anyhow::Result<RoleInvocationTerminal> {
            let process_id = ProcessId::new();
            self.bindings.lock().await.push(binding.clone());
            crate::application::agent_control::CognitiveTaskAdmissionPort::bind_before_launch(
                self.workspace.as_ref(),
                binding,
                process_id,
            )
            .await
            .map_err(anyhow::Error::new)?;
            let role = packet.role_profile.role;
            self.roles.lock().await.push(role);
            let change = packet
                .selected_artifacts
                .iter()
                .rev()
                .find_map(|artifact| match &artifact.artifact {
                    CognitiveArtifact::ChangeSet(change) => Some(change.clone()),
                    _ => None,
                })
                .ok_or_else(|| anyhow::anyhow!("scripted acceptance role has no change"))?;
            let (artifact, evidence_refs) = match role {
                CognitiveRole::Tester => {
                    let mut calls = self.validation_calls.lock().await;
                    *calls += 1;
                    let passed = !(self.fail_first_validation && *calls == 1);
                    (
                        CognitiveArtifact::Validation(CognitiveValidationRecord {
                            transaction_id: change.transaction_id,
                            workspace_version: change.workspace_version,
                            validation_receipt_refs: vec![format!("receipt://validation/{calls}")],
                            passed,
                        }),
                        vec![format!("receipt://validation/{calls}")],
                    )
                }
                CognitiveRole::Reviewer => {
                    let mut calls = self.review_calls.lock().await;
                    *calls += 1;
                    let findings = if self.reject_first_review && *calls == 1 {
                        vec![ReviewFinding {
                            id: "review-1".into(),
                            severity: "high".into(),
                            summary: "missing boundary validation".into(),
                            evidence_refs: vec!["artifact://diff/line-1".into()],
                            affected_paths: self.review_finding_paths.clone(),
                            resolved: false,
                        }]
                    } else {
                        packet
                            .task
                            .unresolved_finding_ids
                            .iter()
                            .map(|id| ReviewFinding {
                                id: id.clone(),
                                severity: "high".into(),
                                summary: "verified repaired boundary".into(),
                                evidence_refs: vec!["receipt://rereview".into()],
                                affected_paths: self
                                    .rereview_finding_paths
                                    .clone()
                                    .unwrap_or_else(|| self.review_finding_paths.clone()),
                                resolved: true,
                            })
                            .collect()
                    };
                    (
                        CognitiveArtifact::Review(ReviewFindingSet {
                            transaction_id: change.transaction_id,
                            workspace_version: change.workspace_version,
                            findings,
                        }),
                        vec!["receipt://review".into()],
                    )
                }
                CognitiveRole::Fixer => {
                    let mut calls = self.repair_calls.lock().await;
                    *calls += 1;
                    let evidence_refs = packet
                        .task
                        .unresolved_finding_ids
                        .iter()
                        .map(|id| format!("finding:{id}"))
                        .collect();
                    (
                        CognitiveArtifact::ChangeSet(CognitiveChangeSetReceipt {
                            transaction_id: format!("repair-{calls}"),
                            workspace_version: format!("tree-repair-{calls}"),
                            changed_paths: self.fixer_changed_paths.clone(),
                            diff_artifact_ref: format!("artifact://repair/{calls}"),
                        }),
                        evidence_refs,
                    )
                }
                role => anyhow::bail!("unexpected acceptance role {role:?}"),
            };
            Ok(RoleInvocationTerminal {
                process_id,
                operation_id: OperationId::new(),
                output: CognitiveRoleOutput {
                    schema_version: 1,
                    projection_id: packet.projection_receipt.projection_id,
                    workspace_version: packet.projection_receipt.workspace_version,
                    source_versions: vec![format!(
                        "agora:{}",
                        packet.projection_receipt.workspace_version
                    )],
                    evidence_refs,
                    confidence: 1.0,
                    artifact,
                },
            })
        }
    }

    #[async_trait]
    impl CognitiveRoleInvoker for ScriptedInvoker {
        async fn invoke(
            &self,
            packet: AgentTaskPacket,
            binding: CognitiveTaskRuntimeBinding,
        ) -> anyhow::Result<RoleInvocationTerminal> {
            let process_id = ProcessId::new();
            crate::application::agent_control::CognitiveTaskAdmissionPort::bind_before_launch(
                self.workspace.as_ref(),
                binding,
                process_id,
            )
            .await
            .map_err(anyhow::Error::new)?;
            self.roles.lock().await.push(packet.role_profile.role);
            let artifact = match packet.role_profile.role {
                CognitiveRole::Planner => CognitiveArtifact::Plan(CognitivePlanArtifact {
                    steps: vec![CognitivePlanArtifactStep {
                        id: "implement".into(),
                        description: "implement the bounded requirement".into(),
                        requirement_refs: if self.incomplete_plan {
                            Vec::new()
                        } else {
                            vec!["spec:req-1".into()]
                        },
                        dependencies: Vec::new(),
                    }],
                }),
                CognitiveRole::Explorer => CognitiveArtifact::Investigation(InvestigationReport {
                    summary: "grounded call chain".into(),
                    findings: vec![GroundedFinding {
                        id: "finding-1".into(),
                        claim: "the affected symbol is production wired".into(),
                        evidence_refs: vec!["artifact://repo/read-1".into()],
                        confidence: 0.9,
                    }],
                }),
                CognitiveRole::Executor => {
                    CognitiveArtifact::ChangeSet(CognitiveChangeSetReceipt {
                        transaction_id: "tx-1".into(),
                        workspace_version: "tree-v2".into(),
                        changed_paths: vec!["crates/a/src/lib.rs".into()],
                        diff_artifact_ref: "artifact://diff/1".into(),
                    })
                }
                role => anyhow::bail!("unexpected scripted role {role:?}"),
            };
            Ok(RoleInvocationTerminal {
                process_id,
                operation_id: OperationId::new(),
                output: CognitiveRoleOutput {
                    schema_version: 1,
                    projection_id: packet.projection_receipt.projection_id,
                    workspace_version: packet.projection_receipt.workspace_version,
                    source_versions: vec![format!(
                        "agora:{}",
                        packet.projection_receipt.workspace_version
                    )],
                    evidence_refs: vec!["receipt://terminal".into()],
                    confidence: 1.0,
                    artifact,
                },
            })
        }
    }

    async fn fixture(
        incomplete_plan: bool,
    ) -> (
        CognitiveRoleWorkflow,
        Arc<CognitiveWorkspaceCoordinator>,
        Arc<ScriptedInvoker>,
        ProcessId,
    ) {
        let service: Arc<dyn fabric::AgoraService> = Arc::new(agora::AgoraRegistry::new(Arc::new(
            kernel::chronos::TestClock::default(),
        )));
        let workspace = Arc::new(CognitiveWorkspaceCoordinator::new(service));
        let owner = ProcessId::new();
        let root_profile = CognitiveRoleProfile::canonical(CognitiveRole::Root);
        workspace
            .commit_task_at(
                AgoraSpaceId("workflow".into()),
                0,
                CognitiveTaskNode {
                    id: CognitiveTaskNodeId("coding".into()),
                    parent_id: None,
                    objective: "implement one bounded requirement".into(),
                    role: CognitiveRole::Root,
                    stage: CognitiveStage::Contract,
                    status: CognitiveTaskStatus::Running,
                    owner: Some(owner),
                    role_profile: root_profile.reference,
                    budget: root_profile.budget,
                    dependencies: Vec::new(),
                    acceptance_criteria: vec!["focused test passes".into()],
                    workspace_scope: vec!["crates/a".into()],
                    required_artifact_kinds: vec![CognitiveArtifactKind::TaskContract],
                    artifact_refs: Vec::new(),
                    unresolved_finding_ids: Vec::new(),
                },
                owner,
            )
            .await
            .unwrap();
        let contract = CognitiveArtifactEnvelope::proposed(
            AgoraSpaceId("workflow".into()),
            CognitiveTaskNodeId("coding".into()),
            owner,
            vec!["requirements:v1".into()],
            Vec::new(),
            1.0,
            CognitiveArtifact::TaskContract(CognitiveTaskContractArtifact {
                objective: "implement one bounded requirement".into(),
                requirement_refs: vec!["spec:req-1".into()],
                acceptance_criteria: vec!["focused test passes".into()],
                instruction_refs: vec!["AGENTS.md".into()],
                workspace_scope: vec!["crates/a".into()],
            }),
        )
        .unwrap();
        workspace
            .commit_artifact_at(AgoraSpaceId("workflow".into()), 1, contract, owner)
            .await
            .unwrap();
        let invoker = Arc::new(ScriptedInvoker {
            workspace: workspace.clone(),
            roles: Mutex::new(Vec::new()),
            incomplete_plan,
        });
        (
            CognitiveRoleWorkflow::new(workspace.clone(), invoker.clone()),
            workspace,
            invoker,
            owner,
        )
    }

    fn request(owner: ProcessId) -> CodingWorkflowRequest {
        CodingWorkflowRequest {
            space: AgoraSpaceId("workflow".into()),
            task_node_id: CognitiveTaskNodeId("coding".into()),
            expected_workspace_version: 2,
            current_owner: owner,
            workspace_scope: vec!["crates/a".into()],
            project_instructions: vec!["AGENTS.md".into()],
            allowed_capabilities: vec![
                fabric::AgentRuntimeCapability::CodeRead,
                fabric::AgentRuntimeCapability::CodeEdit,
                fabric::AgentRuntimeCapability::Test,
            ],
            expected_evidence: vec!["terminal validation receipt".into()],
            risk_level: fabric::types::admission::RiskLevel::SystemModify,
        }
    }

    fn acceptance_request(receipt: &CodingWorkflowReceipt) -> AcceptanceWorkflowRequest {
        AcceptanceWorkflowRequest {
            space: AgoraSpaceId("workflow".into()),
            task_node_id: CognitiveTaskNodeId("coding".into()),
            expected_workspace_version: receipt.workspace_version,
            current_owner: receipt.current_owner,
            workspace_scope: vec!["crates/a".into()],
            project_instructions: vec!["AGENTS.md".into()],
            allowed_capabilities: vec![fabric::AgentRuntimeCapability::Test],
            expected_evidence: vec!["terminal validation receipt".into()],
            risk_level: fabric::types::admission::RiskLevel::SystemModify,
        }
    }

    #[tokio::test]
    async fn typed_planner_explorer_executor_handoffs_pass_each_gate() {
        let (workflow, workspace, invoker, owner) = fixture(false).await;
        let receipt = workflow
            .run_planner_explorer_executor(request(owner))
            .await
            .unwrap();
        assert_eq!(receipt.workspace_version, 11);
        assert_eq!(receipt.artifact_ids.len(), 3);
        assert_eq!(receipt.operation_ids.len(), 3);
        assert_eq!(
            *invoker.roles.lock().await,
            vec![
                CognitiveRole::Planner,
                CognitiveRole::Explorer,
                CognitiveRole::Executor
            ]
        );
        let projection = workspace
            .project_role(
                AgoraSpaceId("workflow".into()),
                CognitiveTaskNodeId("coding".into()),
                CognitiveRole::Root,
                32,
                Vec::new(),
            )
            .await
            .unwrap();
        assert_eq!(projection.workspace_version, 11);
        assert_eq!(projection.task.role, CognitiveRole::Executor);
        assert_eq!(projection.task.owner, Some(receipt.current_owner));
        assert_eq!(projection.artifacts.len(), 4);
    }

    #[tokio::test]
    async fn incomplete_requirement_map_is_durably_rejected_before_exploration() {
        let (workflow, workspace, invoker, owner) = fixture(true).await;
        let error = workflow
            .run_planner_explorer_executor(request(owner))
            .await
            .unwrap_err();
        assert!(error.to_string().contains("requirement mappings"));
        assert_eq!(*invoker.roles.lock().await, vec![CognitiveRole::Planner]);
        let projection = workspace
            .project_role(
                AgoraSpaceId("workflow".into()),
                CognitiveTaskNodeId("coding".into()),
                CognitiveRole::Root,
                32,
                Vec::new(),
            )
            .await
            .unwrap();
        // Bind and typed rejection commit; no invalid plan artifact is shared.
        assert_eq!(projection.workspace_version, 4);
        assert_eq!(projection.artifacts.len(), 1);
        assert_eq!(projection.task.role, CognitiveRole::Planner);
    }

    #[tokio::test]
    async fn reviewer_rejection_repairs_retests_and_preserves_finding_identity() {
        let (coding, workspace, _, owner) = fixture(false).await;
        let coding_receipt = coding
            .run_planner_explorer_executor(request(owner))
            .await
            .unwrap();
        let invoker = Arc::new(AcceptanceInvoker {
            workspace: workspace.clone(),
            roles: Mutex::new(Vec::new()),
            bindings: Mutex::new(Vec::new()),
            reject_first_review: true,
            fail_first_validation: false,
            review_finding_paths: vec!["crates/a/src/lib.rs".into()],
            rereview_finding_paths: None,
            fixer_changed_paths: vec!["crates/a/src/lib.rs".into()],
            review_calls: Mutex::new(0),
            validation_calls: Mutex::new(0),
            repair_calls: Mutex::new(0),
        });
        let acceptance = CognitiveRoleWorkflow::new(workspace.clone(), invoker.clone());
        let receipt = acceptance
            .run_reviewer_tester_fixer(acceptance_request(&coding_receipt))
            .await
            .unwrap();
        assert_eq!(receipt.workspace_version, 28);
        assert_eq!(receipt.resolved_finding_ids, vec!["review-1"]);
        assert_eq!(receipt.validation_artifact_ids.len(), 2);
        assert_eq!(receipt.review_artifact_ids.len(), 2);
        assert_eq!(receipt.repair_artifact_ids.len(), 1);
        assert_eq!(
            *invoker.roles.lock().await,
            vec![
                CognitiveRole::Tester,
                CognitiveRole::Reviewer,
                CognitiveRole::Fixer,
                CognitiveRole::Tester,
                CognitiveRole::Reviewer,
            ]
        );
        let bindings = invoker.bindings.lock().await;
        assert!(bindings
            .iter()
            .filter(|binding| !binding.role.can_write_workspace())
            .all(|binding| binding.workspace_scope.is_empty()));
        let fixer_scopes = bindings
            .iter()
            .filter(|binding| binding.role == CognitiveRole::Fixer)
            .map(|binding| binding.workspace_scope.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            fixer_scopes,
            vec![vec![String::from("crates/a/src/lib.rs")]]
        );
        drop(bindings);
        let projection = workspace
            .project_role(
                AgoraSpaceId("workflow".into()),
                CognitiveTaskNodeId("coding".into()),
                CognitiveRole::Root,
                32,
                Vec::new(),
            )
            .await
            .unwrap();
        assert!(projection.task.unresolved_finding_ids.is_empty());
    }

    #[tokio::test]
    async fn failed_validation_schedules_one_bounded_fix_and_authoritative_retest() {
        let (coding, workspace, _, owner) = fixture(false).await;
        let coding_receipt = coding
            .run_planner_explorer_executor(request(owner))
            .await
            .unwrap();
        let invoker = Arc::new(AcceptanceInvoker {
            workspace: workspace.clone(),
            roles: Mutex::new(Vec::new()),
            bindings: Mutex::new(Vec::new()),
            reject_first_review: false,
            fail_first_validation: true,
            review_finding_paths: vec!["crates/a/src/lib.rs".into()],
            rereview_finding_paths: None,
            fixer_changed_paths: vec!["crates/a/src/lib.rs".into()],
            review_calls: Mutex::new(0),
            validation_calls: Mutex::new(0),
            repair_calls: Mutex::new(0),
        });
        let acceptance = CognitiveRoleWorkflow::new(workspace, invoker.clone());
        let receipt = acceptance
            .run_reviewer_tester_fixer(acceptance_request(&coding_receipt))
            .await
            .unwrap();
        assert_eq!(receipt.validation_artifact_ids.len(), 2);
        assert_eq!(receipt.repair_artifact_ids.len(), 1);
        assert_eq!(receipt.review_artifact_ids.len(), 1);
        assert_eq!(receipt.resolved_finding_ids.len(), 1);
        assert_eq!(
            *invoker.roles.lock().await,
            vec![
                CognitiveRole::Tester,
                CognitiveRole::Fixer,
                CognitiveRole::Tester,
                CognitiveRole::Reviewer,
            ]
        );
        let fixer_scopes = invoker
            .bindings
            .lock()
            .await
            .iter()
            .filter(|binding| binding.role == CognitiveRole::Fixer)
            .map(|binding| binding.workspace_scope.clone())
            .collect::<Vec<_>>();
        assert_eq!(
            fixer_scopes,
            vec![vec![String::from("crates/a/src/lib.rs")]]
        );
    }

    #[tokio::test]
    async fn unresolved_review_finding_requires_admitted_paths() {
        let (coding, workspace, _, owner) = fixture(false).await;
        let coding_receipt = coding
            .run_planner_explorer_executor(request(owner))
            .await
            .unwrap();
        let invoker = Arc::new(AcceptanceInvoker {
            workspace: workspace.clone(),
            roles: Mutex::new(Vec::new()),
            bindings: Mutex::new(Vec::new()),
            reject_first_review: true,
            fail_first_validation: false,
            review_finding_paths: Vec::new(),
            rereview_finding_paths: None,
            fixer_changed_paths: vec!["crates/a/src/lib.rs".into()],
            review_calls: Mutex::new(0),
            validation_calls: Mutex::new(0),
            repair_calls: Mutex::new(0),
        });

        let error = CognitiveRoleWorkflow::new(workspace, invoker.clone())
            .run_reviewer_tester_fixer(acceptance_request(&coding_receipt))
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("invalid, duplicate, or out-of-scope typed findings"));
        assert_eq!(*invoker.repair_calls.lock().await, 0);
    }

    #[tokio::test]
    async fn review_finding_path_must_stay_inside_task_scope() {
        let (coding, workspace, _, owner) = fixture(false).await;
        let coding_receipt = coding
            .run_planner_explorer_executor(request(owner))
            .await
            .unwrap();
        let invoker = Arc::new(AcceptanceInvoker {
            workspace: workspace.clone(),
            roles: Mutex::new(Vec::new()),
            bindings: Mutex::new(Vec::new()),
            reject_first_review: true,
            fail_first_validation: false,
            review_finding_paths: vec!["crates/b/src/lib.rs".into()],
            rereview_finding_paths: None,
            fixer_changed_paths: vec!["crates/a/src/lib.rs".into()],
            review_calls: Mutex::new(0),
            validation_calls: Mutex::new(0),
            repair_calls: Mutex::new(0),
        });

        let error = CognitiveRoleWorkflow::new(workspace, invoker.clone())
            .run_reviewer_tester_fixer(acceptance_request(&coding_receipt))
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("invalid, duplicate, or out-of-scope typed findings"));
        assert_eq!(*invoker.repair_calls.lock().await, 0);
    }

    #[tokio::test]
    async fn fixer_change_set_must_stay_inside_finding_scope() {
        let (coding, workspace, _, owner) = fixture(false).await;
        let coding_receipt = coding
            .run_planner_explorer_executor(request(owner))
            .await
            .unwrap();
        let invoker = Arc::new(AcceptanceInvoker {
            workspace: workspace.clone(),
            roles: Mutex::new(Vec::new()),
            bindings: Mutex::new(Vec::new()),
            reject_first_review: true,
            fail_first_validation: false,
            review_finding_paths: vec!["crates/a/src/lib.rs".into()],
            rereview_finding_paths: None,
            fixer_changed_paths: vec!["crates/a/src/sibling.rs".into()],
            review_calls: Mutex::new(0),
            validation_calls: Mutex::new(0),
            repair_calls: Mutex::new(0),
        });

        let error = CognitiveRoleWorkflow::new(workspace, invoker.clone())
            .run_reviewer_tester_fixer(acceptance_request(&coding_receipt))
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("fixer changed a path outside the finding scope"));
        assert_eq!(*invoker.repair_calls.lock().await, 1);
        let fixer = invoker
            .bindings
            .lock()
            .await
            .iter()
            .find(|binding| binding.role == CognitiveRole::Fixer)
            .cloned()
            .unwrap();
        assert_eq!(fixer.workspace_scope, vec!["crates/a/src/lib.rs"]);
    }

    #[tokio::test]
    async fn rereview_cannot_widen_preserved_finding_paths() {
        let (coding, workspace, _, owner) = fixture(false).await;
        let coding_receipt = coding
            .run_planner_explorer_executor(request(owner))
            .await
            .unwrap();
        let invoker = Arc::new(AcceptanceInvoker {
            workspace: workspace.clone(),
            roles: Mutex::new(Vec::new()),
            bindings: Mutex::new(Vec::new()),
            reject_first_review: true,
            fail_first_validation: false,
            review_finding_paths: vec!["crates/a/src/lib.rs".into()],
            rereview_finding_paths: Some(vec!["crates/a/src/sibling.rs".into()]),
            fixer_changed_paths: vec!["crates/a/src/lib.rs".into()],
            review_calls: Mutex::new(0),
            validation_calls: Mutex::new(0),
            repair_calls: Mutex::new(0),
        });

        let error = CognitiveRoleWorkflow::new(workspace, invoker)
            .run_reviewer_tester_fixer(acceptance_request(&coding_receipt))
            .await
            .unwrap_err();

        assert!(error
            .to_string()
            .contains("re-review changed preserved finding paths"));
    }

    #[tokio::test]
    async fn low_risk_task_with_no_expected_evidence_skips_planner_and_explorer() {
        let (workflow, _workspace, invoker, owner) = fixture(false).await;
        let mut low_risk = request(owner);
        low_risk.risk_level = fabric::types::admission::RiskLevel::ReadOnly;
        low_risk.expected_evidence.clear();
        let receipt = workflow
            .run_planner_explorer_executor(low_risk)
            .await
            .unwrap();
        assert_eq!(
            *invoker.roles.lock().await,
            vec![CognitiveRole::Executor],
            "a low-risk task with no expected evidence must skip Planner and Explorer"
        );
        assert_eq!(receipt.operation_ids.len(), 1);
    }

    #[tokio::test]
    async fn low_risk_task_with_required_evidence_needs_explorer_then_executor() {
        let (workflow, _workspace, invoker, owner) = fixture(false).await;
        let mut low_risk = request(owner);
        low_risk.risk_level = fabric::types::admission::RiskLevel::ReadOnly;
        // non-empty expected_evidence stays from request(owner);
        // evidence is required so Explorer must run before Executor
        let receipt = workflow
            .run_planner_explorer_executor(low_risk)
            .await
            .unwrap();
        assert_eq!(
            *invoker.roles.lock().await,
            vec![CognitiveRole::Explorer, CognitiveRole::Executor],
            "a low-risk task with required evidence must run Explorer then Executor"
        );
        assert_eq!(receipt.operation_ids.len(), 2);
    }
}
