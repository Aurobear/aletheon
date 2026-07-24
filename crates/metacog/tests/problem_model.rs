//! Problem model integration tests — lifecycle transitions and fingerprinting.

use metacog::problem::{
    problem_fingerprint, ProblemRecord, ProblemSeverity, ProblemState, ProblemTransition,
};

// ---------------------------------------------------------------------------
// Lifecycle tests
// ---------------------------------------------------------------------------

#[test]
fn valid_observed_to_confirmed_transition() {
    assert!(ProblemRecord::is_valid_transition(
        ProblemState::Observed,
        ProblemState::Confirmed
    ));
}

#[test]
fn valid_confirmed_to_active_transition() {
    assert!(ProblemRecord::is_valid_transition(
        ProblemState::Confirmed,
        ProblemState::Active
    ));
}

#[test]
fn valid_active_to_mitigated_transition() {
    assert!(ProblemRecord::is_valid_transition(
        ProblemState::Active,
        ProblemState::Mitigated
    ));
}

#[test]
fn valid_mitigated_to_resolved_transition() {
    assert!(ProblemRecord::is_valid_transition(
        ProblemState::Mitigated,
        ProblemState::Resolved
    ));
}

#[test]
fn valid_resolved_to_regressed_transition() {
    // Resolved -> Regressed is allowed (problem came back)
    assert!(ProblemRecord::is_valid_transition(
        ProblemState::Resolved,
        ProblemState::Regressed
    ));
}

#[test]
fn invalid_resolved_to_active_transition() {
    // Resolved -> Active should be rejected (must go through Regressed)
    assert!(!ProblemRecord::is_valid_transition(
        ProblemState::Resolved,
        ProblemState::Active
    ));
}

#[test]
fn valid_regressed_to_active_transition() {
    // Regressed -> Active to re-open the problem
    assert!(ProblemRecord::is_valid_transition(
        ProblemState::Regressed,
        ProblemState::Active
    ));
}

#[test]
fn valid_observed_to_disputed_transition() {
    assert!(ProblemRecord::is_valid_transition(
        ProblemState::Observed,
        ProblemState::Disputed
    ));
}

#[test]
fn valid_active_to_accepted_risk_transition() {
    assert!(ProblemRecord::is_valid_transition(
        ProblemState::Active,
        ProblemState::AcceptedRisk
    ));
}

#[test]
fn valid_confirmed_to_disputed_transition() {
    assert!(ProblemRecord::is_valid_transition(
        ProblemState::Confirmed,
        ProblemState::Disputed
    ));
}

#[test]
fn invalid_active_to_resolved_without_mitigation() {
    // Cannot skip mitigation
    assert!(!ProblemRecord::is_valid_transition(
        ProblemState::Active,
        ProblemState::Resolved
    ));
}

#[test]
fn invalid_backwards_disputed_to_observed() {
    assert!(!ProblemRecord::is_valid_transition(
        ProblemState::Disputed,
        ProblemState::Observed
    ));
}

// ---------------------------------------------------------------------------
// Fingerprint tests
// ---------------------------------------------------------------------------

#[test]
fn identical_inputs_produce_identical_fingerprints() {
    let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error:E0308", 1);
    let fp2 = problem_fingerprint("coding", "rustc", "correctness", "type_error:E0308", 1);
    assert_eq!(fp1, fp2);
}

#[test]
fn different_domain_produces_different_fingerprint() {
    let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error", 1);
    let fp2 = problem_fingerprint("robot", "rustc", "correctness", "type_error", 1);
    assert_ne!(fp1, fp2);
}

#[test]
fn different_subject_produces_different_fingerprint() {
    let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error", 1);
    let fp2 = problem_fingerprint("coding", "clippy", "correctness", "type_error", 1);
    assert_ne!(fp1, fp2);
}

#[test]
fn different_category_produces_different_fingerprint() {
    let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error", 1);
    let fp2 = problem_fingerprint("coding", "rustc", "efficiency", "type_error", 1);
    assert_ne!(fp1, fp2);
}

#[test]
fn different_failure_signature_produces_different_fingerprint() {
    let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error:E0308", 1);
    let fp2 = problem_fingerprint("coding", "rustc", "correctness", "type_error:E0499", 1);
    assert_ne!(fp1, fp2);
}

#[test]
fn rubric_version_change_produces_different_fingerprint() {
    let fp1 = problem_fingerprint("coding", "rustc", "correctness", "type_error", 1);
    let fp2 = problem_fingerprint("coding", "rustc", "correctness", "type_error", 2);
    assert_ne!(fp1, fp2);
}

#[test]
fn fingerprint_is_stable_hex_string() {
    let fp = problem_fingerprint("coding", "rustc", "correctness", "type_error", 1);
    assert_eq!(fp.len(), 64);
    assert!(fp.chars().all(|c| c.is_ascii_hexdigit()));
}

// ---------------------------------------------------------------------------
// ProblemRecord serialization
// ---------------------------------------------------------------------------

#[test]
fn problem_record_roundtrips_through_json() {
    let record = ProblemRecord {
        problem_id: "p-1".into(),
        category: "correctness".into(),
        subtype: "compilation_error".into(),
        domain: "coding".into(),
        subject: "rustc".into(),
        severity: ProblemSeverity::High,
        confidence_millis: 900,
        state: ProblemState::Observed,
        first_seen_at_ms: 100,
        last_seen_at_ms: 200,
        occurrence_count: 5,
        affected_versions: vec!["v1.0".into(), "v1.1".into()],
        expected_summary: "clean compile".into(),
        observed_summary: "type error at line 42".into(),
        failure_signature: "E0308:42".into(),
        evidence_ids: vec!["ev-1".into()],
        causal_hypotheses: vec!["incorrect type inference".into()],
        related_problem_ids: vec!["p-2".into()],
        proposed_mitigations: vec!["add type annotation".into()],
        resolution_evidence: vec![],
        regression_evidence: vec![],
    };

    let json = serde_json::to_string(&record).unwrap();
    let rt: ProblemRecord = serde_json::from_str(&json).unwrap();
    assert_eq!(record, rt);
    assert_eq!(rt.state, ProblemState::Observed);
    assert_eq!(rt.severity, ProblemSeverity::High);
}

#[test]
fn problem_transition_roundtrips_through_json() {
    let transition = ProblemTransition {
        problem_id: "p-1".into(),
        event_id: "evt-1".into(),
        old_state: ProblemState::Observed,
        new_state: ProblemState::Confirmed,
        reason: "reproduced in isolation".into(),
        evidence_ids: vec!["ev-2".into()],
        timestamp_ms: 300,
    };

    let json = serde_json::to_string(&transition).unwrap();
    let rt: ProblemTransition = serde_json::from_str(&json).unwrap();
    assert_eq!(transition, rt);
}
