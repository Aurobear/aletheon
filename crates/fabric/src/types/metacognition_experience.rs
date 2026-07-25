//! Generic metacognition experience contracts — domain-neutral experience envelopes.
//!
//! These types form the stable Fabric ABI that domain adapters implement.
//! Domain-specific types must never enter this module.

use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

use super::metacognition_evidence::EvidenceId;

/// Schema version for the metacognition experience envelope.
pub const METACOGNITION_SCHEMA_V1: u16 = 1;

/// Identifies a capability domain (e.g., "coding", "robot", "research").
///
/// Constructed through `DomainId::new()` which validates the identifier.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct DomainId(String);

impl DomainId {
    /// Create a validated domain identifier.
    ///
    /// Rejects empty strings and identifiers containing control characters.
    pub fn new(raw: &str) -> Result<Self, DomainIdError> {
        let trimmed = raw.trim();
        if trimmed.is_empty() {
            return Err(DomainIdError::Empty);
        }
        if trimmed.len() > 64 {
            return Err(DomainIdError::TooLong);
        }
        if trimmed.chars().any(|c| c.is_control()) {
            return Err(DomainIdError::ControlCharacters);
        }
        if !trimmed
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '_')
        {
            return Err(DomainIdError::InvalidCharacters);
        }
        Ok(Self(trimmed.to_string()))
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

impl std::fmt::Display for DomainId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum DomainIdError {
    #[error("domain id must not be empty")]
    Empty,
    #[error("domain id exceeds 64 characters")]
    TooLong,
    #[error("domain id contains control characters")]
    ControlCharacters,
    #[error("domain id contains invalid characters (use alphanumeric, hyphen, underscore)")]
    InvalidCharacters,
}

/// Identifies one assessable experience.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct ExperienceId(pub String);

/// Identifies the evaluated capability or component.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct SubjectId(pub String);

/// Outcome of an experience.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExperienceOutcome {
    Succeeded,
    Failed,
    Cancelled,
    TimedOut,
    Unknown,
}

/// An experience envelope identifies one assessable unit of work.
///
/// It carries references and normalized summaries, not arbitrary domain-private
/// runtime objects.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExperienceEnvelope {
    pub schema_version: u16,
    pub experience_id: ExperienceId,
    pub domain: DomainId,
    pub subject: SubjectId,
    pub goal_ref: Option<String>,
    pub started_at_ms: i64,
    pub completed_at_ms: Option<i64>,
    pub outcome: ExperienceOutcome,
    pub correlations: BTreeMap<String, String>,
    pub evidence: Vec<EvidenceId>,
}
