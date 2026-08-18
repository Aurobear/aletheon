//! Domain policy and provider-neutral DTOs for durable memory candidates.

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

use crate::MemorySensitivity;

pub const MAX_ARCHITECTURE_DECISION_BYTES: usize = 64 * 1024;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ApprovedArchitectureDecision {
    pub decision_id: String,
    pub approval_id: String,
    pub title: String,
    pub content: String,
    pub principal_id: String,
    pub source_commit: String,
    pub approved_at_ms: i64,
    pub supersedes: Option<String>,
    pub sensitivity: MemorySensitivity,
    /// True only after the approval decision is durably persisted.
    pub approved: bool,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MemoryCandidateSource {
    pub record_id: String,
    pub kind: String,
    pub content: serde_json::Value,
    pub sensitivity: MemorySensitivity,
}

impl MemoryCandidateSource {
    pub fn new(
        record_id: impl Into<String>,
        kind: impl Into<String>,
        content: serde_json::Value,
        sensitivity: MemorySensitivity,
    ) -> Self {
        Self {
            record_id: record_id.into(),
            kind: kind.into(),
            content,
            sensitivity,
        }
    }
}

pub fn candidate_sensitivity_allowed(sensitivity: &MemorySensitivity) -> bool {
    !matches!(
        sensitivity,
        MemorySensitivity::Confidential | MemorySensitivity::Restricted
    )
}

pub fn architecture_decision_candidate(
    decision: &ApprovedArchitectureDecision,
) -> Result<MemoryCandidateSource, &'static str> {
    if !decision.approved {
        return Err("architecture decision is not approved");
    }
    if !candidate_sensitivity_allowed(&decision.sensitivity) {
        return Err("sensitive decision");
    }
    if decision.content.len() > MAX_ARCHITECTURE_DECISION_BYTES {
        return Err("architecture decision exceeds byte limit");
    }

    Ok(MemoryCandidateSource::new(
        format!(
            "decision:{}:{}",
            decision.decision_id,
            short_hash(&decision.approval_id)
        ),
        "architecture_decision",
        serde_json::json!({
            "decision_id": decision.decision_id,
            "approval_id": decision.approval_id,
            "title": decision.title,
            "content": decision.content,
            "principal_id": decision.principal_id,
            "source_commit": decision.source_commit,
            "approved_at_ms": decision.approved_at_ms,
            "supersedes": decision.supersedes,
        }),
        decision.sensitivity.clone(),
    ))
}

fn short_hash(value: &str) -> String {
    format!("{:x}", Sha256::digest(value.as_bytes()))[..16].to_string()
}
