//! Coding metacognition rubric — domain-specific evaluation dimensions for coding tasks.
//!
//! This module defines the Coding rubric used by the generic EvaluationEngine.
//! Dimension names and weights are Coding-specific; the rubric struct itself
//! uses only Fabric evaluation contracts.

use fabric::types::metacognition_evaluation::{
    DimensionScore, DimensionValue, EvaluationReport, GateResult,
};

// ---------------------------------------------------------------------------
// Rubric version constant
// ---------------------------------------------------------------------------

/// The current coding rubric version.
pub const CODING_RUBRIC_V1: u32 = 1;

// ---------------------------------------------------------------------------
// Rubric type
// ---------------------------------------------------------------------------

/// A domain-specific evaluation rubric.
///
/// Rubrics define the dimensions to score, their fixed-point weights,
/// and the hard gates that must pass regardless of numeric score.
#[derive(Debug, Clone)]
pub struct Rubric {
    pub id: String,
    pub version: u32,
    pub description: String,
    /// Dimension names with their fixed-point weights (0-1_000_000 millis).
    pub dimensions: Vec<RubricDimension>,
    /// Hard gates that must all pass for eligibility.
    pub gates: Vec<RubricGate>,
    /// Minimum evidence coverage threshold (fixed-point millis, 0-1_000).
    pub min_evidence_coverage_millis: u16,
}

/// A single evaluable dimension within a rubric.
#[derive(Debug, Clone)]
pub struct RubricDimension {
    /// Human-readable dimension name.
    pub name: String,
    /// Weight in fixed-point millis (0-1_000_000, where 1_000_000 = 1.0).
    pub weight_millis: u32,
    /// Description of what this dimension measures.
    pub description: String,
}

/// A hard gate defined by the rubric.
#[derive(Debug, Clone)]
pub struct RubricGate {
    /// Human-readable gate name.
    pub name: String,
    /// Description of what this gate enforces.
    pub description: String,
}

// ---------------------------------------------------------------------------
// Rubric construction
// ---------------------------------------------------------------------------

impl Rubric {
    /// Build an empty score-vector from this rubric (all dimensions Unknown).
    pub fn build_empty_scores(&self) -> Vec<DimensionScore> {
        self.dimensions
            .iter()
            .map(|d| DimensionScore {
                name: d.name.clone(),
                value: DimensionValue::Unknown,
                weight_millis: d.weight_millis,
                evidence: Vec::new(),
                reasons: Vec::new(),
            })
            .collect()
    }

    /// Build an empty gate-result vector (all gates passed=true by default;
    ///  callers must set `passed=false` for any failed check).
    pub fn build_empty_gates(&self) -> Vec<GateResult> {
        self.gates
            .iter()
            .map(|g| GateResult {
                name: g.name.clone(),
                passed: true,
                evidence: Vec::new(),
            })
            .collect()
    }

    /// Build an evaluation report with all dimensions unscored and gates passing.
    /// The caller must fill in actual scores, evidence coverage, confidence, and eligibility.
    pub fn build_empty_report(&self) -> EvaluationReport {
        EvaluationReport {
            rubric: fabric::types::metacognition_evaluation::RubricId(self.id.clone()),
            rubric_version: self.version,
            dimensions: self.build_empty_scores(),
            gates: self.build_empty_gates(),
            weighted_total_millis: None,
            evidence_coverage_millis: 0,
            confidence_millis: 0,
            eligible: false,
        }
    }
}

// ---------------------------------------------------------------------------
// Coding rubric v1
// ---------------------------------------------------------------------------

