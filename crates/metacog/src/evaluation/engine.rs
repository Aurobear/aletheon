//! Deterministic evaluation engine — fixed-point scoring, gate checks, and report generation.
//!
//! The `DeterministicEvaluator` applies versioned rubrics to pre-scored dimensions
//! and gate results, computing the weighted total, coverage, and eligibility using
//! checked integer arithmetic.

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use thiserror::Error;

use ::contracts::types::metacognition_evaluation::{
    DimensionScore, DimensionValue, EvaluationReport, GateResult, RubricId,
};
use ::contracts::types::metacognition_evidence::{EvidenceId, EvidenceItem, EvidenceTrust};
use ::contracts::types::metacognition_experience::ExperienceEnvelope;
use ::contracts::EvaluationThresholds;

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

    #[error("dimension '{0}' was scored more than once")]
    DuplicateDimension(String),

    #[error("dimension '{0}' reports a weight different from its rubric")]
    DimensionWeightMismatch(String),

    #[error("dimension '{0}' score exceeds 100")]
    InvalidScore(String),

    #[error("gate '{0}' not declared in rubric")]
    UnknownGate(String),

    #[error("gate '{0}' declared in rubric but not checked")]
    MissingGate(String),

    #[error("gate '{0}' was checked more than once")]
    DuplicateGate(String),

    #[error("scored dimension '{0}' has no evidence")]
    MissingEvidence(String),

    #[error("gate '{0}' has no evidence")]
    MissingGateEvidence(String),

    #[error("evidence reference '{0}' is not present in the supplied snapshot")]
    UnknownEvidenceReference(String),

    #[error("evidence '{0}' has unsupported unverified trust")]
    UnsupportedEvidenceTrust(String),

    #[error("evidence '{0}' payload digest does not match")]
    EvidenceDigestMismatch(String),

    #[error("evidence '{0}' appears more than once")]
    DuplicateEvidence(String),

    #[error("invalid evaluation thresholds: {0}")]
    InvalidThreshold(String),

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
    /// `weighted_total_millis = sum(score[i] * weight[i]) / sum(applicable_weight[i])`
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
        let mut rubric_dimensions = HashSet::with_capacity(rubric.dimensions.len());
        for dimension in &rubric.dimensions {
            if !rubric_dimensions.insert(dimension.name.clone()) {
                return Err(EvaluationError::DuplicateDimension(dimension.name.clone()));
            }
        }
        let mut rubric_gates = HashSet::with_capacity(rubric.gates.len());
        for gate in &rubric.gates {
            if !rubric_gates.insert(gate.name.clone()) {
                return Err(EvaluationError::DuplicateGate(gate.name.clone()));
            }
        }
        let mut seen_dimensions = HashSet::with_capacity(dimension_scores.len());
        // Validate that all scored dimensions are unique and in the rubric.
        for ds in &dimension_scores {
            if !seen_dimensions.insert(ds.name.clone()) {
                return Err(EvaluationError::DuplicateDimension(ds.name.clone()));
            }
            if !rubric.dimensions.iter().any(|rd| rd.name == ds.name) {
                return Err(EvaluationError::UnknownDimension(ds.name.clone()));
            }
            let declared = rubric
                .dimensions
                .iter()
                .find(|dimension| dimension.name == ds.name)
                .expect("dimension presence was just checked");
            if ds.weight_millis != declared.weight_millis {
                return Err(EvaluationError::DimensionWeightMismatch(ds.name.clone()));
            }
            if matches!(ds.value, DimensionValue::Scored(score) if score > 100) {
                return Err(EvaluationError::InvalidScore(ds.name.clone()));
            }
        }

        // Validate that all rubric dimensions have a score
        for rd in &rubric.dimensions {
            if !dimension_scores.iter().any(|ds| ds.name == rd.name) {
                return Err(EvaluationError::MissingDimension(rd.name.clone()));
            }
        }

        let mut seen_gates = HashSet::with_capacity(gate_results.len());
        // Validate that all gate results are unique and match rubric gates.
        for gr in &gate_results {
            if !seen_gates.insert(gr.name.clone()) {
                return Err(EvaluationError::DuplicateGate(gr.name.clone()));
            }
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

    /// Validate evidence integrity and references before applying contract
    /// eligibility thresholds to a structurally valid evaluation report.
    pub fn evaluate_evidence_backed(
        &self,
        rubric: &Rubric,
        dimension_scores: Vec<DimensionScore>,
        gate_results: Vec<GateResult>,
        evidence: &[EvidenceItem],
        thresholds: EvaluationThresholds,
    ) -> Result<EvaluationReport, EvaluationError> {
        thresholds
            .validate()
            .map_err(|error| EvaluationError::InvalidThreshold(error.to_string()))?;
        let mut report = self.evaluate(rubric, dimension_scores, gate_results)?;
        let evidence_by_id = validate_evidence_set(evidence)?;

        let total_weight = rubric
            .dimensions
            .iter()
            .try_fold(0u32, |total, dimension| {
                total
                    .checked_add(dimension.weight_millis)
                    .ok_or(EvaluationError::WeightOverflow)
            })?;
        let mut covered_weight = 0u32;
        let mut confidence_weighted_sum = 0u64;

        for dimension in report
            .dimensions
            .iter()
            .filter(|dimension| matches!(dimension.value, DimensionValue::Scored(_)))
        {
            if dimension.evidence.is_empty() {
                return Err(EvaluationError::MissingEvidence(dimension.name.clone()));
            }
            let trust = resolve_reference_trust(&dimension.evidence, &evidence_by_id)?;
            let rubric_dimension = rubric
                .dimensions
                .iter()
                .find(|candidate| candidate.name == dimension.name)
                .expect("structural evaluation verified the dimension");
            covered_weight = covered_weight
                .checked_add(rubric_dimension.weight_millis)
                .ok_or(EvaluationError::WeightOverflow)?;
            confidence_weighted_sum = confidence_weighted_sum
                .checked_add(u64::from(rubric_dimension.weight_millis) * u64::from(trust))
                .ok_or(EvaluationError::WeightOverflow)?;
        }

        for gate in &report.gates {
            if gate.evidence.is_empty() {
                return Err(EvaluationError::MissingGateEvidence(gate.name.clone()));
            }
            resolve_reference_trust(&gate.evidence, &evidence_by_id)?;
        }

        report.evidence_coverage_millis = if total_weight == 0 {
            0
        } else {
            ((u64::from(covered_weight) * 1_000) / u64::from(total_weight)) as u16
        };
        report.confidence_millis = if covered_weight == 0 {
            0
        } else {
            (confidence_weighted_sum / u64::from(covered_weight)) as u16
        };

        let score_eligible = report
            .weighted_total_millis
            .is_some_and(|score| score >= thresholds.min_score_millis);
        let coverage_eligible =
            report.evidence_coverage_millis >= thresholds.min_evidence_coverage_millis;
        let confidence_eligible = report.confidence_millis >= thresholds.min_confidence_millis;
        report.eligible &= score_eligible && coverage_eligible && confidence_eligible;
        Ok(report)
    }
}

