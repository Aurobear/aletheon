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

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value", rename_all = "snake_case")]
pub enum WorkspaceContent {
    Observation(WorkspaceObservation),
    Evidence(Evidence),
    Hypothesis(Hypothesis),
    Prediction(PredictionFrame),
    PredictionError(PredictionErrorFrame),
    Goal(GoalFrame),
    Concern(SelfSignal),
    Plan(Plan),
    ActionProposal(ActionProposalFrame),
    ToolOutcome(ToolOutcomeFrame),
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
                valid_text(&value.what) && valid_text(&value.source),
                "observation is incomplete"
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
    pub selected_because: SelectionExplanation,
    pub dasein_version: crate::dasein::SelfVersion,
    pub workspace_version: u64,
}
