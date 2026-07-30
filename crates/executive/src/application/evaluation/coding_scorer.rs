use fabric::types::metacognition_evaluation::{DimensionScore, DimensionValue, GateResult};
use fabric::types::metacognition_evidence::{EvidenceId, EvidenceItem, EvidenceTrust};
use fabric::TaskEvaluationContract;
use metacog::evaluation::{Rubric, RubricDimension, RubricGate};

use super::evidence_collector::{SCOPE_SUMMARY_SOURCE, VERIFICATION_SUMMARY_SOURCE};
use super::EvaluationApplicationError;

const PRODUCER: &str = "executive.evaluation/coding-v2";

#[derive(Debug, Clone)]
pub struct ScoredEvaluationInput {
    pub dimensions: Vec<DimensionScore>,
    pub gates: Vec<GateResult>,
}

pub trait CodingDimensionScorer: Send + Sync {
    fn score(
        &self,
        contract: &TaskEvaluationContract,
        evidence: &fabric::EvaluationEvidenceSnapshot,
    ) -> Result<ScoredEvaluationInput, EvaluationApplicationError>;
}

#[derive(Debug, Default)]
pub struct CodingV2Scorer;

impl CodingDimensionScorer for CodingV2Scorer {
    fn score(
        &self,
        contract: &TaskEvaluationContract,
        snapshot: &fabric::EvaluationEvidenceSnapshot,
    ) -> Result<ScoredEvaluationInput, EvaluationApplicationError> {
        if contract.rubric.0 != "coding-v2" || contract.rubric_version != 2 {
            return Err(EvaluationApplicationError::UnsupportedRubric(format!(
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
                        == fabric::types::metacognition_evidence::EvidenceKind::VerificationResult
                    && item.trust == EvidenceTrust::Authoritative
            })
            .map(|item| item.evidence_id.clone())
            .collect::<Vec<_>>();
        let validation_count = verification
            .payload
            .get("validation_count")
            .and_then(serde_json::Value::as_u64)
            .ok_or_else(|| EvaluationApplicationError::Scoring("invalid validation_count".into()))?
            as usize;
        if validation_count != validation_ids.len() {
            return Err(EvaluationApplicationError::Scoring(
                "verification summary and terminal receipt count differ".into(),
            ));
        }
        let all_passed = verification
            .payload
            .get("all_passed")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| EvaluationApplicationError::Scoring("invalid all_passed".into()))?;
        let within_scope = scope
            .payload
            .get("within_scope")
            .and_then(serde_json::Value::as_bool)
            .ok_or_else(|| EvaluationApplicationError::Scoring("invalid within_scope".into()))?;
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

        Ok(ScoredEvaluationInput {
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
}

fn summary<'a>(
    snapshot: &'a fabric::EvaluationEvidenceSnapshot,
    source: &str,
) -> Result<&'a EvidenceItem, EvaluationApplicationError> {
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
        return Err(EvaluationApplicationError::Scoring(format!(
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

pub fn coding_v2_rubric() -> Rubric {
    Rubric {
        id: "coding-v2".into(),
        version: 2,
        dimensions: vec![
            dim("requirement_coverage", 200_000, false),
            dim("correctness", 250_000, true),
            dim("scope_discipline", 150_000, true),
            dim("maintainability", 100_000, false),
            dim("verification_sufficiency", 200_000, true),
            dim("regression_safety", 100_000, true),
        ],
        gates: vec![
            gate("required_verification_passed"),
            gate("change_within_scope"),
        ],
    }
}

fn dim(name: &str, weight_millis: u32, mandatory: bool) -> RubricDimension {
    RubricDimension {
        name: name.into(),
        weight_millis,
        mandatory,
    }
}

fn gate(name: &str) -> RubricGate {
    RubricGate {
        name: name.into(),
        description: format!("coding-v2 hard gate: {name}"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::application::evaluation::{
        CodingEvidenceCollector, DefaultCodingEvidenceCollector, TurnEvaluationArtifacts,
    };
    use crate::application::turn_diff_tracker::TurnFileDeltaSnapshot;
    use fabric::types::metacognition_evaluation::RubricId;
    use fabric::{
        CapabilityRetryDisposition, CapabilityTerminalReceipt, CapabilityTerminalStatus,
        EvaluationContractId, EvaluationMode, EvaluationSubject, EvaluationThresholds, EvidenceRef,
        MonoTime, OperationId, ProcessId, RequiredGate, TaskEvaluationContract, TaskKind, TurnId,
        EVALUATION_SCHEMA_V1,
    };

    fn contract() -> TaskEvaluationContract {
        TaskEvaluationContract {
            schema_version: EVALUATION_SCHEMA_V1,
            contract_id: EvaluationContractId::new(),
            task_kind: TaskKind::Coding,
            subject: EvaluationSubject::Turn {
                turn_id: TurnId::new(),
                operation_id: OperationId::new(),
            },
            rubric: RubricId("coding-v2".into()),
            rubric_version: 2,
            mode: EvaluationMode::Shadow,
            objective_ref: EvidenceRef("turn:user-message".into()),
            requirement_refs: vec![],
            required_evidence: vec![],
            required_gates: vec![
                RequiredGate {
                    name: "required_verification_passed".into(),
                },
                RequiredGate {
                    name: "change_within_scope".into(),
                },
            ],
            thresholds: EvaluationThresholds {
                min_score_millis: 70_000,
                min_evidence_coverage_millis: 600,
                min_confidence_millis: 700,
            },
            issued_by: "test".into(),
            issued_at_ms: 1,
        }
    }

    fn validation_receipt(status: CapabilityTerminalStatus) -> CapabilityTerminalReceipt {
        CapabilityTerminalReceipt {
            invocation_id: "validation-1".into(),
            operation_id: OperationId::new(),
            process_id: ProcessId::new(),
            capability: "validation_run".into(),
            status,
            started_at: MonoTime(1),
            finished_at: MonoTime(2),
            exit_code: Some(if status == CapabilityTerminalStatus::Succeeded {
                0
            } else {
                1
            }),
            error_class: None,
            artifact_ids: vec![],
            evidence_ids: vec![],
            output_ref: None,
            truncated: false,
            retry_disposition: CapabilityRetryDisposition::Never,
            audit_id: None,
        }
    }

    fn file_delta(path: &str) -> TurnFileDeltaSnapshot {
        TurnFileDeltaSnapshot {
            path: path.into(),
            edits: 1,
            hunks_applied: 1,
            bytes_before: 1,
            bytes_after: 2,
        }
    }

    fn artifacts(
        receipts: Vec<CapabilityTerminalReceipt>,
        deltas: Vec<TurnFileDeltaSnapshot>,
    ) -> TurnEvaluationArtifacts {
        TurnEvaluationArtifacts {
            workspace: Some(
                fabric::WorkspacePolicy::from_resolved_roots("/tmp/project".into(), vec![])
                    .unwrap(),
            ),
            profile_name: "code-agent".into(),
            capability_receipts: receipts,
            file_deltas: deltas,
            runtime_faults: vec![],
            supplemental_evidence: vec![],
        }
    }

    fn dimension_value(input: &ScoredEvaluationInput, name: &str) -> Option<u8> {
        input
            .dimensions
            .iter()
            .find(|dimension| dimension.name == name)
            .and_then(|dimension| match dimension.value {
                DimensionValue::Scored(score) => Some(score),
                DimensionValue::Unknown => None,
            })
    }

    fn gate_passed(input: &ScoredEvaluationInput, name: &str) -> bool {
        input
            .gates
            .iter()
            .find(|gate| gate.name == name)
            .is_some_and(|gate| gate.passed)
    }

    fn score_artifacts(artifacts: TurnEvaluationArtifacts) -> ScoredEvaluationInput {
        let contract = contract();
        let snapshot = DefaultCodingEvidenceCollector
            .collect(&contract, &artifacts)
            .unwrap();
        CodingV2Scorer.score(&contract, &snapshot).unwrap()
    }

    #[test]
    fn passing_validation_and_in_scope_diff_produce_eligible_input() {
        let contract = contract();
        let artifacts = artifacts(
            vec![validation_receipt(CapabilityTerminalStatus::Succeeded)],
            vec![file_delta("src/lib.rs")],
        );
        let snapshot = DefaultCodingEvidenceCollector
            .collect(&contract, &artifacts)
            .unwrap();
        let scored = CodingV2Scorer.score(&contract, &snapshot).unwrap();
        assert_eq!(dimension_value(&scored, "correctness"), Some(100));
        assert_eq!(dimension_value(&scored, "scope_discipline"), Some(100));
        assert!(gate_passed(&scored, "required_verification_passed"));
        assert!(gate_passed(&scored, "change_within_scope"));

        let report = metacog::evaluation::DeterministicEvaluator::new()
            .evaluate_evidence_backed(
                &coding_v2_rubric(),
                scored.dimensions,
                scored.gates,
                &snapshot.evidence,
                contract.thresholds,
            )
            .unwrap();
        assert!(report.eligible);
    }

    #[test]
    fn missing_validation_is_unknown_and_fails_required_gate() {
        let scored = score_artifacts(artifacts(vec![], vec![file_delta("src/lib.rs")]));
        assert_eq!(dimension_value(&scored, "correctness"), None);
        assert!(!gate_passed(&scored, "required_verification_passed"));
    }

    #[test]
    fn parent_traversal_fails_scope_gate() {
        let scored = score_artifacts(artifacts(
            vec![validation_receipt(CapabilityTerminalStatus::Succeeded)],
            vec![file_delta("../outside.rs")],
        ));
        assert_eq!(dimension_value(&scored, "scope_discipline"), Some(0));
        assert!(!gate_passed(&scored, "change_within_scope"));
    }
}
