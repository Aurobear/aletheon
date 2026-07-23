//! Deterministic evaluation engine — fixed-point scoring, gate checks, and report generation.
//!
//! The `DeterministicEvaluator` applies versioned rubrics to pre-scored dimensions
//! and gate results, computing the weighted total, coverage, and eligibility using
//! checked integer arithmetic.

use async_trait::async_trait;
use thiserror::Error;

use fabric::types::metacognition_evaluation::{
    DimensionScore, DimensionValue, EvaluationReport, GateResult, RubricId,
};
use fabric::types::metacognition_evidence::EvidenceItem;
use fabric::types::metacognition_experience::ExperienceEnvelope;

use super::rubric::Rubric;

/// Errors that can occur during evaluation.
#[derive(Debug, Error)]
pub enum EvaluationError {
    #[error("weight overflow: sum of dimension weights exceeds u32::MAX")]
    WeightOverflow,

    #[error("dimension '{0}' not declared in rubric")]
    UnknownDimension(String),

    #[error("dimension '{0}' declared in rubric but not scored")]
    MissingDimension(String),

    #[error("gate '{0}' not declared in rubric")]
    UnknownGate(String),

    #[error("gate '{0}' declared in rubric but not checked")]
    MissingGate(String),

    #[error("evaluator internal error: {0}")]
    Internal(String),
}

/// The evaluator port — domain adapters implement this to score experiences.
#[async_trait]
pub trait Evaluator: Send + Sync {
    async fn evaluate(
        &self,
        experience: &ExperienceEnvelope,
        evidence: &[EvidenceItem],
        rubric: &Rubric,
    ) -> Result<EvaluationReport, EvaluationError>;
}

/// A deterministic evaluator that computes fixed-point evaluation reports
/// from pre-scored dimensions and pre-checked gates.
///
/// This is the pure scoring kernel. Domain adapters produce dimension scores
/// and gate results; this engine validates, computes the weighted total,
/// and determines eligibility.
pub struct DeterministicEvaluator;

impl DeterministicEvaluator {
    pub fn new() -> Self {
        Self
    }

