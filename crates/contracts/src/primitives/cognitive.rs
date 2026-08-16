//! Cognitive objects — the canonical vocabulary of RFC-017.
//!
//! Existing types are re-exported from their current homes (no redefinition).
//! Evidence is now defined in `fabric` for broader access.

use serde::{Deserialize, Serialize};

// -- Re-exports of existing cognitive objects (single source of truth) --

// Evidence remains a shared immutable value consumed by the Agora owner.
pub use crate::types::evidence::Evidence;

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
    fn evidence_from_tool_result_weights_by_error() {
        let ok = Evidence::from_tool_result("c1", "bash", "exit 0", false);
        assert_eq!(ok.id, "c1");
        assert_eq!(ok.source, "bash");
        assert_eq!(ok.weight, 1.0);

        let err = Evidence::from_tool_result("c2", "bash", "boom", true);
        assert_eq!(err.weight, 0.0);
    }
}
