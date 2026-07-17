//! Typed, immutable contracts for Agora candidate competition and broadcast.

use crate::dasein::SelfSignal;
use crate::primitives::cognitive::Hypothesis;
use crate::types::agent_control::AgentResult;
use crate::types::evidence::Evidence;
use crate::types::operation::{OperationId, ProcessId};
use crate::types::session::TurnId;
use crate::types::space::AgoraSpaceId;
use crate::types::time::{MonoDeadline, MonoTime, WallTime};
use crate::Plan;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use uuid::Uuid;

pub const WORKSPACE_SCHEMA_V1: u16 = 1;
const MAX_TEXT_BYTES: usize = 32 * 1024;
const MAX_EXTENSION_BYTES: usize = 64 * 1024;
const MAX_REFERENCES: usize = 64;
const MAX_DEPENDENCIES: usize = 32;
pub const MAX_BROADCAST_WINNERS: usize = 32;
pub const MAX_BROADCAST_RESPONSES: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ContentId(pub Uuid);

impl ContentId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for ContentId {
    fn default() -> Self {
        Self::new()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct BroadcastEpoch(pub u64);

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceObservation {
    pub what: String,
    pub source: String,
    pub data: serde_json::Value,
    #[serde(default)]
    pub attribution: WorkspaceAttribution,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionFrame {
    pub statement: String,
    pub horizon_ms: u64,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PredictionErrorFrame {
    pub prediction_id: ContentId,
    pub description: String,
    pub magnitude: f32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalFrame {
    pub id: String,
    pub summary: String,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionProposalFrame {
    pub id: String,
    pub summary: String,
    pub risk: f32,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolOutcomeFrame {
    pub call_id: String,
    pub tool: String,
    pub output_ref: String,
    pub is_error: bool,
}
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceReflection {
    pub findings: Vec<String>,
    pub confidence: f32,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CareConcernFrame {
    pub purpose: String,
    pub urgency: f32,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum WorkspaceAttribution {
    User,
    #[default]
    Environment,
    RootAgent {
        process: ProcessId,
    },
    ChildAgent {
        process: ProcessId,
    },
    ExternalMemory {
        provider: String,
    },
    Dasein,
    Cognit,
    Metacog,
    Corpus,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecalledExperienceFrame {
    pub memory_id: String,
    pub summary: String,
    pub trust: f32,
    pub attribution: WorkspaceAttribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernedActionOutcomeFrame {
    pub action_id: ContentId,
    pub permit_id: String,
    pub operation: OperationId,
    pub output_ref: String,
    pub is_error: bool,
    pub attribution: WorkspaceAttribution,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum WorkspaceContent {
    Observation(WorkspaceObservation),
    RecalledExperience(RecalledExperienceFrame),
    Evidence(Evidence),
    Hypothesis(Hypothesis),
    Prediction(PredictionFrame),
    PredictionError(PredictionErrorFrame),
    Goal(GoalFrame),
    Concern(SelfSignal),
    CareConcern(CareConcernFrame),
    Plan(Plan),
    ActionProposal(ActionProposalFrame),
    ToolOutcome(ToolOutcomeFrame),
    GovernedActionOutcome(GovernedActionOutcomeFrame),
    AgentResult(AgentResult),
    Reflection(WorkspaceReflection),
    Extension {
        schema: String,
        payload: serde_json::Value,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum VisibilityScope {
    PrivateProcess { process: ProcessId },
    AgentTree { root: ProcessId },
    Session,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct WorkspaceProvenance {
    pub producer: ProcessId,
    pub operation: Option<OperationId>,
    pub source_refs: Vec<String>,
    pub observed_at: WallTime,
}

#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub struct SalienceVector {
    pub urgency: f32,
    pub goal_relevance: f32,
    pub self_relevance: f32,
    pub novelty: f32,
    pub confidence: f32,
    pub prediction_error: f32,
    pub affect_intensity: f32,
    pub social_relevance: f32,
}

impl SalienceVector {
    pub fn values(self) -> [f32; 8] {
        [
            self.urgency,
            self.goal_relevance,
            self.self_relevance,
            self.novelty,
            self.confidence,
            self.prediction_error,
            self.affect_intensity,
            self.social_relevance,
        ]
    }
    pub fn validate(self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.values()
                .into_iter()
                .all(|value| value.is_finite() && (0.0..=1.0).contains(&value)),
            "salience dimensions must be finite values in [0,1]"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceCandidate {
    pub schema_version: u16,
    pub id: ContentId,
    pub space: AgoraSpaceId,
    pub source: ProcessId,
    pub turn: Option<TurnId>,
    pub content: WorkspaceContent,
    pub confidence: f32,
    pub salience: SalienceVector,
    pub provenance: WorkspaceProvenance,
    pub visibility: VisibilityScope,
    pub dependencies: Vec<ContentId>,
    pub created_at: MonoTime,
    pub expires_at: Option<MonoDeadline>,
}

impl WorkspaceCandidate {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == WORKSPACE_SCHEMA_V1,
            "unsupported workspace candidate schema"
        );
        anyhow::ensure!(
            !self.space.0.trim().is_empty(),
            "workspace candidate space is empty"
        );
        anyhow::ensure!(
            self.source == self.provenance.producer,
            "candidate source and provenance producer differ"
        );
        anyhow::ensure!(
            self.confidence.is_finite() && (0.0..=1.0).contains(&self.confidence),
            "candidate confidence must be finite and in [0,1]"
        );
        self.salience.validate()?;
        anyhow::ensure!(
            !self.provenance.source_refs.is_empty()
                && self.provenance.source_refs.len() <= MAX_REFERENCES,
            "candidate provenance references are missing or excessive"
        );
        anyhow::ensure!(
            self.provenance
                .source_refs
                .iter()
                .all(|reference| !reference.trim().is_empty() && reference.len() <= MAX_TEXT_BYTES),
            "candidate provenance reference is invalid"
        );
        anyhow::ensure!(
            self.dependencies.len() <= MAX_DEPENDENCIES,
            "candidate dependency count exceeds limit"
        );
        anyhow::ensure!(
            !self.dependencies.contains(&self.id),
            "candidate cannot depend on itself"
        );
        if let Some(deadline) = self.expires_at {
            anyhow::ensure!(
                deadline.0 > self.created_at,
                "candidate expiry must follow creation"
            );
        }
        self.validate_content()
    }

    pub fn is_expired_at(&self, now: MonoTime) -> bool {
        self.expires_at
            .is_some_and(|deadline| deadline.is_expired_at(now))
    }

    pub fn content_fingerprint(&self) -> anyhow::Result<String> {
        let material = serde_json::json!({
            "schema_version": self.schema_version,
            "space": self.space,
            "content": self.content,
            "provenance_refs": self.provenance.source_refs,
            "visibility": self.visibility,
            "dependencies": self.dependencies,
        });
        let digest = Sha256::digest(serde_json::to_vec(&material)?);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }

    fn validate_content(&self) -> anyhow::Result<()> {
        let valid_text = |text: &str| !text.trim().is_empty() && text.len() <= MAX_TEXT_BYTES;
        match &self.content {
            WorkspaceContent::Observation(value) => anyhow::ensure!(
                valid_text(&value.what)
                    && valid_text(&value.source)
                    && valid_attribution(&value.attribution),
                "observation is incomplete"
            ),
            WorkspaceContent::RecalledExperience(value) => anyhow::ensure!(
                valid_text(&value.memory_id)
                    && valid_text(&value.summary)
                    && value.trust.is_finite()
                    && (0.0..=1.0).contains(&value.trust)
                    && valid_attribution(&value.attribution),
                "recalled experience is invalid"
            ),
            WorkspaceContent::Evidence(value) => anyhow::ensure!(
                valid_text(&value.id)
                    && valid_text(&value.source)
                    && value.weight.is_finite()
                    && (0.0..=1.0).contains(&value.weight),
                "evidence is invalid"
            ),
            WorkspaceContent::Hypothesis(value) => anyhow::ensure!(
                valid_text(&value.id)
                    && valid_text(&value.statement)
                    && value.confidence.is_finite()
                    && (0.0..=1.0).contains(&value.confidence),
                "hypothesis is invalid"
            ),
            WorkspaceContent::Prediction(value) => {
                anyhow::ensure!(valid_text(&value.statement), "prediction is empty")
            }
            WorkspaceContent::PredictionError(value) => anyhow::ensure!(
                valid_text(&value.description)
                    && value.magnitude.is_finite()
                    && (0.0..=1.0).contains(&value.magnitude),
                "prediction error is invalid"
            ),
            WorkspaceContent::Goal(value) => anyhow::ensure!(
                valid_text(&value.id) && valid_text(&value.summary),
                "goal is invalid"
            ),
            WorkspaceContent::Concern(_) => {}
            WorkspaceContent::CareConcern(value) => anyhow::ensure!(
                valid_text(&value.purpose)
                    && value.urgency.is_finite()
                    && (0.0..=1.0).contains(&value.urgency),
                "care concern is invalid"
            ),
            WorkspaceContent::Plan(value) => anyhow::ensure!(
                !value.steps.is_empty() && value.steps.len() <= MAX_DEPENDENCIES,
                "plan is empty or excessive"
            ),
            WorkspaceContent::ActionProposal(value) => anyhow::ensure!(
                valid_text(&value.id)
                    && valid_text(&value.summary)
                    && value.risk.is_finite()
                    && (0.0..=1.0).contains(&value.risk),
                "action proposal is invalid"
            ),
            WorkspaceContent::ToolOutcome(value) => anyhow::ensure!(
                valid_text(&value.call_id)
                    && valid_text(&value.tool)
                    && valid_text(&value.output_ref),
                "tool outcome is invalid"
            ),
            WorkspaceContent::GovernedActionOutcome(value) => anyhow::ensure!(
                valid_text(&value.permit_id)
                    && valid_text(&value.output_ref)
                    && valid_attribution(&value.attribution),
                "governed action outcome is invalid"
            ),
            WorkspaceContent::AgentResult(value) => value
                .validate()
                .map_err(|error| anyhow::anyhow!(error.to_string()))?,
            WorkspaceContent::Reflection(value) => anyhow::ensure!(
                !value.findings.is_empty()
                    && value.findings.len() <= MAX_REFERENCES
                    && value.findings.iter().all(|finding| valid_text(finding))
                    && value.confidence.is_finite()
                    && (0.0..=1.0).contains(&value.confidence),
                "reflection is invalid"
            ),
            WorkspaceContent::Extension { schema, payload } => anyhow::ensure!(
                schema.starts_with("v1/")
                    && schema.len() <= 256
                    && serde_json::to_vec(payload)?.len() <= MAX_EXTENSION_BYTES,
                "workspace extension is invalid"
            ),
        }
        Ok(())
    }
}

fn valid_attribution(attribution: &WorkspaceAttribution) -> bool {
    match attribution {
        WorkspaceAttribution::ExternalMemory { provider } => {
            !provider.trim().is_empty() && provider.len() <= MAX_TEXT_BYTES
        }
        _ => true,
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct CandidateScore {
    pub id: ContentId,
    pub source: ProcessId,
    pub salience: f64,
    pub aging_boost: f64,
    pub dependency_boost: f64,
    pub repetition_penalty: f64,
    pub refractory_penalty: f64,
    pub total: f64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SelectionExplanation {
    pub policy_version: u16,
    pub evaluated: Vec<CandidateScore>,
    pub selected_ids: Vec<ContentId>,
    pub rejected_below_ignition: Vec<ContentId>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SelectionResult {
    pub selected: Vec<WorkspaceCandidate>,
    pub explanation: SelectionExplanation,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct WorkspaceBroadcast {
    pub schema_version: u16,
    pub epoch: BroadcastEpoch,
    pub space: AgoraSpaceId,
    pub winner_ids: Vec<ContentId>,
    pub contents: Vec<WorkspaceContent>,
    /// Complete immutable selected records. This preserves visibility and
    /// provenance across durable replay instead of reconstructing them from
    /// parallel, lossy arrays.
    pub selected: Vec<WorkspaceCandidate>,
    pub selected_because: SelectionExplanation,
    pub dasein_version: crate::dasein::SelfVersion,
    pub workspace_version: u64,
}

impl WorkspaceBroadcast {
    pub fn from_selection(
        epoch: BroadcastEpoch,
        selection: SelectionResult,
        dasein_version: crate::dasein::SelfVersion,
        workspace_version: u64,
    ) -> anyhow::Result<Self> {
        anyhow::ensure!(epoch.0 > 0, "broadcast epoch must be non-zero");
        anyhow::ensure!(workspace_version > 0, "workspace version must be non-zero");
        anyhow::ensure!(
            !selection.selected.is_empty() && selection.selected.len() <= MAX_BROADCAST_WINNERS,
            "broadcast selection is empty or excessive"
        );
        let space = selection.selected[0].space.clone();
        let value = Self {
            schema_version: WORKSPACE_SCHEMA_V1,
            epoch,
            space,
            winner_ids: selection.selected.iter().map(|value| value.id).collect(),
            contents: selection
                .selected
                .iter()
                .map(|value| value.content.clone())
                .collect(),
            selected: selection.selected,
            selected_because: selection.explanation,
            dasein_version,
            workspace_version,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == WORKSPACE_SCHEMA_V1,
            "unsupported workspace broadcast schema"
        );
        anyhow::ensure!(self.epoch.0 > 0, "broadcast epoch must be non-zero");
        anyhow::ensure!(
            self.workspace_version > 0,
            "workspace version must be non-zero"
        );
        anyhow::ensure!(
            !self.selected.is_empty() && self.selected.len() <= MAX_BROADCAST_WINNERS,
            "broadcast selection is empty or excessive"
        );
        for candidate in &self.selected {
            candidate.validate()?;
            anyhow::ensure!(candidate.space == self.space, "broadcast crosses spaces");
        }
        let ids: Vec<_> = self.selected.iter().map(|value| value.id).collect();
        let mut unique_ids = ids.clone();
        unique_ids.sort();
        unique_ids.dedup();
        anyhow::ensure!(
            unique_ids.len() == ids.len(),
            "broadcast winner IDs are duplicated"
        );
        anyhow::ensure!(
            self.winner_ids == ids,
            "broadcast winner IDs are inconsistent"
        );
        anyhow::ensure!(
            self.selected_because.selected_ids == ids,
            "broadcast explanation is inconsistent"
        );
        let selected_contents = serde_json::to_vec(
            &self
                .selected
                .iter()
                .map(|value| &value.content)
                .collect::<Vec<_>>(),
        )?;
        anyhow::ensure!(
            serde_json::to_vec(&self.contents)? == selected_contents,
            "broadcast contents are inconsistent"
        );
        Ok(())
    }

    pub fn checksum(&self) -> anyhow::Result<String> {
        self.validate()?;
        let digest = Sha256::digest(serde_json::to_vec(self)?);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BroadcastAckStatus {
    Responded,
    Delivered,
    Failed,
    TimedOut,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BroadcastAck {
    pub schema_version: u16,
    pub space: AgoraSpaceId,
    pub epoch: BroadcastEpoch,
    pub processor: ProcessId,
    pub response_ids: Vec<ContentId>,
    pub status: BroadcastAckStatus,
    pub observed_at: WallTime,
    pub detail: Option<String>,
}

impl BroadcastAck {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == WORKSPACE_SCHEMA_V1,
            "unsupported broadcast acknowledgement schema"
        );
        anyhow::ensure!(self.epoch.0 > 0, "acknowledgement epoch must be non-zero");
        anyhow::ensure!(
            self.response_ids.len() <= MAX_BROADCAST_RESPONSES,
            "broadcast response count exceeds limit"
        );
        let mut unique = self.response_ids.clone();
        unique.sort();
        unique.dedup();
        anyhow::ensure!(
            unique.len() == self.response_ids.len(),
            "broadcast response IDs contain duplicates"
        );
        match self.status {
            BroadcastAckStatus::Responded => anyhow::ensure!(
                !self.response_ids.is_empty(),
                "responded acknowledgement has no responses"
            ),
            BroadcastAckStatus::Delivered => anyhow::ensure!(
                self.response_ids.is_empty(),
                "delivered acknowledgement contains responses"
            ),
            BroadcastAckStatus::Failed | BroadcastAckStatus::TimedOut => {
                anyhow::ensure!(
                    self.response_ids.is_empty()
                        && self.detail.as_deref().is_some_and(|v| !v.trim().is_empty()),
                    "failed acknowledgement is missing terminal detail"
                )
            }
        }
        anyhow::ensure!(
            self.detail
                .as_ref()
                .is_none_or(|value| !value.trim().is_empty() && value.len() <= 1024),
            "broadcast acknowledgement detail is invalid"
        );
        Ok(())
    }

    pub fn checksum(&self) -> anyhow::Result<String> {
        self.validate()?;
        let digest = Sha256::digest(serde_json::to_vec(self)?);
        Ok(digest.iter().map(|byte| format!("{byte:02x}")).collect())
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BroadcastDelivery {
    pub schema_version: u16,
    pub epoch: BroadcastEpoch,
    pub space: AgoraSpaceId,
    pub recipient: ProcessId,
    pub recipient_agent_root: ProcessId,
    pub broadcast_checksum: String,
    pub dasein_version: crate::dasein::SelfVersion,
    pub workspace_version: u64,
    /// Visibility-filtered immutable subset of the durable broadcast.
    pub selected: Vec<WorkspaceCandidate>,
}

impl BroadcastDelivery {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.schema_version == WORKSPACE_SCHEMA_V1,
            "unsupported broadcast delivery schema"
        );
        anyhow::ensure!(self.epoch.0 > 0, "broadcast delivery epoch is zero");
        anyhow::ensure!(
            self.workspace_version > 0,
            "delivery workspace version is zero"
        );
        anyhow::ensure!(
            self.broadcast_checksum.len() == 64
                && self
                    .broadcast_checksum
                    .bytes()
                    .all(|byte| byte.is_ascii_hexdigit()),
            "delivery broadcast checksum is invalid"
        );
        anyhow::ensure!(!self.selected.is_empty(), "broadcast delivery is empty");
        for candidate in &self.selected {
            candidate.validate()?;
            anyhow::ensure!(candidate.space == self.space, "delivery crosses spaces");
            let visible = match candidate.visibility {
                VisibilityScope::Session => true,
                VisibilityScope::PrivateProcess { process } => process == self.recipient,
                VisibilityScope::AgentTree { root } => root == self.recipient_agent_root,
            };
            anyhow::ensure!(visible, "delivery leaks a visibility-scoped candidate");
        }
        Ok(())
    }
}
