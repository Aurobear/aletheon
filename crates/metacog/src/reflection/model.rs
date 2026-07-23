//! Reflection data model — reports, patterns, hypotheses, and input types.

use serde::{Deserialize, Serialize};

pub use crate::improvement::ImprovementProposal;

/// A recurring problem pattern grouped by deterministic category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct RecurringPattern {
    /// The shared category (e.g., "timeout", "assertion_failure").
    pub category: String,
    /// How many times this pattern was observed.
    pub occurrence_count: usize,
    /// Example problem IDs that contributed to this pattern.
    pub example_ids: Vec<String>,
    /// Evidence that explicitly contradicts the pattern (it was not always present).
    pub contrary_evidence: Vec<String>,
}

/// A causal hypothesis — a tentative explanation backed by evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CausalHypothesis {
    /// Human-readable description of the hypothesis.
    pub description: String,
    /// Confidence in fixed-point millis (0-1_000, where 1_000 = 1.0).
    pub confidence_millis: u16,
    /// Evidence problem IDs supporting the hypothesis.
    pub supporting_ids: Vec<String>,
    /// Contrary problem IDs or observations that weaken the hypothesis.
    pub contrary_ids: Vec<String>,
}

/// A minimal problem summary used by the reflection engine.
///
/// This is a lightweight view of a problem record. The real `ProblemRecord`
/// lives in the `problem` module and can be converted into this summary.
/// No floating-point fields — severity is a fixed-point ordinal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProblemSummary {
    /// Stable problem identifier.
    pub problem_id: String,
    /// Domain-neutral category (e.g., "timeout", "correctness", "regression").
    pub category: String,
    /// Lifecycle state (e.g., "confirmed", "active", "resolved").
    pub state: String,
    /// Severity ordinal: 0 = Info, 1 = Low, 2 = Medium, 3 = High, 4 = Critical.
    pub severity_ordinal: u8,
    /// Human-readable summary of the problem.
    pub description: String,
    /// Whether evidence contradicts this problem in some contexts.
    pub has_contrary_evidence: bool,
    /// Contrary evidence descriptions if any.
    pub contrary_evidence: Vec<String>,
}

/// Input to the reflection engine — bundles experiences, evaluations, and problems.
#[derive(Debug, Clone)]
pub struct ReflectionInput {
    /// Experience envelopes to reflect upon.
    pub experiences: Vec<fabric::types::metacognition_experience::ExperienceEnvelope>,
    /// Evaluation reports for the experiences.
    pub evaluations: Vec<fabric::types::metacognition_evaluation::EvaluationReport>,
    /// Problem summaries derived from the problem ledger.
    pub problems: Vec<ProblemSummary>,
}

/// A reflection report produced by the reflection engine.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReflectionReport {
    /// The experience range covered (e.g., "exp-1..exp-5").
    pub experience_range: String,
    /// Identified strengths across the evaluated experiences.
    pub strengths: Vec<String>,
    /// Identified weaknesses across the evaluated experiences.
    pub weaknesses: Vec<String>,
    /// Recurring patterns found in confirmed problems.
    pub recurring_patterns: Vec<RecurringPattern>,
    /// Causal hypotheses with confidence and evidence.
    pub causal_hypotheses: Vec<CausalHypothesis>,
    /// Knowledge gaps — areas where evidence is insufficient.
    pub knowledge_gaps: Vec<String>,
    /// Recommended next observations to gather more evidence.
    pub recommended_observations: Vec<String>,
    /// Governable proposals generated from evidence-backed patterns.
    pub improvement_proposals: Vec<ImprovementProposal>,
}