    /// Evaluate pre-scored dimensions and gates against a rubric.
    ///
    /// # Fixed-point calculation
    ///
    /// weighted_total_millis = sum(score[i] * weight[i]) / sum(applicable_weight[i])
    ///
    /// where each score is 0-100 and each weight is in fixed-point millis
    /// (1_000_000 = 1.0). The result is also in fixed-point millis
    /// (100_000 = 100.0).
    ///
    /// Unknown dimensions are excluded from both numerator and denominator.
    ///
    /// # Errors
    ///
    /// Returns `WeightOverflow` if the sum of weights or the weighted sum
    /// exceeds `u32::MAX`.
    /// Returns `UnknownDimension` if a scored dimension is not in the rubric.
    /// Returns `MissingDimension` if a rubric dimension has no score.
    /// Returns `UnknownGate` if a gate result is not in the rubric.
    /// Returns `MissingGate` if a rubric gate has no result.
    pub fn evaluate(
        &self,
        rubric: &Rubric,
        dimension_scores: Vec<DimensionScore>,
        gate_results: Vec<GateResult>,
    ) -> Result<EvaluationReport, EvaluationError> {
        // Validate that all scored dimensions are in the rubric
        for ds in &dimension_scores {
            if !rubric.dimensions.iter().any(|rd| rd.name == ds.name) {
                return Err(EvaluationError::UnknownDimension(ds.name.clone()));
            }
        }

        // Validate that all rubric dimensions have a score
        for rd in &rubric.dimensions {
            if !dimension_scores.iter().any(|ds| ds.name == rd.name) {
                return Err(EvaluationError::MissingDimension(rd.name.clone()));
            }
        }

        // Validate that all gate results match rubric gates
        for gr in &gate_results {
            if !rubric.gates.iter().any(|rg| rg.name == gr.name) {
                return Err(EvaluationError::UnknownGate(gr.name.clone()));
            }
        }

        // Validate that all rubric gates have a result
        for rg in &rubric.gates {
            if !gate_results.iter().any(|gr| gr.name == rg.name) {
                return Err(EvaluationError::MissingGate(rg.name.clone()));
            }
        }

        // Compute weighted total
        // sum(score * weight): u32
        // sum(applicable_weight): u32
        let mut weighted_sum: u32 = 0;
        let mut applicable_weight_sum: u32 = 0;

        for ds in &dimension_scores {
            // Find the matching rubric dimension for weight
            let rd = rubric
                .dimensions
                .iter()
                .find(|rd| rd.name == ds.name)
                .expect("already validated dimension exists in rubric");

            match ds.value {
                DimensionValue::Scored(score) => {
                    // Checked: score (u8, 0-100) * weight (u32)
                    let product = (score as u32)
                        .checked_mul(rd.weight_millis)
                        .ok_or(EvaluationError::WeightOverflow)?;
                    weighted_sum = weighted_sum
                        .checked_add(product)
                        .ok_or(EvaluationError::WeightOverflow)?;
                    applicable_weight_sum = applicable_weight_sum
                        .checked_add(rd.weight_millis)
                        .ok_or(EvaluationError::WeightOverflow)?;
                }
                DimensionValue::Unknown => {
                    // Exclude from both numerator and denominator
                }
            }
        }

        // weighted_total_millis is in fixed-point millis (100_000 = 100.0)
        let weighted_total_millis = if applicable_weight_sum == 0 {
            None
        } else {
            // Use u64 for the division to avoid overflow in intermediate multiplication
            let ratio = (weighted_sum as u64) * 1000 / (applicable_weight_sum as u64);
            // ratio is now in range 0-100_000
            Some(ratio as u32)
        };

        // Evidence coverage: applicable dimensions / total dimensions
        let total_dims = rubric.dimensions.len();
        let applicable_dims = dimension_scores
            .iter()
            .filter(|ds| matches!(ds.value, DimensionValue::Scored(_)))
            .count();

        let evidence_coverage_millis = if total_dims == 0 {
            0u16
        } else {
            ((applicable_dims as u32 * 1000) / total_dims as u32) as u16
        };

        // Confidence: basic model = evidence coverage
        let confidence_millis = evidence_coverage_millis;

        // Eligibility: all gates must pass AND weighted total must exist
        // Check mandatory dimensions: all rubric dimensions that are mandatory
        // must be applicable (scored)
        let all_mandatory_applicable =
            rubric
                .dimensions
                .iter()
                .filter(|rd| rd.mandatory)
                .all(|rd| {
                    dimension_scores.iter().any(|ds| {
                        ds.name == rd.name && matches!(ds.value, DimensionValue::Scored(_))
                    })
                });

        let all_gates_passed = gate_results.iter().all(|g| g.passed);

        let eligible =
            all_mandatory_applicable && all_gates_passed && weighted_total_millis.is_some();

        Ok(EvaluationReport {
            rubric: RubricId(rubric.id.clone()),
            rubric_version: rubric.version,
            dimensions: dimension_scores,
            gates: gate_results,
            weighted_total_millis,
            evidence_coverage_millis,
            confidence_millis,
            eligible,
        })
    }
}

impl Default for DeterministicEvaluator {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::evaluation::rubric::{Rubric, RubricDimension, RubricGate};
    use fabric::types::metacognition_evaluation::DimensionValue;

    fn make_rubric() -> Rubric {
        Rubric {
            id: "test".into(),
            version: 1,
            dimensions: vec![
                RubricDimension {
                    name: "goal".into(),
                    weight_millis: 500_000,
                    mandatory: true,
                },
                RubricDimension {
                    name: "safety".into(),
                    weight_millis: 300_000,
                    mandatory: false,
                },
                RubricDimension {
                    name: "efficiency".into(),
                    weight_millis: 200_000,
                    mandatory: false,
                },
            ],
            gates: vec![RubricGate {
                name: "invariant".into(),
                description: "system invariant holds".into(),
            }],
        }
    }

