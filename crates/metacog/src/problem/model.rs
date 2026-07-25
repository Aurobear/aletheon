//! Problem data model — records, states, severities, and lifecycle transitions.
//!
//! Per spec §6: Problems are durable, append-only findings with typed lifecycle states.

use serde::{Deserialize, Serialize};

/// Lifecycle state for a problem record.
///
/// ```text
/// Observed -> Confirmed -> Active -> Mitigated -> Resolved
///     |          |           |          |           |
///     +-------> Disputed     +-------> AcceptedRisk
///                                       |
///                                       +---------> Regressed
/// ```
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemState {
    /// Initial observation — not yet confirmed.
    Observed,
    /// Observation has been verified and the problem is confirmed.
    Confirmed,
    /// Problem is actively being addressed.
    Active,
    /// Mitigation has been applied but not yet proven resolved.
    Mitigated,
    /// Problem is fully resolved.
    Resolved,
    /// Observation is contested.
    Disputed,
    /// The problem is acknowledged but accepted as a risk.
    AcceptedRisk,
    /// A previously resolved or mitigated problem has reappeared.
    Regressed,
}

/// Severity classification for a problem.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ProblemSeverity {
    /// Informational — no action required.
    Info,
    /// Low severity.
    Low,
    /// Medium severity.
    Medium,
    /// High severity — needs attention.
    High,
    /// Critical — must be addressed immediately.
    Critical,
}

/// A durable problem record (per spec §6).
///
/// Records are append-only events projected into current state.
/// Corrections append new facts; they do not rewrite history.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProblemRecord {
    /// Stable problem identifier.
    pub problem_id: String,
    /// Domain-neutral category (e.g., "correctness", "safety", "efficiency").
    pub category: String,
    /// Domain-specific subtype (e.g., "compilation_error").
    pub subtype: String,
    /// The domain where this problem was observed.
    pub domain: String,
    /// The subject (capability or component) affected.
    pub subject: String,
    /// Severity classification.
    pub severity: ProblemSeverity,
    /// Confidence in the problem classification (0-1_000 millis).
    pub confidence_millis: u16,
    /// Current lifecycle state.
    pub state: ProblemState,
    /// Timestamp (ms since epoch) when first observed.
    pub first_seen_at_ms: i64,
    /// Timestamp (ms since epoch) when last observed.
    pub last_seen_at_ms: i64,
    /// Number of times this problem has been observed.
    pub occurrence_count: u64,
    /// Affected runtime versions.
    pub affected_versions: Vec<String>,
    /// Summary of what was expected.
    pub expected_summary: String,
    /// Summary of what was observed.
    pub observed_summary: String,
    /// Normalized failure signature for fingerprinting.
    pub failure_signature: String,
    /// Evidence references supporting this problem.
    pub evidence_ids: Vec<String>,
    /// Causal hypotheses (explicitly marked as hypotheses).
    pub causal_hypotheses: Vec<String>,
    /// Links to related problem IDs.
    pub related_problem_ids: Vec<String>,
    /// Proposed mitigations.
    pub proposed_mitigations: Vec<String>,
    /// Resolution evidence (when resolved).
    pub resolution_evidence: Vec<String>,
    /// Regression evidence (when regressed).
    pub regression_evidence: Vec<String>,
}

/// A state transition event recorded in the problem ledger.
///
/// Every transition is an append-only event with old/new state, reason,
/// evidence, and timestamp.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProblemTransition {
    /// The problem identifier being transitioned.
    pub problem_id: String,
    /// Unique event identifier for this transition.
    pub event_id: String,
    /// Previous state.
    pub old_state: ProblemState,
    /// New state.
    pub new_state: ProblemState,
    /// Reason for the transition.
    pub reason: String,
    /// Evidence references supporting the transition.
    pub evidence_ids: Vec<String>,
    /// Timestamp (ms since epoch) of the transition.
    pub timestamp_ms: i64,
}

impl ProblemRecord {
    /// Check whether a state transition is valid.
    ///
    /// Allowed transitions:
    /// - Observed -> Confirmed, Disputed
    /// - Confirmed -> Active, Disputed
    /// - Active -> Mitigated, AcceptedRisk
    /// - Mitigated -> Resolved, Regressed
    /// - Resolved -> Regressed
    /// - Disputed -> Confirmed (re-confirmation), Resolved (error)
    /// - AcceptedRisk -> Regressed
    /// - Regressed -> Active (re-open)
    pub fn is_valid_transition(from: ProblemState, to: ProblemState) -> bool {
        matches!(
            (from, to),
            (ProblemState::Observed, ProblemState::Confirmed)
                | (ProblemState::Observed, ProblemState::Disputed)
                | (ProblemState::Confirmed, ProblemState::Active)
                | (ProblemState::Confirmed, ProblemState::Disputed)
                | (ProblemState::Active, ProblemState::Mitigated)
                | (ProblemState::Active, ProblemState::AcceptedRisk)
                | (ProblemState::Mitigated, ProblemState::Resolved)
                | (ProblemState::Mitigated, ProblemState::Regressed)
                | (ProblemState::Resolved, ProblemState::Regressed)
                | (ProblemState::Disputed, ProblemState::Confirmed)
                | (ProblemState::Disputed, ProblemState::Resolved)
                | (ProblemState::AcceptedRisk, ProblemState::Regressed)
                | (ProblemState::Regressed, ProblemState::Active)
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn allowed_transitions() {
        assert!(ProblemRecord::is_valid_transition(
            ProblemState::Observed,
            ProblemState::Confirmed
        ));
        assert!(ProblemRecord::is_valid_transition(
            ProblemState::Confirmed,
            ProblemState::Active
        ));
        assert!(ProblemRecord::is_valid_transition(
            ProblemState::Active,
            ProblemState::Mitigated
        ));
        assert!(ProblemRecord::is_valid_transition(
            ProblemState::Mitigated,
            ProblemState::Resolved
        ));
        assert!(ProblemRecord::is_valid_transition(
            ProblemState::Resolved,
            ProblemState::Regressed
        ));
        assert!(ProblemRecord::is_valid_transition(
            ProblemState::Regressed,
            ProblemState::Active
        ));
    }

    #[test]
    fn rejected_transitions() {
        // Resolved -> Active should be rejected (must go through Regressed)
        assert!(!ProblemRecord::is_valid_transition(
            ProblemState::Resolved,
            ProblemState::Active
        ));
        // Confirmed -> Resolved should be rejected (no mitigation)
        assert!(!ProblemRecord::is_valid_transition(
            ProblemState::Confirmed,
            ProblemState::Resolved
        ));
        // Active -> Resolved should be rejected (no mitigation)
        assert!(!ProblemRecord::is_valid_transition(
            ProblemState::Active,
            ProblemState::Resolved
        ));
    }
}
