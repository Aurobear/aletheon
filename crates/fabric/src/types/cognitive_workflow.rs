//! Stable cross-crate contracts for versioned cognitive work in Agora.

use crate::{AgentRuntimeCapability, AgoraSpaceId, ProcessId};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CognitiveTaskNodeId(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveRole {
    Root,
    Planner,
    Explorer,
    Executor,
    Reviewer,
    Tester,
    Fixer,
}

impl CognitiveRole {
    pub const fn can_write_workspace(self) -> bool {
        matches!(self, Self::Executor | Self::Fixer)
    }

    pub const fn can_validate(self) -> bool {
        matches!(self, Self::Reviewer | Self::Tester)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CognitiveRoleProfileRef {
    pub id: String,
    pub version: u32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveRoleBudget {
    pub max_input_tokens: u64,
    pub max_output_tokens: u64,
    pub max_tool_calls: u32,
    pub max_elapsed_ms: u64,
}

impl CognitiveRoleBudget {
    pub fn fits_within(&self, parent: &Self) -> bool {
        self.max_input_tokens <= parent.max_input_tokens
            && self.max_output_tokens <= parent.max_output_tokens
            && self.max_tool_calls <= parent.max_tool_calls
            && self.max_elapsed_ms <= parent.max_elapsed_ms
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.max_input_tokens > 0
                && self.max_output_tokens > 0
                && self.max_tool_calls > 0
                && self.max_elapsed_ms > 0,
            "cognitive role budget limits must be nonzero"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ValidationAuthority {
    None,
    Recommend,
    IndependentGate,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContextProjectionPolicy {
    pub accepted_kinds: Vec<CognitiveArtifactKind>,
    pub max_artifacts: usize,
    pub include_peer_history: bool,
}

/// Host-enforced role behavior. The prompt may explain this contract but is
/// never its source of authority.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveRoleProfile {
    pub reference: CognitiveRoleProfileRef,
    pub role: CognitiveRole,
    pub responsibilities: Vec<String>,
    pub prohibited_actions: Vec<String>,
    pub accepted_input_artifacts: Vec<CognitiveArtifactKind>,
    pub required_output_artifacts: Vec<CognitiveArtifactKind>,
    pub workspace_write_allowed: bool,
    pub validation_authority: ValidationAuthority,
    pub context_projection: ContextProjectionPolicy,
    pub budget: CognitiveRoleBudget,
    pub maximum_concurrency: u32,
    pub escalation_behavior: String,
    pub completion_conditions: Vec<String>,
}

impl CognitiveRoleProfile {
    pub fn canonical(role: CognitiveRole) -> Self {
        let (inputs, outputs, write, validation, responsibilities, prohibited) = match role {
            CognitiveRole::Root => (
                vec![
                    CognitiveArtifactKind::TaskContract,
                    CognitiveArtifactKind::Plan,
                    CognitiveArtifactKind::Investigation,
                    CognitiveArtifactKind::ChangeSet,
                    CognitiveArtifactKind::Validation,
                    CognitiveArtifactKind::Review,
                    CognitiveArtifactKind::Evidence,
                    CognitiveArtifactKind::Decision,
                    CognitiveArtifactKind::AgentResult,
                ],
                vec![
                    CognitiveArtifactKind::TaskContract,
                    CognitiveArtifactKind::Decision,
                ],
                false,
                ValidationAuthority::IndependentGate,
                vec!["own the root contract and stage gates"],
                vec!["infer success from prose"],
            ),
            CognitiveRole::Planner => (
                vec![
                    CognitiveArtifactKind::TaskContract,
                    CognitiveArtifactKind::Evidence,
                ],
                vec![CognitiveArtifactKind::Plan],
                false,
                ValidationAuthority::None,
                vec!["map every requirement to an executable plan step"],
                vec![
                    "modify the workspace",
                    "accept an incomplete requirement map",
                ],
            ),
            CognitiveRole::Explorer => (
                vec![
                    CognitiveArtifactKind::TaskContract,
                    CognitiveArtifactKind::Plan,
                ],
                vec![
                    CognitiveArtifactKind::Investigation,
                    CognitiveArtifactKind::Evidence,
                ],
                false,
                ValidationAuthority::None,
                vec!["collect grounded repository evidence"],
                vec!["modify the workspace", "report uninspected claims as facts"],
            ),
            CognitiveRole::Executor => (
                vec![
                    CognitiveArtifactKind::TaskContract,
                    CognitiveArtifactKind::Plan,
                    CognitiveArtifactKind::Investigation,
                ],
                vec![CognitiveArtifactKind::ChangeSet],
                true,
                ValidationAuthority::None,
                vec!["apply the accepted change within the owned scope"],
                vec!["write outside the owned scope", "certify its own changes"],
            ),
            CognitiveRole::Reviewer => (
                vec![
                    CognitiveArtifactKind::TaskContract,
                    CognitiveArtifactKind::Plan,
                    CognitiveArtifactKind::ChangeSet,
                    CognitiveArtifactKind::Validation,
                ],
                vec![CognitiveArtifactKind::Review],
                false,
                ValidationAuthority::IndependentGate,
                vec!["review exact requirement, diff, and validation versions"],
                vec!["modify the workspace", "certify its own changes"],
            ),
            CognitiveRole::Tester => (
                vec![
                    CognitiveArtifactKind::TaskContract,
                    CognitiveArtifactKind::ChangeSet,
                    CognitiveArtifactKind::Review,
                    CognitiveArtifactKind::Evidence,
                ],
                vec![CognitiveArtifactKind::Validation],
                false,
                ValidationAuthority::IndependentGate,
                vec!["produce authoritative terminal validation evidence"],
                vec![
                    "modify production sources",
                    "summarize success without a terminal receipt",
                ],
            ),
            CognitiveRole::Fixer => (
                vec![
                    CognitiveArtifactKind::TaskContract,
                    CognitiveArtifactKind::ChangeSet,
                    CognitiveArtifactKind::Review,
                    CognitiveArtifactKind::Validation,
                ],
                vec![CognitiveArtifactKind::ChangeSet],
                true,
                ValidationAuthority::None,
                vec!["repair the identified findings within the owned scope"],
                vec![
                    "write outside the owned scope",
                    "discard finding identities",
                ],
            ),
        };
        let name = format!("{:?}", role).to_ascii_lowercase();
        Self {
            reference: CognitiveRoleProfileRef {
                id: format!("aletheon.cognitive.{name}"),
                version: 1,
            },
            role,
            responsibilities: responsibilities.into_iter().map(str::to_owned).collect(),
            prohibited_actions: prohibited.into_iter().map(str::to_owned).collect(),
            accepted_input_artifacts: inputs.clone(),
            required_output_artifacts: outputs,
            workspace_write_allowed: write,
            validation_authority: validation,
            context_projection: ContextProjectionPolicy {
                accepted_kinds: inputs,
                max_artifacts: 32,
                include_peer_history: false,
            },
            budget: CognitiveRoleBudget {
                max_input_tokens: 131_072,
                max_output_tokens: 32_768,
                max_tool_calls: 256,
                max_elapsed_ms: 3_600_000,
            },
            maximum_concurrency: 1,
            escalation_behavior: "return a typed blocked or rejected decision to the owning parent"
                .into(),
            completion_conditions: vec![
                "all required output artifacts are committed at the current workspace version"
                    .into(),
            ],
        }
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.reference.version > 0 && !self.reference.id.trim().is_empty(),
            "cognitive role profile reference is invalid"
        );
        anyhow::ensure!(
            self.maximum_concurrency > 0,
            "role maximum concurrency must be nonzero"
        );
        anyhow::ensure!(
            self.workspace_write_allowed == self.role.can_write_workspace(),
            "role write authority does not match its host policy"
        );
        anyhow::ensure!(
            !self.context_projection.include_peer_history,
            "peer history cannot be projected by default"
        );
        self.budget.validate()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveStage {
    Contract,
    Planning,
    Investigation,
    Execution,
    Validation,
    Review,
    Decision,
    Complete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveTaskStatus {
    Pending,
    Running,
    Blocked,
    Suspended,
    Completed,
    Failed,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveTaskNode {
    pub id: CognitiveTaskNodeId,
    pub parent_id: Option<CognitiveTaskNodeId>,
    pub objective: String,
    pub role: CognitiveRole,
    pub stage: CognitiveStage,
    pub status: CognitiveTaskStatus,
    pub owner: Option<ProcessId>,
    pub role_profile: CognitiveRoleProfileRef,
    pub budget: CognitiveRoleBudget,
    pub dependencies: Vec<CognitiveTaskNodeId>,
    pub acceptance_criteria: Vec<String>,
    pub workspace_scope: Vec<String>,
    pub required_artifact_kinds: Vec<CognitiveArtifactKind>,
    pub artifact_refs: Vec<CognitiveArtifactId>,
    pub unresolved_finding_ids: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveArtifactKind {
    TaskContract,
    Plan,
    Investigation,
    ChangeSet,
    Validation,
    Review,
    Evidence,
    Decision,
    AgentResult,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ArtifactLifecycle {
    Proposed,
    Committed,
    Rejected,
    Superseded,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CognitiveArtifactId(pub Uuid);

impl CognitiveArtifactId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CognitiveArtifactId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveArtifactEnvelope {
    pub id: CognitiveArtifactId,
    pub space: AgoraSpaceId,
    pub task_node_id: CognitiveTaskNodeId,
    pub author: ProcessId,
    pub source_versions: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub confidence: f32,
    pub lifecycle: ArtifactLifecycle,
    pub artifact: CognitiveArtifact,
    pub content_digest: String,
}

impl CognitiveArtifactEnvelope {
    pub fn proposed(
        space: AgoraSpaceId,
        task_node_id: CognitiveTaskNodeId,
        author: ProcessId,
        source_versions: Vec<String>,
        evidence_refs: Vec<String>,
        confidence: f32,
        artifact: CognitiveArtifact,
    ) -> anyhow::Result<Self> {
        let content_digest = artifact.digest()?;
        Ok(Self {
            id: CognitiveArtifactId::new(),
            space,
            task_node_id,
            author,
            source_versions,
            evidence_refs,
            confidence,
            lifecycle: ArtifactLifecycle::Proposed,
            artifact,
            content_digest,
        })
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.content_digest == self.artifact.digest()?,
            "cognitive artifact digest mismatch"
        );
        anyhow::ensure!(
            self.confidence.is_finite() && (0.0..=1.0).contains(&self.confidence),
            "cognitive artifact confidence is invalid"
        );
        anyhow::ensure!(
            !self.task_node_id.0.trim().is_empty(),
            "cognitive artifact task node is empty"
        );
        Ok(())
    }

    pub fn kind(&self) -> CognitiveArtifactKind {
        self.artifact.kind()
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "content", rename_all = "snake_case")]
pub enum CognitiveArtifact {
    TaskContract(CognitiveTaskContractArtifact),
    Plan(CognitivePlanArtifact),
    Investigation(InvestigationReport),
    ChangeSet(CognitiveChangeSetReceipt),
    Validation(CognitiveValidationRecord),
    Review(ReviewFindingSet),
    Evidence(CognitiveEvidenceRecord),
    Decision(StageDecision),
    AgentResult(AgentResultReceipt),
}

impl CognitiveArtifact {
    pub fn kind(&self) -> CognitiveArtifactKind {
        match self {
            Self::TaskContract(_) => CognitiveArtifactKind::TaskContract,
            Self::Plan(_) => CognitiveArtifactKind::Plan,
            Self::Investigation(_) => CognitiveArtifactKind::Investigation,
            Self::ChangeSet(_) => CognitiveArtifactKind::ChangeSet,
            Self::Validation(_) => CognitiveArtifactKind::Validation,
            Self::Review(_) => CognitiveArtifactKind::Review,
            Self::Evidence(_) => CognitiveArtifactKind::Evidence,
            Self::Decision(_) => CognitiveArtifactKind::Decision,
            Self::AgentResult(_) => CognitiveArtifactKind::AgentResult,
        }
    }

    pub fn digest(&self) -> anyhow::Result<String> {
        let bytes = serde_json::to_vec(self)?;
        Ok(Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveTaskContractArtifact {
    pub objective: String,
    pub requirement_refs: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub instruction_refs: Vec<String>,
    pub workspace_scope: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitivePlanArtifact {
    pub steps: Vec<CognitivePlanArtifactStep>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitivePlanArtifactStep {
    pub id: String,
    pub description: String,
    pub requirement_refs: Vec<String>,
    pub dependencies: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct InvestigationReport {
    pub summary: String,
    pub findings: Vec<GroundedFinding>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct GroundedFinding {
    pub id: String,
    pub claim: String,
    pub evidence_refs: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveChangeSetReceipt {
    pub transaction_id: String,
    pub workspace_version: String,
    pub changed_paths: Vec<String>,
    pub diff_artifact_ref: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveValidationRecord {
    pub transaction_id: String,
    pub workspace_version: String,
    pub validation_receipt_refs: Vec<String>,
    pub passed: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFindingSet {
    pub transaction_id: String,
    pub workspace_version: String,
    pub findings: Vec<ReviewFinding>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewFinding {
    pub id: String,
    pub severity: String,
    pub summary: String,
    pub evidence_refs: Vec<String>,
    pub resolved: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveEvidenceRecord {
    pub claim: String,
    pub evidence_refs: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum StageDecisionKind {
    Accept,
    Reject,
    Replan,
    Repair,
    Block,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct StageDecision {
    pub decision: StageDecisionKind,
    pub reason: String,
    pub finding_ids: Vec<String>,
    pub evidence_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgentResultReceipt {
    pub role: CognitiveRole,
    pub operation_id: String,
    pub terminal_status: String,
    pub output_ref: Option<String>,
    pub produced_artifact_refs: Vec<CognitiveArtifactId>,
    pub task_packet_digest: String,
    pub projection_receipt: AgoraProjectionReceipt,
}

/// Versioned, bounded work definition supplied to any native or external role
/// runtime. It carries selected artifacts, never another role's transcript.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgentTaskPacket {
    pub schema_version: u32,
    pub task: CognitiveTaskNode,
    pub role_profile: CognitiveRoleProfile,
    pub project_instructions: Vec<String>,
    pub workspace_roots: Vec<String>,
    pub allowed_capabilities: Vec<AgentRuntimeCapability>,
    pub expected_evidence: Vec<String>,
    pub acceptance_criteria: Vec<String>,
    pub selected_artifacts: Vec<CognitiveArtifactEnvelope>,
    pub projection_receipt: AgoraProjectionReceipt,
}

impl AgentTaskPacket {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == 1,
            "unsupported Agent task packet version"
        );
        self.role_profile.validate()?;
        anyhow::ensure!(
            self.task.role == self.role_profile.role,
            "task packet role mismatch"
        );
        anyhow::ensure!(
            self.task.role_profile == self.role_profile.reference,
            "task packet profile mismatch"
        );
        anyhow::ensure!(
            self.projection_receipt.task_node_id == self.task.id,
            "task packet projection mismatch"
        );
        anyhow::ensure!(
            self.projection_receipt.role == self.role_profile.role,
            "task packet projection role mismatch"
        );
        anyhow::ensure!(
            self.selected_artifacts.len() <= self.role_profile.context_projection.max_artifacts,
            "task packet artifact bound exceeded"
        );
        anyhow::ensure!(
            self.selected_artifacts.iter().all(|artifact| self
                .projection_receipt
                .included_artifact_ids
                .contains(&artifact.id)),
            "task packet contains an unreceipted artifact"
        );
        Ok(())
    }

    pub fn digest(&self) -> anyhow::Result<String> {
        self.validate()?;
        let bytes = serde_json::to_vec(self)?;
        Ok(Sha256::digest(bytes)
            .iter()
            .map(|byte| format!("{byte:02x}"))
            .collect())
    }
}

/// Strict response envelope parsed at the host boundary. Free-form prose can
/// accompany work inside typed fields, but cannot itself advance a stage.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CognitiveRoleOutput {
    pub schema_version: u32,
    pub projection_id: Uuid,
    pub workspace_version: u64,
    pub source_versions: Vec<String>,
    pub evidence_refs: Vec<String>,
    pub confidence: f32,
    pub artifact: CognitiveArtifact,
}

/// Host-only two-phase admission attached to an Agent spawn. AgentControl
/// binds the allocated process to this Agora task before launching the runtime.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveTaskRuntimeBinding {
    pub space: AgoraSpaceId,
    pub task_node_id: CognitiveTaskNodeId,
    pub expected_workspace_version: u64,
    pub expected_owner: ProcessId,
    pub role: CognitiveRole,
    pub role_profile: CognitiveRoleProfileRef,
    pub budget: CognitiveRoleBudget,
    pub workspace_scope: Vec<String>,
}

impl CognitiveRoleOutput {
    pub fn validate_for(&self, packet: &AgentTaskPacket) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == 1,
            "unsupported cognitive role output version"
        );
        anyhow::ensure!(
            self.projection_id == packet.projection_receipt.projection_id,
            "role output projection identity mismatch"
        );
        anyhow::ensure!(
            self.workspace_version == packet.projection_receipt.workspace_version,
            "role output workspace version mismatch"
        );
        anyhow::ensure!(
            self.source_versions
                .contains(&format!("agora:{}", self.workspace_version)),
            "role output is not bound to the projected Agora version"
        );
        anyhow::ensure!(
            packet
                .role_profile
                .required_output_artifacts
                .contains(&self.artifact.kind()),
            "role output kind is not authorized"
        );
        anyhow::ensure!(
            self.confidence.is_finite() && (0.0..=1.0).contains(&self.confidence),
            "role output confidence is invalid"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgoraProjectionRequest {
    pub space: AgoraSpaceId,
    pub task_node_id: CognitiveTaskNodeId,
    pub role: CognitiveRole,
    pub max_artifacts: usize,
    pub include_kinds: Vec<CognitiveArtifactKind>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AgoraTaskProjection {
    pub space: AgoraSpaceId,
    pub workspace_version: u64,
    pub task: CognitiveTaskNode,
    pub artifacts: Vec<CognitiveArtifactEnvelope>,
    pub omitted_artifact_ids: Vec<CognitiveArtifactId>,
    pub clarification: Option<ClarificationRecord>,
    pub interruption: Option<CognitiveInterruptionRecord>,
    pub receipt: AgoraProjectionReceipt,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgoraTaskList {
    pub space: AgoraSpaceId,
    pub workspace_version: u64,
    pub tasks: Vec<CognitiveTaskNode>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgoraProjectionReceipt {
    pub projection_id: Uuid,
    pub space: AgoraSpaceId,
    pub workspace_version: u64,
    pub task_node_id: CognitiveTaskNodeId,
    pub role: CognitiveRole,
    pub included_artifact_ids: Vec<CognitiveArtifactId>,
    pub omitted_artifact_ids: Vec<CognitiveArtifactId>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ClarificationId(pub Uuid);

impl ClarificationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ClarificationId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ClarificationState {
    Pending,
    Answered,
    Cancelled,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveCheckpoint {
    pub workspace_version: u64,
    pub task_contract_artifact_ref: Option<CognitiveArtifactId>,
    pub outstanding_obligations: Vec<String>,
    pub validation_artifact_refs: Vec<CognitiveArtifactId>,
    pub runtime_receipt_refs: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ClarificationRecord {
    pub id: ClarificationId,
    pub task_node_id: CognitiveTaskNodeId,
    pub requested_by: ProcessId,
    pub question: String,
    pub choices: Vec<String>,
    pub checkpoint: CognitiveCheckpoint,
    pub state: ClarificationState,
    pub response: Option<String>,
    /// Identity of the canonical user-input event that resumed the task.
    pub response_event_id: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct CognitiveInterruptionId(pub Uuid);

impl CognitiveInterruptionId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for CognitiveInterruptionId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CognitiveInterruptionReason {
    User,
    DaemonShutdown,
    ProviderUnavailable,
    AuthorityRequired,
    ExternalStateChanged,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CognitiveInterruptionRecord {
    pub id: CognitiveInterruptionId,
    pub task_node_id: CognitiveTaskNodeId,
    pub owner: ProcessId,
    pub reason: CognitiveInterruptionReason,
    pub checkpoint: CognitiveCheckpoint,
    pub resume_event_id: Option<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_digest_binds_typed_content() {
        let mut envelope = CognitiveArtifactEnvelope::proposed(
            AgoraSpaceId("root-task".into()),
            CognitiveTaskNodeId("plan".into()),
            ProcessId::new(),
            vec!["requirements:v1".into()],
            Vec::new(),
            0.9,
            CognitiveArtifact::Plan(CognitivePlanArtifact {
                steps: vec![CognitivePlanArtifactStep {
                    id: "read".into(),
                    description: "read authoritative code".into(),
                    requirement_refs: vec!["spec:section 1".into()],
                    dependencies: Vec::new(),
                }],
            }),
        )
        .unwrap();
        envelope.validate().unwrap();
        let CognitiveArtifact::Plan(plan) = &mut envelope.artifact else {
            unreachable!()
        };
        plan.steps[0].description = "unsupported guess".into();
        assert!(envelope.validate().is_err());
    }

    #[test]
    fn canonical_profiles_are_host_enforced_and_reviewer_is_independent() {
        for role in [
            CognitiveRole::Root,
            CognitiveRole::Planner,
            CognitiveRole::Explorer,
            CognitiveRole::Executor,
            CognitiveRole::Reviewer,
            CognitiveRole::Tester,
            CognitiveRole::Fixer,
        ] {
            CognitiveRoleProfile::canonical(role).validate().unwrap();
        }
        let reviewer = CognitiveRoleProfile::canonical(CognitiveRole::Reviewer);
        assert!(!reviewer.workspace_write_allowed);
        assert_eq!(
            reviewer.validation_authority,
            ValidationAuthority::IndependentGate
        );
        assert!(reviewer
            .required_output_artifacts
            .contains(&CognitiveArtifactKind::Review));
        assert!(!reviewer.context_projection.include_peer_history);
    }
}