    #[test]
    fn all_applicable_dimensions() {
        let engine = DeterministicEvaluator::new();
        let rubric = make_rubric();

        let scores = vec![
            DimensionScore {
                name: "goal".into(),
                value: DimensionValue::Scored(80),
                weight_millis: 500_000,
                evidence: vec![],
                reasons: vec!["ok".into()],
            },
            DimensionScore {
                name: "safety".into(),
                value: DimensionValue::Scored(100),
                weight_millis: 300_000,
                evidence: vec![],
                reasons: vec!["safe".into()],
            },
            DimensionScore {
                name: "efficiency".into(),
                value: DimensionValue::Scored(90),
                weight_millis: 200_000,
                evidence: vec![],
                reasons: vec!["fast".into()],
            },
        ];

        let gates = vec![GateResult {
            name: "invariant".into(),
            passed: true,
            evidence: vec![],
        }];

        let report = engine.evaluate(&rubric, scores, gates).unwrap();

        // (80*500000 + 100*300000 + 90*200000) / (500000+300000+200000)
        // = (40000000 + 30000000 + 18000000) / 1000000
        // = 88000000 / 1000000 = 88.0
        // In millis: 88 * 1000 = 88000
        assert_eq!(report.weighted_total_millis, Some(88_000));
        assert_eq!(report.evidence_coverage_millis, 1000);
        assert_eq!(report.confidence_millis, 1000);
        assert!(report.eligible);
    }

    #[test]
    fn one_unknown_dimension() {
        let engine = DeterministicEvaluator::new();
        let rubric = make_rubric();

        let scores = vec![
            DimensionScore {
                name: "goal".into(),
                value: DimensionValue::Scored(100),
                weight_millis: 500_000,
                evidence: vec![],
                reasons: vec!["done".into()],
            },
            DimensionScore {
                name: "safety".into(),
                value: DimensionValue::Unknown,
                weight_millis: 300_000,
                evidence: vec![],
                reasons: vec!["no data".into()],
            },
            DimensionScore {
                name: "efficiency".into(),
                value: DimensionValue::Scored(50),
                weight_millis: 200_000,
                evidence: vec![],
                reasons: vec!["slow".into()],
            },
        ];

        let gates = vec![GateResult {
            name: "invariant".into(),
            passed: true,
            evidence: vec![],
        }];

        let report = engine.evaluate(&rubric, scores, gates).unwrap();

        // Safety is unknown, excluded from denominator
        // (100*500000 + 50*200000) / (500000+200000)
        // = (50000000 + 10000000) / 700000
        // = 60000000 / 700000 ≈ 85.714...
        // In millis: ~85714
        let wt = report.weighted_total_millis.unwrap();
        // Allow small rounding due to integer division
        assert!((85710..85720).contains(&wt), "got {}", wt);

        // 2 applicable out of 3 total → 2/3 ≈ 667
        assert_eq!(report.evidence_coverage_millis, 666);
        assert_eq!(report.confidence_millis, 666);

        // All mandatory (goal) are applicable, gates pass, weighted total exists
        assert!(report.eligible);
    }

    #[test]
    fn zero_applicable_dimensions() {
        let engine = DeterministicEvaluator::new();
        let rubric = make_rubric();

        let scores = vec![
            DimensionScore {
                name: "goal".into(),
                value: DimensionValue::Unknown,
                weight_millis: 500_000,
                evidence: vec![],
                reasons: vec!["none".into()],
            },
            DimensionScore {
                name: "safety".into(),
                value: DimensionValue::Unknown,
                weight_millis: 300_000,
                evidence: vec![],
                reasons: vec!["none".into()],
            },
            DimensionScore {
                name: "efficiency".into(),
                value: DimensionValue::Unknown,
                weight_millis: 200_000,
                evidence: vec![],
                reasons: vec!["none".into()],
            },
        ];

        let gates = vec![GateResult {
            name: "invariant".into(),
            passed: true,
            evidence: vec![],
        }];

        let report = engine.evaluate(&rubric, scores, gates).unwrap();

        assert_eq!(report.weighted_total_millis, None);
        assert_eq!(report.evidence_coverage_millis, 0);
        assert_eq!(report.confidence_millis, 0);
        // All gates pass but weighted total is None → not eligible
        assert!(!report.eligible);
    }

