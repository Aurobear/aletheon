//! Stable verification decision and report types.
//! No free-form boolean success — every outcome is a tagged decision.

use serde::{Deserialize, Serialize};

use crate::types::embodiment::EvidenceRef;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "decision", rename_all = "snake_case")]
pub enum VerificationDecision {
    /// The observed state matched the expected outcome.
    Matched,
    /// The predicate was not satisfied but the operation can be retried.
    RetryableMismatch,
    /// The predicate was not satisfied and a new plan is needed.
    ReplannableMismatch,
    /// The observed state indicates an unsafe condition — must SafeStop.
    Unsafe,
    /// Verification could not produce a conclusive result (stale, missing, etc.).
    Unknown,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VerificationReport {
    pub decision: VerificationDecision,
    /// Monotonic operation sequence.
    pub evaluated_sequence: u64,
    /// Dot-paths that were observed during evaluation.
    pub observed_paths: Vec<String>,
    /// Human-readable reason (for audit, not for programmatic branching).
    pub reasons: Vec<String>,
    /// Supporting evidence references.
    pub evidence: Vec<EvidenceRef>,
}
