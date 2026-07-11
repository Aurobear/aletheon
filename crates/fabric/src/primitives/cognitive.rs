//! Cognitive objects — the canonical vocabulary of RFC-017.
//!
//! Existing types are re-exported from their current homes (no redefinition).
//! The four objects that had no home before (Hypothesis, Evidence, Narrative,
//! Commitment) are defined here as simple serde structs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// -- Re-exports of existing cognitive objects (single source of truth) --
pub use crate::include::cognit::{Experience, Observation, Plan};
pub use crate::include::self_field::Intent;
pub use crate::policy::execpolicy::Decision;

// -- New cognitive objects --

/// A tentative explanation awaiting verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hypothesis {
    pub id: String,
    pub statement: String,
    /// Confidence in [0.0, 1.0].
    pub confidence: f64,
    /// IDs of `Evidence` supporting or refuting this hypothesis.
    pub evidence_ids: Vec<String>,
}

/// A piece of evidence bearing on a hypothesis or decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    /// Where the evidence came from (tool name, memory id, observation id).
    pub source: String,
    pub content: String,
    /// Relative weight in [0.0, 1.0].
    pub weight: f64,
}

impl Evidence {
    /// Build `Evidence` from a tool result — the canonical producer today.
    ///
    /// A successful result carries full weight (1.0); an error carries none
    /// (0.0) so downstream reasoning can discount it.
    pub fn from_tool_result(
        call_id: impl Into<String>,
        source: impl Into<String>,
        content: impl Into<String>,
        is_error: bool,
    ) -> Self {
        Self {
            id: call_id.into(),
            source: source.into(),
            content: content.into(),
            weight: if is_error { 0.0 } else { 1.0 },
        }
    }
}

/// A running self-narrative summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Narrative {
    pub id: String,
    pub summary: String,
    /// Ordered narrative fragments.
    pub entries: Vec<String>,
}

/// Lifecycle of a commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitmentStatus {
    Open,
    Fulfilled,
    Abandoned,
}

/// A commitment the agent has made and intends to honor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub id: String,
    pub statement: String,
    pub created_at: DateTime<Utc>,
    pub status: CommitmentStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypothesis_roundtrips_json() {
        let h = Hypothesis {
            id: "h1".into(),
            statement: "the disk is full".into(),
            confidence: 0.8,
            evidence_ids: vec!["e1".into()],
        };
        let json = serde_json::to_string(&h).unwrap();
        let back: Hypothesis = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "h1");
        assert_eq!(back.evidence_ids, vec!["e1".to_string()]);
    }

    #[test]
    fn commitment_status_serializes() {
        assert_eq!(
            serde_json::to_string(&CommitmentStatus::Open).unwrap(),
            "\"Open\""
        );
    }

    #[test]
    fn evidence_from_tool_result_weights_by_error() {
        let ok = Evidence::from_tool_result("c1", "bash", "exit 0", false);
        assert_eq!(ok.id, "c1");
        assert_eq!(ok.source, "bash");
        assert_eq!(ok.weight, 1.0);

        let err = Evidence::from_tool_result("c2", "bash", "boom", true);
        assert_eq!(err.weight, 0.0);
    }
}
