//! Adapter from Application scoring port to the Metacog coding-v2 domain policy.

use ::contracts::TaskEvaluationContract;
use application::evaluation::{
    CodingDimensionScorer, EvaluationApplicationError, ScoredEvaluationInput,
};

#[derive(Debug, Default)]
pub struct CodingV2Scorer;

impl CodingDimensionScorer for CodingV2Scorer {
    fn score(
        &self,
        contract: &TaskEvaluationContract,
        snapshot: &::contracts::EvaluationEvidenceSnapshot,
    ) -> Result<ScoredEvaluationInput, EvaluationApplicationError> {
        metacog::evaluation::score_coding_v2(contract, snapshot)
            .map(|scored| ScoredEvaluationInput {
                dimensions: scored.dimensions,
                gates: scored.gates,
            })
            .map_err(|error| EvaluationApplicationError::Scoring(error.to_string()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::types::metacognition_evaluation::DimensionValue;
    use ::contracts::types::metacognition_evaluation::RubricId;
    use ::contracts::{
        CapabilityRetryDisposition, CapabilityTerminalReceipt, CapabilityTerminalStatus,
        EvaluationContractId, EvaluationMode, EvaluationSubject, EvaluationThresholds, EvidenceRef,
        MonoTime, OperationId, ProcessId, RequiredGate, TaskEvaluationContract, TaskKind, TurnId,
        EVALUATION_SCHEMA_V1,
    };
    use application::evaluation::{
        CodingEvidenceCollector, DefaultCodingEvidenceCollector, TurnEvaluationArtifacts,
    };
    use runtime::turn_diff_tracker::TurnFileDeltaSnapshot;

    fn collector() -> DefaultCodingEvidenceCollector {
        DefaultCodingEvidenceCollector::new(
            std::sync::Arc::new(kernel::chronos::TestClock::new(1, 0)),
            std::sync::Arc::new(platform::evaluation_path::PlatformEvaluationPathResolver),
        )
    }

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
            session_id: "test-session".into(),
            runtime_id: "test-runtime".into(),
            effective_model_id: "test-provider/test-model".into(),
            model_display_name: "test-model".into(),
            workspace: Some(
                ::contracts::WorkspacePolicy::from_resolved_roots("/tmp/project".into(), vec![])
                    .unwrap(),
            ),
            profile_name: "code-agent".into(),
            capability_receipts: receipts,
            file_deltas: deltas,
            runtime_faults: vec![],
            supplemental_evidence: vec![],
            projection_metrics: Default::default(),
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
        let snapshot = collector().collect(&contract, &artifacts).unwrap();
        CodingV2Scorer.score(&contract, &snapshot).unwrap()
    }

    #[test]
    fn passing_validation_and_in_scope_diff_produce_eligible_input() {
        let contract = contract();
        let artifacts = artifacts(
            vec![validation_receipt(CapabilityTerminalStatus::Succeeded)],
            vec![file_delta("src/lib.rs")],
        );
        let snapshot = collector().collect(&contract, &artifacts).unwrap();
        let scored = CodingV2Scorer.score(&contract, &snapshot).unwrap();
        assert_eq!(dimension_value(&scored, "correctness"), Some(100));
        assert_eq!(dimension_value(&scored, "scope_discipline"), Some(100));
        assert!(gate_passed(&scored, "required_verification_passed"));
        assert!(gate_passed(&scored, "change_within_scope"));

        let report = metacog::evaluation::DeterministicEvaluator::new()
            .evaluate_evidence_backed(
                &metacog::evaluation::coding_v2_rubric(),
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
