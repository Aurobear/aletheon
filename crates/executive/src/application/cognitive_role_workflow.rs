//! Typed Planner → Explorer → Executor workflow over AgentControl and Agora.
//! Agent runtimes receive bounded task packets; only host-validated artifacts
//! and gate decisions advance the shared task.

use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use fabric::cognitive_workflow::{
    AgentTaskPacket, ArtifactLifecycle, CognitiveArtifact, CognitiveArtifactEnvelope,
    CognitiveRole, CognitiveRoleOutput, CognitiveRoleProfile, CognitiveTaskNodeId,
    CognitiveTaskRuntimeBinding, StageDecision, StageDecisionKind,
};
use fabric::{
    AgentBudget, AgentContextFork, AgentControlPort, AgentId, AgentProfileId, AgentRunStatus,
    AgentSpawnRequest, AgentWaitRequest, AgoraSpaceId, OperationId, ProcessId, RuntimeId,
    WorkspacePolicy,
};

use super::cognitive_workspace::{CognitiveWorkspaceCoordinator, CognitiveWorkspaceError};

#[derive(Debug, Clone)]
pub struct RoleLaunchProfile {
    pub profile_id: AgentProfileId,
    pub allowed_tools: Vec<String>,
}

#[derive(Clone)]
pub struct AgentControlRoleInvoker {
    control: Arc<dyn AgentControlPort>,
    root_agent_id: AgentId,
    parent_agent_id: Option<AgentId>,
    parent_process_id: Option<ProcessId>,
    runtime_id: RuntimeId,
    trusted_workspace: WorkspacePolicy,
    profiles: HashMap<CognitiveRole, RoleLaunchProfile>,
    wait_timeout_ms: u64,
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
        })
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
        let handle = self
            .control
            .spawn(AgentSpawnRequest {
                root_agent_id: self.root_agent_id,
                parent_agent_id: self.parent_agent_id,
                parent_process_id: self.parent_process_id,
                profile_id: launch.profile_id.clone(),
                runtime_id: self.runtime_id.clone(),
                trusted_workspace: Some(self.trusted_workspace.clone()),
                cognitive_binding: Some(binding),
                task,
                context: AgentContextFork::None,
                broadcast_refs: Vec::new(),
                allowed_tools: launch.allowed_tools.clone(),
                budget: AgentBudget {
                    max_input_tokens: budget.max_input_tokens,
                    max_output_tokens: budget.max_output_tokens,
                    max_tool_calls: budget.max_tool_calls,
                    max_elapsed_ms: budget.max_elapsed_ms,
                    max_cost_usd: None,
                    max_depth: 1,
                },
                background_decls: Vec::new(),
            })
            .await
            .map_err(anyhow::Error::new)?;
        let snapshot = self
            .control
            .wait(AgentWaitRequest {
                caller_root_agent_id: self.root_agent_id,
                agent_id: handle.agent_id,
                timeout_ms: self.wait_timeout_ms.min(budget.max_elapsed_ms),
            })
            .await
            .map_err(anyhow::Error::new)?;
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
}

#[derive(Debug, Clone)]
pub struct CodingWorkflowReceipt {
    pub workspace_version: u64,
    pub current_owner: ProcessId,
    pub artifact_ids: Vec<fabric::cognitive_workflow::CognitiveArtifactId>,
    pub operation_ids: Vec<OperationId>,
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
        for role in [
            CognitiveRole::Planner,
            CognitiveRole::Explorer,
            CognitiveRole::Executor,
        ] {
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
                            reason: gate_error.to_string(),
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
                        reason: format!("host gate accepted {role:?} artifact at exact version"),
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
}

fn workspace_error(error: CognitiveWorkspaceError) -> anyhow::Error {
    anyhow::Error::new(error)
}

fn validate_stage_artifact(
    role: CognitiveRole,
    packet: &AgentTaskPacket,
    envelope: &CognitiveArtifactEnvelope,
) -> anyhow::Result<()> {
    anyhow::ensure!(
        envelope.lifecycle == ArtifactLifecycle::Proposed,
        "stage artifact is not a proposal"
    );
    match (&role, &envelope.artifact) {
        (CognitiveRole::Planner, CognitiveArtifact::Plan(plan)) => {
            let contract = packet
                .selected_artifacts
                .iter()
                .find_map(|artifact| match &artifact.artifact {
                    CognitiveArtifact::TaskContract(contract) => Some(contract),
                    _ => None,
                })
                .ok_or_else(|| {
                    anyhow::anyhow!("planner gate requires a versioned task contract")
                })?;
            anyhow::ensure!(!plan.steps.is_empty(), "planner gate rejects an empty plan");
            let step_ids = plan
                .steps
                .iter()
                .map(|step| step.id.as_str())
                .collect::<std::collections::HashSet<_>>();
            anyhow::ensure!(
                plan.steps.iter().all(|step| !step.id.trim().is_empty()
                    && step
                        .dependencies
                        .iter()
                        .all(|dependency| step_ids.contains(dependency.as_str()))),
                "planner gate rejects invalid step identity or dependency"
            );
            anyhow::ensure!(
                contract.requirement_refs.iter().all(|requirement| plan
                    .steps
                    .iter()
                    .any(|step| step.requirement_refs.contains(requirement))),
                "planner gate rejects incomplete requirement mappings"
            );
        }
        (CognitiveRole::Explorer, CognitiveArtifact::Investigation(report)) => {
            anyhow::ensure!(
                !report.findings.is_empty(),
                "explorer gate requires grounded findings"
            );
            anyhow::ensure!(
                report
                    .findings
                    .iter()
                    .all(|finding| !finding.claim.trim().is_empty()
                        && !finding.evidence_refs.is_empty()
                        && finding.confidence.is_finite()
                        && (0.0..=1.0).contains(&finding.confidence)),
                "explorer gate rejects unsupported findings"
            );
        }
        (CognitiveRole::Executor, CognitiveArtifact::ChangeSet(change)) => {
            anyhow::ensure!(
                !change.transaction_id.trim().is_empty()
                    && !change.workspace_version.trim().is_empty()
                    && !change.diff_artifact_ref.trim().is_empty(),
                "executor gate requires an exact change transaction and diff artifact"
            );
            anyhow::ensure!(
                !change.changed_paths.is_empty(),
                "executor gate requires a material change set"
            );
            anyhow::ensure!(
                change
                    .changed_paths
                    .iter()
                    .all(|path| path_is_within_roots(path, &packet.workspace_roots)),
                "executor gate rejects a changed path outside the owned scope"
            );
        }
        _ => anyhow::bail!("role returned the wrong typed stage artifact"),
    }
    Ok(())
}

fn path_is_within_roots(path: &str, roots: &[String]) -> bool {
    use std::path::Component;
    let path = std::path::Path::new(path);
    if path.as_os_str().is_empty()
        || path
            .components()
            .any(|component| matches!(component, Component::ParentDir))
    {
        return false;
    }
    roots.iter().any(|root| {
        let root = std::path::Path::new(root);
        path.is_absolute() == root.is_absolute() && path.starts_with(root)
    })
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
}