    #[test]
    fn failed_hard_gate() {
        let engine = DeterministicEvaluator::new();
        let rubric = Rubric {
            id: "test".into(),
            version: 1,
            dimensions: vec![
                RubricDimension {
                    name: "goal".into(),
                    weight_millis: 1_000_000,
                    mandatory: true,
                },
                RubricDimension {
                    name: "safety".into(),
                    weight_millis: 0,
                    mandatory: false,
                },
                RubricDimension {
                    name: "efficiency".into(),
                    weight_millis: 0,
                    mandatory: false,
                },
            ],
            gates: vec![RubricGate {
                name: "invariant".into(),
                description: "system invariant holds".into(),
            }],
        };

        let scores = vec![
            DimensionScore {
                name: "goal".into(),
                value: DimensionValue::Scored(95),
                weight_millis: 1_000_000,
                evidence: vec![],
                reasons: vec!["great".into()],
            },
            DimensionScore {
                name: "safety".into(),
                value: DimensionValue::Scored(100),
                weight_millis: 0,
                evidence: vec![],
                reasons: vec!["safe".into()],
            },
            DimensionScore {
                name: "efficiency".into(),
                value: DimensionValue::Scored(100),
                weight_millis: 0,
                evidence: vec![],
                reasons: vec!["fast".into()],
            },
        ];

        let gates = vec![GateResult {
            name: "invariant".into(),
            passed: false,
            evidence: vec![],
        }];

        let report = engine.evaluate(&rubric, scores, gates).unwrap();

        // High score but failed gate
        assert_eq!(report.weighted_total_millis, Some(95_000));
        assert!(!report.eligible);
    }

    #[test]
    fn missing_evidence_dimension_becomes_unknown() {
        let engine = DeterministicEvaluator::new();
        let rubric = make_rubric();

        // When there is no evidence for a dimension, it should be scored as Unknown
        let scores = vec![
            DimensionScore {
                name: "goal".into(),
                value: DimensionValue::Scored(70),
                weight_millis: 500_000,
                evidence: vec![],
                reasons: vec!["partial".into()],
            },
            DimensionScore {
                name: "safety".into(),
                value: DimensionValue::Unknown, // no evidence available
                weight_millis: 300_000,
                evidence: vec![],
                reasons: vec!["no safety log found".into()],
            },
            DimensionScore {
                name: "efficiency".into(),
                value: DimensionValue::Scored(80),
                weight_millis: 200_000,
                evidence: vec![],
                reasons: vec!["ok".into()],
            },
        ];

        let gates = vec![GateResult {
            name: "invariant".into(),
            passed: true,
            evidence: vec![],
        }];

        let report = engine.evaluate(&rubric, scores, gates).unwrap();

        // Safety is Unknown → excluded from weighted total
        let wt = report.weighted_total_millis.unwrap();
        // (70*500000 + 80*200000) / (500000+200000) = (35000000+16000000)/700000 ≈ 72857
        assert!((72850..72860).contains(&wt), "got {}", wt);

        // 2/3 applicable → 666
        assert_eq!(report.evidence_coverage_millis, 666);

        // All mandatory (goal) are applicable
        assert!(report.eligible);
    }

    #[test]
    fn weight_overflow_rejected() {
        let engine = DeterministicEvaluator::new();
        let rubric = Rubric {
            id: "overflow".into(),
            version: 1,
            dimensions: vec![RubricDimension {
                name: "huge".into(),
                weight_millis: u32::MAX,
                mandatory: false,
            }],
            gates: vec![],
        };

        let scores = vec![DimensionScore {
            name: "huge".into(),
            value: DimensionValue::Scored(100),
            weight_millis: u32::MAX,
            evidence: vec![],
            reasons: vec!["overflow".into()],
        }];

        let result = engine.evaluate(&rubric, scores, vec![]);
        assert!(matches!(result, Err(EvaluationError::WeightOverflow)));
    }

    #[test]
    fn unknown_dimension_rejected() {
        let engine = DeterministicEvaluator::new();
        let rubric = make_rubric();

        let scores = vec![DimensionScore {
            name: "not_in_rubric".into(),
            value: DimensionValue::Scored(50),
            weight_millis: 100_000,
            evidence: vec![],
            reasons: vec![],
        }];

        let result = engine.evaluate(&rubric, scores, vec![]);
        assert!(matches!(result, Err(EvaluationError::UnknownDimension(_))));
    }

    #[test]
    fn missing_dimension_rejected() {
        let engine = DeterministicEvaluator::new();
        let rubric = make_rubric();

        // Only score one dimension, but rubric has three
        let scores = vec![DimensionScore {
            name: "goal".into(),
            value: DimensionValue::Scored(50),
            weight_millis: 500_000,
            evidence: vec![],
            reasons: vec![],
        }];

        let result = engine.evaluate(
            &rubric,
            scores,
            vec![GateResult {
                name: "invariant".into(),
                passed: true,
                evidence: vec![],
            }],
        );
        assert!(matches!(result, Err(EvaluationError::MissingDimension(_))));
    }
}
