use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::outcome_verification::VerificationDecision;
use fabric::types::embodiment::DeviceId;
use fabric::types::world_state::WorldSnapshot;
use metacog::outcome_verifier;

fn snapshot(payload: serde_json::Value, seq: u64, stale: bool) -> WorldSnapshot {
    WorldSnapshot {
        device: DeviceId("bot".into()),
        schema: "test".into(),
        sequence: seq,
        payload,
        observed_at: fabric::MonoTime(seq),
        stale,
    }
}

#[test]
fn equals_matches_when_value_equal() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Equals { path: "mode".into(), value: serde_json::json!("stance") },
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let after = snapshot(serde_json::json!({"mode": "stance"}), 1, false);
    let report = outcome_verifier::evaluate(&eo, None, Some(&after), 1);
    assert_eq!(report.decision, VerificationDecision::Matched);
}

#[test]
fn equals_mismatch_on_wrong_value() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Equals { path: "mode".into(), value: serde_json::json!("stance") },
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let after = snapshot(serde_json::json!({"mode": "walking"}), 1, false);
    let report = outcome_verifier::evaluate(&eo, None, Some(&after), 1);
    assert_eq!(report.decision, VerificationDecision::RetryableMismatch);
}

#[test]
fn mismatch_after_retry_becomes_replannable() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Equals { path: "x".into(), value: serde_json::json!(1) },
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let after = snapshot(serde_json::json!({"x": 2}), 1, false);
    // Attempt 2 (third try) -> ReplannableMismatch
    let report = outcome_verifier::evaluate(&eo, None, Some(&after), 2);
    assert_eq!(report.decision, VerificationDecision::ReplannableMismatch);
}

#[test]
fn range_inclusive_bounds() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Range { path: "x".into(), min: Some(0.0), max: Some(10.0) },
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let after = snapshot(serde_json::json!({"x": 5.0}), 1, false);
    assert_eq!(outcome_verifier::evaluate(&eo, None, Some(&after), 1).decision, VerificationDecision::Matched);

    // boundary
    let after2 = snapshot(serde_json::json!({"x": 0.0}), 1, false);
    assert_eq!(outcome_verifier::evaluate(&eo, None, Some(&after2), 1).decision, VerificationDecision::Matched);
}

#[test]
fn range_out_of_bounds() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Range { path: "x".into(), min: Some(0.0), max: Some(10.0) },
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let after = snapshot(serde_json::json!({"x": 15.0}), 1, false);
    assert_eq!(outcome_verifier::evaluate(&eo, None, Some(&after), 1).decision, VerificationDecision::RetryableMismatch);
}

#[test]
fn change_delta_detected() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Change { path: "x".into(), min_delta: Some(1.0), max_delta: None },
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let before = snapshot(serde_json::json!({"x": 0.0}), 0, false);
    let after = snapshot(serde_json::json!({"x": 3.0}), 1, false);
    assert_eq!(outcome_verifier::evaluate(&eo, Some(&before), Some(&after), 1).decision, VerificationDecision::Matched);
}

#[test]
fn change_no_delta_is_mismatch() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Change { path: "x".into(), min_delta: Some(1.0), max_delta: None },
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let before = snapshot(serde_json::json!({"x": 0.0}), 0, false);
    let after = snapshot(serde_json::json!({"x": 0.5}), 1, false);
    assert_eq!(outcome_verifier::evaluate(&eo, Some(&before), Some(&after), 1).decision, VerificationDecision::RetryableMismatch);
}

#[test]
fn all_nested_predicates() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::All { predicates: vec![
            OutcomePredicate::Equals { path: "mode".into(), value: serde_json::json!("stance") },
            OutcomePredicate::Range { path: "x".into(), min: Some(0.0), max: Some(5.0) },
        ]},
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let after = snapshot(serde_json::json!({"mode": "stance", "x": 2.0}), 1, false);
    assert_eq!(outcome_verifier::evaluate(&eo, None, Some(&after), 1).decision, VerificationDecision::Matched);
}

#[test]
fn any_predicate_one_matches() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Any { predicates: vec![
            OutcomePredicate::Equals { path: "mode".into(), value: serde_json::json!("stance") },
            OutcomePredicate::Equals { path: "mode".into(), value: serde_json::json!("walk") },
        ]},
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let after = snapshot(serde_json::json!({"mode": "walk"}), 1, false);
    assert_eq!(outcome_verifier::evaluate(&eo, None, Some(&after), 1).decision, VerificationDecision::Matched);
}

#[test]
fn stale_snapshot_is_unknown() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Equals { path: "x".into(), value: serde_json::json!(1) },
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let after = snapshot(serde_json::json!({"x": 1}), 1, true);
    assert_eq!(outcome_verifier::evaluate(&eo, None, Some(&after), 1).decision, VerificationDecision::Unknown);
}

#[test]
fn fault_field_maps_to_unsafe() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Equals { path: "mode".into(), value: serde_json::json!("stance") },
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let after = snapshot(serde_json::json!({"fault": "motor_stall"}), 1, false);
    assert_eq!(outcome_verifier::evaluate(&eo, None, Some(&after), 1).decision, VerificationDecision::Unsafe);
}

#[test]
fn missing_path_is_unknown() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Equals { path: "nonexistent".into(), value: serde_json::json!(1) },
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let after = snapshot(serde_json::json!({"x": 1}), 1, false);
    assert_eq!(outcome_verifier::evaluate(&eo, None, Some(&after), 1).decision, VerificationDecision::Unknown);
}

#[test]
fn type_mismatch_path_is_unknown() {
    let eo = ExpectedOutcome {
        predicate: OutcomePredicate::Range { path: "x".into(), min: Some(0.0), max: None },
        freshness_ms: 500, stable_window_ms: 200, timeout_ms: 5000,
    };
    let after = snapshot(serde_json::json!({"x": "not_a_number"}), 1, false);
    assert_eq!(outcome_verifier::evaluate(&eo, None, Some(&after), 1).decision, VerificationDecision::Unknown);
}
