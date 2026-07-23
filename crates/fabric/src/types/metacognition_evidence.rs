//! Generic metacognition evidence contracts — domain-neutral evidence items.
//!
//! Every factual score, problem, and improvement claim must cite evidence.

use serde::{Deserialize, Serialize};

use super::metacognition_experience::ExperienceId;

/// Identifies one piece of evidence.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct EvidenceId(pub String);

/// The kind of evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceKind {
    Assertion,
    Observation,
    ActionResult,
    VerificationResult,
    Metric,
    Artifact,
    HumanFeedback,
    PolicyDecision,
    RuntimeFault,
}

/// Trust classification for evidence.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum EvidenceTrust {
    Authoritative,
    Corroborated,
    Unverified,
}

/// An evidence item captures one factual observation with integrity.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvidenceItem {
    pub schema_version: u16,
    pub evidence_id: EvidenceId,
    pub experience_id: ExperienceId,
    pub kind: EvidenceKind,
    pub source: String,
    pub producer: String,
    pub captured_at_ms: i64,
    pub payload: serde_json::Value,
    /// SHA-256 hex digest over canonical serialized payload bytes.
    pub sha256: String,
    pub trust: EvidenceTrust,
    pub freshness_ms: Option<u64>,
    pub redacted: bool,
}
