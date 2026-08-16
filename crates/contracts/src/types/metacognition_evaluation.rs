//! Generic metacognition evaluation contracts — rubrics and evaluation reports.
//!
//! These types form the stable Fabric ABI that evaluators produce and
//! downstream governance consumes. Domain-specific rubric logic must
//! never enter this module.

use serde::{Deserialize, Serialize};

use super::metacognition_evidence::EvidenceId;

/// Identifies a versioned evaluation rubric.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub struct RubricId(pub String);

/// The scored value for a single dimension.
///
/// A dimension may be scored (0-100) or unknown.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DimensionValue {
    /// A score in the range 0-100.
    Scored(u8),
    /// The dimension could not be evaluated with available evidence.
    Unknown,
}

/// A scored dimension with name, weight, evidence, and reasons.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DimensionScore {
    /// Human-readable dimension name (e.g., "goal_attainment").
    pub name: String,
    /// The scored value or Unknown.
    pub value: DimensionValue,
    /// Weight in fixed-point millis (0-1_000_000, where 1_000_000 = 1.0).
    pub weight_millis: u32,
    /// Evidence supporting this score.
    pub evidence: Vec<EvidenceId>,
    /// Human-readable reasons for the score.
    pub reasons: Vec<String>,
}

/// Result of a hard gate check.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GateResult {
    /// Human-readable gate name.
    pub name: String,
    /// Whether the gate passed.
    pub passed: bool,
    /// Evidence supporting the gate result.
    pub evidence: Vec<EvidenceId>,
}

/// An evaluation report produced by applying a rubric to an experience and its evidence.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EvaluationReport {
    /// The rubric identifier used.
    pub rubric: RubricId,
    /// The rubric version applied.
    pub rubric_version: u32,
    /// Individual dimension scores.
    pub dimensions: Vec<DimensionScore>,
    /// Hard gate results.
    pub gates: Vec<GateResult>,
    /// Weighted total in fixed-point millis (0-100_000, where 100_000 = 100.0).
    /// None when no dimensions are applicable.
    pub weighted_total_millis: Option<u32>,
    /// Evidence coverage in fixed-point millis (0-1_000, where 1_000 = 1.0).
    pub evidence_coverage_millis: u16,
    /// Evaluator confidence in fixed-point millis (0-1_000, where 1_000 = 1.0).
    pub confidence_millis: u16,
    /// Whether the evaluation is eligible for further governance (all gates passed + weighted total exists).
    pub eligible: bool,
}