/// Build the Coding rubric v1 with six dimensions and two hard gates.
///
/// Dimensions:
/// - `requirement_coverage` (weight 200_000): Did the change address all stated requirements?
/// - `correctness` (weight 250_000): Does the change produce correct output and pass tests?
/// - `scope_discipline` (weight 150_000): Did the change stay within allowed scope?
/// - `maintainability` (weight 100_000): Is the change clean and reviewable?
/// - `verification_sufficiency` (weight 200_000): Is there enough verification evidence?
/// - `regression_risk` (weight 100_000): Does the change risk breaking existing behavior?
///
/// Hard gates:
/// - `verification_evidence`: at least one verification or test result must exist.
/// - `change_within_scope`: no forbidden paths were modified.
///
/// Evidence coverage threshold: 400_000 millis (40%).
pub fn coding_rubric_v1() -> Rubric {
    Rubric {
        id: "coding-v1".to_string(),
        version: CODING_RUBRIC_V1,
        description: "Evidence-backed coding quality rubric".to_string(),
        dimensions: vec![
            RubricDimension {
                name: "requirement_coverage".to_string(),
                weight_millis: 200_000,
                description: "Did the change address all stated requirements?".to_string(),
            },
            RubricDimension {
                name: "correctness".to_string(),
                weight_millis: 250_000,
                description: "Does the change produce correct output and pass tests?".to_string(),
            },
            RubricDimension {
                name: "scope_discipline".to_string(),
                weight_millis: 150_000,
                description: "Did the change stay within allowed scope?".to_string(),
            },
            RubricDimension {
                name: "maintainability".to_string(),
                weight_millis: 100_000,
                description: "Is the change clean and reviewable?".to_string(),
            },
            RubricDimension {
                name: "verification_sufficiency".to_string(),
                weight_millis: 200_000,
                description: "Is there enough verification evidence?".to_string(),
            },
            RubricDimension {
                name: "regression_risk".to_string(),
                weight_millis: 100_000,
                description: "Does the change risk breaking existing behavior?".to_string(),
            },
        ],
        gates: vec![
            RubricGate {
                name: "verification_evidence".to_string(),
                description:
                    "At least one verification or test result must exist for this evaluation."
                        .to_string(),
            },
            RubricGate {
                name: "change_within_scope".to_string(),
                description: "No forbidden paths were modified.".to_string(),
            },
        ],
        min_evidence_coverage_millis: 400, // 40% minimum coverage
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rubric_v1_has_six_dimensions_and_two_gates() {
        let rubric = coding_rubric_v1();
        assert_eq!(rubric.id, "coding-v1");
        assert_eq!(rubric.version, CODING_RUBRIC_V1);
        assert_eq!(rubric.dimensions.len(), 6);
        assert_eq!(rubric.gates.len(), 2);
        assert_eq!(rubric.min_evidence_coverage_millis, 400);
    }

    #[test]
    fn empty_scores_are_all_unknown() {
        let rubric = coding_rubric_v1();
        let scores = rubric.build_empty_scores();
        assert_eq!(scores.len(), 6);
        for s in &scores {
            assert!(matches!(s.value, DimensionValue::Unknown));
        }
    }

    #[test]
    fn empty_gates_start_passing() {
        let rubric = coding_rubric_v1();
        let gates = rubric.build_empty_gates();
        assert_eq!(gates.len(), 2);
        assert!(gates.iter().all(|g| g.passed));
    }

    #[test]
    fn empty_report_is_ineligible() {
        let rubric = coding_rubric_v1();
        let report = rubric.build_empty_report();
        assert!(!report.eligible);
        assert_eq!(report.weighted_total_millis, None);
        assert_eq!(report.confidence_millis, 0);
        assert_eq!(report.evidence_coverage_millis, 0);
    }

    #[test]
    fn rubric_dimension_names_are_coding_specific() {
        let rubric = coding_rubric_v1();
        let names: Vec<&str> = rubric.dimensions.iter().map(|d| d.name.as_str()).collect();
        assert!(names.contains(&"requirement_coverage"));
        assert!(names.contains(&"correctness"));
        assert!(names.contains(&"scope_discipline"));
        assert!(names.contains(&"maintainability"));
        assert!(names.contains(&"verification_sufficiency"));
        assert!(names.contains(&"regression_risk"));
    }

    /// A completion message without tool/test evidence must produce low coverage.
    /// The rubric itself defines the threshold; actual evaluation happens in the
    /// EvaluationEngine. This test verifies the rubric is configured correctly.
    #[test]
    fn completion_only_message_produces_low_evidence_coverage_cap() {
        let rubric = coding_rubric_v1();
        let report = rubric.build_empty_report();
        // No evidence attached → coverage is 0
        assert_eq!(report.evidence_coverage_millis, 0);
        // Below minimum threshold
        assert!(
            report.evidence_coverage_millis < rubric.min_evidence_coverage_millis,
            "Empty report (completion-only message) must be below evidence coverage threshold"
        );
        assert!(!report.eligible);
    }

    /// Verify all dimension weights sum to a recognizable total.
    #[test]
    fn dimension_weights_sum_to_1_0() {
        let rubric = coding_rubric_v1();
        let total: u32 = rubric.dimensions.iter().map(|d| d.weight_millis).sum();
        assert_eq!(
            total, 1_000_000,
            "Coding rubric dimension weights must sum to 1_000_000 millis (1.0)"
        );
    }

    /// Gate names are stable and discoverable.
    #[test]
    fn gate_names_are_discoverable() {
        let rubric = coding_rubric_v1();
        let names: Vec<&str> = rubric.gates.iter().map(|g| g.name.as_str()).collect();
        assert!(names.contains(&"verification_evidence"));
        assert!(names.contains(&"change_within_scope"));
    }
}
