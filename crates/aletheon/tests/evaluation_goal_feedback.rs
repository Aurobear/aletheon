use ::contracts::{
    AttemptId, CognitiveRole, EvaluationContractId, EvaluationDecision, EvaluationReceiptId,
    EvaluationReceiptRef, GoalBudget, GoalSpec, PrincipalId, RuntimeId, EVALUATION_SCHEMA_V1,
};
use adapters_sqlite::goal::ObjectiveStore;
use tempfile::NamedTempFile;

fn receipt(
    goal: ::contracts::GoalId,
    attempt: AttemptId,
    decision: EvaluationDecision,
) -> EvaluationReceiptRef {
    EvaluationReceiptRef {
        schema_version: EVALUATION_SCHEMA_V1,
        receipt_id: EvaluationReceiptId::new(),
        contract_id: EvaluationContractId::new(),
        subject_kind: "goal_attempt".into(),
        subject_id: format!("{}:{}", goal.0, attempt.0),
        decision,
        weighted_total_millis: Some(50_000),
        evidence_coverage_millis: 900,
        confidence_millis: 900,
        failed_gates: if matches!(
            decision,
            EvaluationDecision::ObservedFail
                | EvaluationDecision::Rejected
                | EvaluationDecision::Indeterminate
        ) {
            vec!["tests_passed".into()]
        } else {
            vec![]
        },
        created_at_ms: 1,
    }
}

fn fixture() -> (
    NamedTempFile,
    ObjectiveStore,
    ::contracts::GoalId,
    AttemptId,
) {
    let file = NamedTempFile::new().unwrap();
    let store = ObjectiveStore::open(file.path()).unwrap();
    let goal = store
        .create_goal(
            &PrincipalId("owner".into()),
            "session",
            "session",
            &GoalSpec {
                original_intent: "fix".into(),
                desired_state: vec![],
                constraints: vec![],
                acceptance_criteria: vec![],
                budget: GoalBudget {
                    max_input_tokens: 100,
                    max_output_tokens: 100,
                    max_cost_usd: None,
                    max_attempts: 3,
                    deadline_ms: None,
                },
            },
        )
        .unwrap();
    let attempt = store
        .begin_attempt(
            goal.id,
            1,
            &RuntimeId("coding".into()),
            CognitiveRole::Worker,
            &serde_json::json!({"task":"fix"}),
        )
        .unwrap();
    (file, store, goal.id, attempt.id)
}

#[test]
fn failing_receipt_creates_one_typed_retry_replan_input() {
    let (_file, store, goal, attempt) = fixture();
    let receipt = receipt(goal, attempt, EvaluationDecision::ObservedFail);
    let feedback = store.record_evaluation_feedback(&receipt).unwrap().unwrap();
    assert!(feedback.retry_replan_required);
    assert_eq!(feedback.failed_gates, vec!["tests_passed"]);
    assert!(store
        .record_evaluation_feedback(&receipt)
        .unwrap()
        .is_none());
}

#[test]
fn passing_receipt_is_recorded_without_retry() {
    let (_file, store, goal, attempt) = fixture();
    let feedback = store
        .record_evaluation_feedback(&receipt(goal, attempt, EvaluationDecision::ObservedPass))
        .unwrap()
        .unwrap();
    assert!(!feedback.retry_replan_required);
    assert!(feedback.failed_gates.is_empty());
}

#[test]
fn receipt_for_unknown_attempt_is_rejected() {
    let (_file, store, goal, _attempt) = fixture();
    let error = store
        .record_evaluation_feedback(&receipt(
            goal,
            AttemptId::new(),
            EvaluationDecision::Rejected,
        ))
        .unwrap_err();
    assert!(error.to_string().contains("does not match"));
}
