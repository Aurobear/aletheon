//! Governed skill proposal from a policy provider.
//! Cannot express raw joint/torque/topic commands.

use serde::{Deserialize, Serialize};

use crate::types::embodiment::{DeviceId, SkillId};
use crate::types::expected_outcome::ExpectedOutcome;
use crate::types::frame::FrameRef;

/// Policy's typed claim about how the proposed skill relates to the user's
/// goal. The Host accepts only `Direct` proposals for normal execution. A
/// `SafetyFallback` is evidence that the requested goal could not be satisfied;
/// it must enter the Host-owned safe-stop path and can never settle completed.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GoalAlignment {
    Direct,
    SafetyFallback,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyProvenance {
    /// Provider identifier (e.g. "openvla-v1").
    pub provider: String,
    /// Model name.
    pub model: String,
    /// Model version.
    pub version: String,
    /// Policy protocol negotiated from the gateway connection facts.
    #[serde(default)]
    pub protocol_version: String,
    /// Content digest of the model weights/config.
    pub digest: String,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SkillProposal {
    /// Proposed skill to execute.
    pub skill: SkillId,
    /// Target device.
    pub device: DeviceId,
    /// Skill parameters (validated against registered skill schema).
    pub parameters: serde_json::Value,
    /// What outcome is expected from this skill execution.
    pub expected_outcome: ExpectedOutcome,
    /// Whether this action directly fulfills the goal or is only a safety
    /// fallback. This is explicit Policy output, not inferred from skill names.
    pub goal_alignment: GoalAlignment,
    /// Confidence [0.0, 1.0].
    pub confidence: f32,
    /// Frame references that informed this proposal (max 4).
    pub frame_refs: Vec<FrameRef>,
    /// Policy provenance for audit.
    pub provenance: PolicyProvenance,
}

impl SkillProposal {
    pub fn validate(&self) -> Result<(), String> {
        self.expected_outcome
            .validate()
            .map_err(|e| format!("expected_outcome: {e}"))?;
        if !self.confidence.is_finite() || self.confidence < 0.0 || self.confidence > 1.0 {
            return Err(format!("confidence out of range: {}", self.confidence));
        }
        if self.frame_refs.len() > 4 {
            return Err("too many frame refs (max 4)".into());
        }
        for frame in &self.frame_refs {
            frame.validate()?;
        }
        for (field, value) in [
            ("provider", self.provenance.provider.as_str()),
            ("model", self.provenance.model.as_str()),
            ("version", self.provenance.version.as_str()),
            (
                "protocol_version",
                self.provenance.protocol_version.as_str(),
            ),
            ("digest", self.provenance.digest.as_str()),
        ] {
            if value.trim().is_empty() {
                return Err(format!("provenance {field} is required"));
            }
        }
        Ok(())
    }
}
