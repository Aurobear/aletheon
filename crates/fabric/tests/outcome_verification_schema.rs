use fabric::types::embodiment::EvidenceRef;
use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};

#[test]
fn verification_decision_serde_roundtrip() {
    for decision in &[
        VerificationDecision::Matched,
        VerificationDecision::RetryableMismatch,
        VerificationDecision::ReplannableMismatch,
        VerificationDecision::Unsafe,
        VerificationDecision::Unknown,
    ] {
        let json = serde_json::to_string(decision).unwrap();
        let back: VerificationDecision = serde_json::from_str(&json).unwrap();
        assert_eq!(*decision, back);
    }
}

#[test]
fn verification_report_serde_roundtrip() {
    let report = VerificationReport {
        decision: VerificationDecision::Matched,
        evaluated_sequence: 42,
        observed_paths: vec!["pose.x".into(), "pose.y".into()],
        reasons: vec!["pose within tolerance".into()],
        evidence: vec![EvidenceRef {
            kind: "rosbag".into(),
            uri: "artifact://b/1".into(),
        }],
    };
    let json = serde_json::to_string(&report).unwrap();
    let back: VerificationReport = serde_json::from_str(&json).unwrap();
    assert_eq!(report, back);
}

#[test]
fn verification_decision_is_exhaustive() {
    let json = serde_json::json!({"decision": "matched"});
    let decision: VerificationDecision = serde_json::from_value(json).unwrap();
    assert_eq!(decision, VerificationDecision::Matched);

    let json = serde_json::json!({"decision": "retryable_mismatch"});
    let decision: VerificationDecision = serde_json::from_value(json).unwrap();
    assert_eq!(decision, VerificationDecision::RetryableMismatch);

    let json = serde_json::json!({"decision": "replannable_mismatch"});
    let decision: VerificationDecision = serde_json::from_value(json).unwrap();
    assert_eq!(decision, VerificationDecision::ReplannableMismatch);

    let json = serde_json::json!({"decision": "unsafe"});
    let decision: VerificationDecision = serde_json::from_value(json).unwrap();
    assert_eq!(decision, VerificationDecision::Unsafe);

    let json = serde_json::json!({"decision": "unknown"});
    let decision: VerificationDecision = serde_json::from_value(json).unwrap();
    assert_eq!(decision, VerificationDecision::Unknown);
}
