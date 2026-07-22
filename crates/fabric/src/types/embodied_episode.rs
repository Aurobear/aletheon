//! Immutable embodied execution episode types for Mnemosyne persistence.

use serde::{Deserialize, Serialize};

use crate::types::embodiment::{DeviceId, EvidenceRef, SkillId, SkillResult};
use crate::types::expected_outcome::ExpectedOutcome;
use crate::types::outcome_verification::VerificationReport;
use crate::types::world_state::WorldSnapshot;
use crate::OperationId;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EmbodiedEpisode {
    /// Schema version for forward compatibility.
    pub schema_version: u16,
    /// Goal identifier that triggered this episode.
    pub goal_id: String,
    /// Device this episode executed on.
    pub device: DeviceId,
    /// Ordered attempts within this episode.
    pub attempts: Vec<EpisodeAttempt>,
    /// Evidence references (no raw images/joint streams inline).
    pub evidence: Vec<EvidenceRef>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct EpisodeAttempt {
    /// Unique operation ID for this attempt.
    pub operation_id: OperationId,
    /// The skill requested.
    pub skill: SkillId,
    /// Expected outcome for this attempt.
    pub expected_outcome: ExpectedOutcome,
    /// World snapshot before execution.
    pub before: Option<WorldSnapshot>,
    /// World snapshot after execution.
    pub after: Option<WorldSnapshot>,
    /// Result from the embodiment provider.
    pub result: Option<SkillResult>,
    /// Verification report (if verification was performed).
    pub verification: Option<VerificationReport>,
    /// Recovery action taken, if any.
    pub recovery: Option<String>,
}