fn validate_evidence_set(
    evidence: &[EvidenceItem],
) -> Result<HashMap<EvidenceId, &EvidenceItem>, EvaluationError> {
    let mut by_id = HashMap::with_capacity(evidence.len());
    for item in evidence {
        if by_id.insert(item.evidence_id.clone(), item).is_some() {
            return Err(EvaluationError::DuplicateEvidence(
                item.evidence_id.0.clone(),
            ));
        }
        let bytes = serde_json::to_vec(&item.payload)
            .map_err(|error| EvaluationError::Internal(error.to_string()))?;
        if format!("{:x}", Sha256::digest(bytes)) != item.sha256 {
            return Err(EvaluationError::EvidenceDigestMismatch(
                item.evidence_id.0.clone(),
            ));
        }
    }
    Ok(by_id)
}

fn resolve_reference_trust(
    references: &[EvidenceId],
    evidence_by_id: &HashMap<EvidenceId, &EvidenceItem>,
) -> Result<u16, EvaluationError> {
    let mut trust_sum = 0u32;
    for reference in references {
        let item = evidence_by_id
            .get(reference)
            .ok_or_else(|| EvaluationError::UnknownEvidenceReference(reference.0.clone()))?;
        let trust = match item.trust {
            EvidenceTrust::Authoritative => 1_000,
            EvidenceTrust::Corroborated => 700,
            EvidenceTrust::Unverified => {
                return Err(EvaluationError::UnsupportedEvidenceTrust(
                    reference.0.clone(),
                ));
            }
        };
        trust_sum = trust_sum
            .checked_add(trust)
            .ok_or(EvaluationError::WeightOverflow)?;
    }
    Ok((trust_sum / references.len() as u32) as u16)
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
    use ::contracts::types::metacognition_evaluation::DimensionValue;

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
        assert!((85710..85720).contains(&wt), "got {wt}");

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
        assert!((72850..72860).contains(&wt), "got {wt}");

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

    fn evidence(id: &str, trust: EvidenceTrust) -> EvidenceItem {
        let payload = serde_json::json!({"id": id});
        let bytes = serde_json::to_vec(&payload).unwrap();
        EvidenceItem {
            schema_version: 1,
            evidence_id: EvidenceId(id.into()),
            experience_id: ::contracts::types::metacognition_experience::ExperienceId(
                "evaluation-test".into(),
            ),
            kind: ::contracts::types::metacognition_evidence::EvidenceKind::VerificationResult,
            source: "test".into(),
            producer: "test".into(),
            captured_at_ms: 1,
            payload,
            sha256: format!("{:x}", Sha256::digest(bytes)),
            trust,
            freshness_ms: Some(0),
            redacted: false,
        }
    }

    fn evidence_backed_input(reference: EvidenceId) -> (Vec<DimensionScore>, Vec<GateResult>) {
        (
            vec![
                DimensionScore {
                    name: "goal".into(),
                    value: DimensionValue::Unknown,
                    weight_millis: 500_000,
                    evidence: vec![],
                    reasons: vec![],
                },
                DimensionScore {
                    name: "safety".into(),
                    value: DimensionValue::Unknown,
                    weight_millis: 300_000,
                    evidence: vec![],
                    reasons: vec![],
                },
                DimensionScore {
                    name: "efficiency".into(),
                    value: DimensionValue::Scored(90),
                    weight_millis: 200_000,
                    evidence: vec![reference.clone()],
                    reasons: vec![],
                },
            ],
            vec![GateResult {
                name: "invariant".into(),
                passed: true,
                evidence: vec![reference],
            }],
        )
    }

    fn permissive_thresholds() -> EvaluationThresholds {
        EvaluationThresholds {
            min_score_millis: 0,
            min_evidence_coverage_millis: 0,
            min_confidence_millis: 0,
        }
    }

    #[test]
    fn evidence_backed_evaluation_rejects_unknown_reference() {
        let reference = EvidenceId("missing".into());
        let (scores, gates) = evidence_backed_input(reference);
        let result = DeterministicEvaluator::new().evaluate_evidence_backed(
            &make_rubric(),
            scores,
            gates,
            &[],
            permissive_thresholds(),
        );
        assert!(matches!(
            result,
            Err(EvaluationError::UnknownEvidenceReference(_))
        ));
    }

    #[test]
    fn coverage_is_weight_based_and_confidence_uses_evidence_trust() {
        let item = evidence("verified", EvidenceTrust::Authoritative);
        let (scores, gates) = evidence_backed_input(item.evidence_id.clone());
        let report = DeterministicEvaluator::new()
            .evaluate_evidence_backed(
                &make_rubric(),
                scores,
                gates,
                &[item],
                permissive_thresholds(),
            )
            .unwrap();
        assert_eq!(report.evidence_coverage_millis, 200);
        assert_eq!(report.confidence_millis, 1_000);
    }

    #[test]
    fn unverified_evidence_cannot_back_a_score() {
        let item = evidence("claim", EvidenceTrust::Unverified);
        let (scores, gates) = evidence_backed_input(item.evidence_id.clone());
        let result = DeterministicEvaluator::new().evaluate_evidence_backed(
            &make_rubric(),
            scores,
            gates,
            &[item],
            permissive_thresholds(),
        );
        assert!(matches!(
            result,
            Err(EvaluationError::UnsupportedEvidenceTrust(_))
        ));
    }

    #[test]
    fn evidence_thresholds_are_part_of_eligibility() {
        let item = evidence("verified", EvidenceTrust::Corroborated);
        let (scores, gates) = evidence_backed_input(item.evidence_id.clone());
        let report = DeterministicEvaluator::new()
            .evaluate_evidence_backed(
                &make_rubric(),
                scores,
                gates,
                &[item],
                EvaluationThresholds {
                    min_score_millis: 70_000,
                    min_evidence_coverage_millis: 600,
                    min_confidence_millis: 700,
                },
            )
            .unwrap();
        assert_eq!(report.confidence_millis, 700);
        assert!(!report.eligible);
    }
}
