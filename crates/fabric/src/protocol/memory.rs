//! Product-neutral Memory Gateway v1 wire contracts.

use std::path::PathBuf;

use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

pub const MEMORY_GATEWAY_SCHEMA_V1: u16 = 1;
pub const MAX_MEMORY_ID_BYTES: usize = 256;
pub const MAX_MEMORY_QUERY_BYTES: usize = 4 * 1024;
pub const MAX_MEMORY_CONTENT_BYTES: usize = 256 * 1024;
pub const MAX_MEMORY_SOURCE_REFS: usize = 64;
pub const MAX_MEMORY_SOURCE_REF_BYTES: usize = 512;
pub const MAX_MEMORY_RECALL_ITEMS: usize = 100;
pub const MAX_MEMORY_RECALL_CONTENT_BYTES: usize = 256 * 1024;
pub const MAX_MEMORY_REQUESTED_KINDS: usize = 32;

#[derive(Debug, thiserror::Error, Clone, PartialEq, Eq)]
#[error("invalid memory gateway request: {0}")]
pub struct MemoryProtocolValidationError(pub String);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryObservationKindV1 {
    UserMessage,
    AssistantMessage,
    ToolOutcome,
    TaskOutcome,
    ExplicitNote,
    Correction,
    Feedback,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemorySensitivityV1 {
    Public,
    Internal,
    Confidential,
    Restricted,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryObservationRequestV1 {
    pub observation_id: String,
    pub client_session_id: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub client_turn_id: Option<String>,
    pub working_dir: PathBuf,
    pub kind: MemoryObservationKindV1,
    pub content: String,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub occurred_at: Option<String>,
    #[serde(default)]
    pub source_refs: Vec<String>,
    pub sensitivity_hint: MemorySensitivityV1,
    #[serde(default)]
    pub explicit_user_action: bool,
}

impl MemoryObservationRequestV1 {
    pub fn validate(&self) -> Result<(), MemoryProtocolValidationError> {
        validate_id("observation_id", &self.observation_id)?;
        validate_id("client_session_id", &self.client_session_id)?;
        if let Some(turn_id) = &self.client_turn_id {
            validate_id("client_turn_id", turn_id)?;
        }
        validate_working_dir(&self.working_dir)?;
        validate_content("content", &self.content, false)?;
        if self.source_refs.len() > MAX_MEMORY_SOURCE_REFS {
            return invalid("source_refs exceeds item limit");
        }
        for source_ref in &self.source_refs {
            if source_ref.trim().is_empty() || source_ref.len() > MAX_MEMORY_SOURCE_REF_BYTES {
                return invalid("source_ref is empty or exceeds byte limit");
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryReceiptGetRequestV1 {
    pub durable_intake_id: String,
}

impl MemoryReceiptGetRequestV1 {
    pub fn validate(&self) -> Result<(), MemoryProtocolValidationError> {
        validate_id("durable_intake_id", &self.durable_intake_id)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryRecordKindV1 {
    Message,
    ToolOutcome,
    GoalOutcome,
    Reflection,
    Episodic,
    SemanticFact,
    Procedure,
    CoreState,
    ArchitectureDecision,
    ExternalReference,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRecallRequestV1 {
    pub request_id: String,
    pub client_session_id: String,
    pub working_dir: PathBuf,
    pub query: String,
    pub max_items: usize,
    pub max_content_bytes: usize,
    #[serde(default)]
    pub include_historical: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub requested_kinds: Option<Vec<MemoryRecordKindV1>>,
}

impl MemoryRecallRequestV1 {
    pub fn validate(&self) -> Result<(), MemoryProtocolValidationError> {
        validate_id("request_id", &self.request_id)?;
        validate_id("client_session_id", &self.client_session_id)?;
        validate_working_dir(&self.working_dir)?;
        if self.query.trim().is_empty() || self.query.len() > MAX_MEMORY_QUERY_BYTES {
            return invalid("query is empty or exceeds byte limit");
        }
        if self.max_items == 0 {
            return invalid("max_items must be positive");
        }
        if self.max_content_bytes == 0 {
            return invalid("max_content_bytes must be positive");
        }
        if self
            .requested_kinds
            .as_ref()
            .is_some_and(|kinds| kinds.len() > MAX_MEMORY_REQUESTED_KINDS)
        {
            return invalid("requested_kinds exceeds item limit");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryFeedbackSignalV1 {
    Useful,
    NotUseful,
    Incorrect,
    Stale,
    Sensitive,
    Corrected,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryFeedbackRequestV1 {
    pub observation_id: String,
    pub client_session_id: String,
    pub target_record_id: String,
    pub signal: MemoryFeedbackSignalV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub correction_text: Option<String>,
    pub working_dir: PathBuf,
}

impl MemoryFeedbackRequestV1 {
    pub fn validate(&self) -> Result<(), MemoryProtocolValidationError> {
        validate_id("observation_id", &self.observation_id)?;
        validate_id("client_session_id", &self.client_session_id)?;
        validate_id("target_record_id", &self.target_record_id)?;
        validate_working_dir(&self.working_dir)?;
        if let Some(correction) = &self.correction_text {
            validate_content("correction_text", correction, true)?;
        }
        if matches!(self.signal, MemoryFeedbackSignalV1::Corrected)
            && self
                .correction_text
                .as_ref()
                .is_none_or(|text| text.trim().is_empty())
        {
            return invalid("corrected feedback requires correction_text");
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryIntakeStatusV1 {
    Observed,
    Duplicate,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryWorkspaceStateV1 {
    Bound,
    LocalOnly,
    Incompatible,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryProjectionStateV1 {
    NotEligible,
    PendingEvaluation,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryObservationReceiptV1 {
    pub observation_id: String,
    pub durable_intake_id: String,
    pub intake_status: MemoryIntakeStatusV1,
    pub workspace_state: MemoryWorkspaceStateV1,
    pub projection_state: MemoryProjectionStateV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reason_code: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryLifecycleStateV1 {
    Observed,
    Evaluating,
    Rejected,
    PromotedLocal,
    ProjectionQueued,
    ProjectedRemote,
    ProjectionFailed,
    Superseded,
    Tombstoned,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryScorecardV1 {
    pub policy_version: String,
    pub evidence_provenance: i16,
    pub future_utility: i16,
    pub stability: i16,
    pub novelty_dedup: i16,
    pub scope_fit: i16,
    pub verification: i16,
    pub privacy_risk: i16,
    pub contradiction_risk: i16,
    pub total: i16,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryLifecycleReceiptV1 {
    pub durable_intake_id: String,
    pub revision: u64,
    pub state: MemoryLifecycleStateV1,
    #[serde(default)]
    pub resulting_record_ids: Vec<String>,
    #[serde(default)]
    pub remote_receipt_ids: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub scorecard: Option<MemoryScorecardV1>,
    #[serde(default)]
    pub reason_codes: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub terminal_at: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScopeKindV1 {
    Global,
    Principal,
    Workspace,
    Session,
    Goal,
    Agent,
    Task,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryScopeViewV1 {
    pub kind: MemoryScopeKindV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryAuthorityV1 {
    ApprovedCore,
    VerifiedLocalSemantic,
    LocalEpisode,
    AletheonExternal,
    ExternalReference,
    RawExperience,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryTemporalStateV1 {
    Current,
    Superseded,
    Expired,
    Unknown,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "snake_case")]
pub enum MemoryEvidenceV1 {
    AliasHit,
    ExactTitleMatch,
    HighVectorMatch,
    KeywordExact,
    WeakSemantic,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRecallItemV1 {
    pub record_id: String,
    pub kind: MemoryRecordKindV1,
    pub scope: MemoryScopeViewV1,
    pub authority: MemoryAuthorityV1,
    pub temporal_state: MemoryTemporalStateV1,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub evidence: Option<MemoryEvidenceV1>,
    pub score: f32,
    pub sensitivity: MemorySensitivityV1,
    pub source: String,
    pub source_id: String,
    pub content: String,
    #[serde(default)]
    pub untrusted_reference: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryRecallResultV1 {
    pub request_id: String,
    pub items: Vec<MemoryRecallItemV1>,
    #[serde(default)]
    pub degraded_sources: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, JsonSchema)]
pub struct MemoryFeedbackReceiptV1 {
    pub observation: MemoryObservationReceiptV1,
    pub lifecycle: MemoryLifecycleReceiptV1,
}

fn validate_id(name: &str, value: &str) -> Result<(), MemoryProtocolValidationError> {
    if value.trim().is_empty() || value.len() > MAX_MEMORY_ID_BYTES {
        return invalid(format!("{name} is empty or exceeds byte limit"));
    }
    Ok(())
}

fn validate_working_dir(path: &std::path::Path) -> Result<(), MemoryProtocolValidationError> {
    if path.as_os_str().is_empty() || !path.is_absolute() {
        return invalid("working_dir must be an absolute path");
    }
    Ok(())
}

fn validate_content(
    name: &str,
    value: &str,
    allow_empty: bool,
) -> Result<(), MemoryProtocolValidationError> {
    if (!allow_empty && value.trim().is_empty()) || value.len() > MAX_MEMORY_CONTENT_BYTES {
        return invalid(format!("{name} is empty or exceeds byte limit"));
    }
    Ok(())
}

fn invalid<T>(message: impl Into<String>) -> Result<T, MemoryProtocolValidationError> {
    Err(MemoryProtocolValidationError(message.into()))
}
