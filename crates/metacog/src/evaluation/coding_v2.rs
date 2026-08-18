//! Coding-v2 evidence-to-dimension scoring domain policy.

use ::contracts::types::metacognition_evaluation::{DimensionScore, DimensionValue, GateResult};
use ::contracts::types::metacognition_evidence::{EvidenceId, EvidenceItem, EvidenceTrust};
use ::contracts::TaskEvaluationContract;

pub const VERIFICATION_SUMMARY_SOURCE: &str = "coding_v2.verification_summary";
pub const SCOPE_SUMMARY_SOURCE: &str = "coding_v2.scope_summary";

#[derive(Debug, Clone)]
pub struct CodingV2Score {
    pub dimensions: Vec<DimensionScore>,
    pub gates: Vec<GateResult>,
}

#[derive(Debug, thiserror::Error)]
pub enum CodingV2ScoreError {
    #[error("unsupported evaluation rubric: {0}")]
    UnsupportedRubric(String),
    #[error("evaluation contract is invalid: {0}")]
    Contract(#[from] ::contracts::EvaluationContractError),
    #[error("evaluation scoring failed: {0}")]
    Scoring(String),
}

const PRODUCER: &str = "executive.evaluation/coding-v2";

pub fn score_coding_v2(
    contract: &TaskEvaluationContract,
    snapshot: &::contracts::EvaluationEvidenceSnapshot,
) -> Result<CodingV2Score, CodingV2ScoreError> {
    if contract.rubric.0 != "coding-v2" || contract.rubric_version != 2 {
        return Err(CodingV2ScoreError::UnsupportedRubric(format!(
            "{}@{}",
            contract.rubric.0, contract.rubric_version
        )));
    }
    snapshot.validate()?;
    let verification = summary(snapshot, VERIFICATION_SUMMARY_SOURCE)?;
    let scope = summary(snapshot, SCOPE_SUMMARY_SOURCE)?;
    let validation_ids = snapshot
        .evidence
        .iter()
        .filter(|item| {
            item.producer == PRODUCER
                && item.source == "capability_terminal_receipt"
                && item.kind
                    == ::contracts::types::metacognition_evidence::EvidenceKind::VerificationResult
                && item.trust == EvidenceTrust::Authoritative
        })
        .map(|item| item.evidence_id.clone())
        .collect::<Vec<_>>();
    let validation_count = verification
        .payload
        .get("validation_count")
        .and_then(serde_json::Value::as_u64)
        .ok_or_else(|| CodingV2ScoreError::Scoring("invalid validation_count".into()))?
        as usize;
    if validation_count != validation_ids.len() {
        return Err(CodingV2ScoreError::Scoring(
            "verification summary and terminal receipt count differ".into(),
        ));
    }
    let all_passed = verification
        .payload
        .get("all_passed")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| CodingV2ScoreError::Scoring("invalid all_passed".into()))?;
    let within_scope = scope
        .payload
        .get("within_scope")
        .and_then(serde_json::Value::as_bool)
        .ok_or_else(|| CodingV2ScoreError::Scoring("invalid within_scope".into()))?;
    let verification_summary_id = vec![verification.evidence_id.clone()];
    let scope_summary_id = vec![scope.evidence_id.clone()];
    let correctness = if validation_count == 0 {
        DimensionValue::Unknown
    } else if all_passed {
        DimensionValue::Scored(100)
    } else {
        DimensionValue::Scored(0)
    };
    let correctness_evidence = if validation_count == 0 {
        Vec::new()
    } else {
        validation_ids.clone()
    };
    let verification_score = if all_passed { 100 } else { 0 };

    Ok(CodingV2Score {
        dimensions: vec![
            dimension(
                "requirement_coverage",
                200_000,
                DimensionValue::Unknown,
                vec![],
            ),
            dimension("correctness", 250_000, correctness, correctness_evidence),
            dimension(
                "scope_discipline",
                150_000,
                DimensionValue::Scored(if within_scope { 100 } else { 0 }),
                scope_summary_id.clone(),
            ),
            dimension("maintainability", 100_000, DimensionValue::Unknown, vec![]),
            dimension(
                "verification_sufficiency",
                200_000,
                DimensionValue::Scored(verification_score),
                verification_summary_id.clone(),
            ),
            dimension(
                "regression_safety",
                100_000,
                DimensionValue::Scored(verification_score),
                verification_summary_id.clone(),
            ),
        ],
        gates: vec![
            GateResult {
                name: "required_verification_passed".into(),
                passed: all_passed,
                evidence: verification_summary_id,
            },
            GateResult {
                name: "change_within_scope".into(),
                passed: within_scope,
                evidence: scope_summary_id,
            },
        ],
    })
}

fn summary<'a>(
    snapshot: &'a ::contracts::EvaluationEvidenceSnapshot,
    source: &str,
) -> Result<&'a EvidenceItem, CodingV2ScoreError> {
    let matches = snapshot
        .evidence
        .iter()
        .filter(|item| {
            item.producer == PRODUCER
                && item.source == source
                && item.trust == EvidenceTrust::Authoritative
        })
        .collect::<Vec<_>>();
    if matches.len() != 1 {
        return Err(CodingV2ScoreError::Scoring(format!(
            "expected one {source}, found {}",
            matches.len()
        )));
    }
    Ok(matches[0])
}

fn dimension(
    name: &str,
    weight_millis: u32,
    value: DimensionValue,
    evidence: Vec<EvidenceId>,
) -> DimensionScore {
    DimensionScore {
        name: name.into(),
        value,
        weight_millis,
        evidence,
        reasons: Vec::new(),
    }
}
