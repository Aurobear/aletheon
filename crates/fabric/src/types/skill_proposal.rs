//! Governed skill proposal from a policy provider.
//! Cannot express raw joint/torque/topic commands.

use serde::{Deserialize, Serialize};

use crate::types::embodiment::{DeviceId, SkillId};
use crate::types::expected_outcome::ExpectedOutcome;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct PolicyProvenance {
    /// Provider identifier (e.g. "openvla-v1").
    pub provider: String,
    /// Model name.
    pub model: String,
    /// Model version.
    pub version: String,
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
    /// Confidence [0.0, 1.0].
    pub confidence: f32,
    /// Frame references that informed this proposal (max 4).
    pub frame_refs: Vec<String>,
    /// Policy provenance for audit.
    pub provenance: PolicyProvenance,
}

impl SkillProposal {
    pub fn validate(&self) -> Result<(), String> {
        self.expected_outcome
            .validate()
            .map_err(|e| format!("expected_outcome: {e}"))?;
        if self.confidence < 0.0 || self.confidence > 1.0 {
            return Err(format!("confidence out of range: {}", self.confidence));
        }
        if self.frame_refs.len() > 4 {
            return Err("too many frame refs (max 4)".into());
        }
        if self.provenance.digest.is_empty() {
            return Err("provenance digest is required".into());
        }
        Ok(())
    }
}
