//! Integration tests for embodied recovery mapping.

use executive::application::embodied_recovery::{
    map_verification_to_recovery, RecoveryAction, MAX_REPLANS, MAX_RETRIES,
};
use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};

#[test]
fn hard_limits_are_p3_constants() {
    assert_eq!(MAX_RETRIES, 1);
    assert_eq!(MAX_REPLANS, 1);
}

#[test]
fn full_recovery_table() {
    // Test the complete decision->action table
    let cases = vec![
        (VerificationDecision::Matched, 0, 0, RecoveryAction::Proceed),
        (VerificationDecision::Matched, 1, 1, RecoveryAction::Proceed),
        (
            VerificationDecision::RetryableMismatch,
            0,
            0,
            RecoveryAction::Retry,
        ),
        (
            VerificationDecision::RetryableMismatch,
            1,
            0,
            RecoveryAction::Replan,
        ),
        (
            VerificationDecision::ReplannableMismatch,
            0,
            0,
            RecoveryAction::Replan,
        ),
        (
            VerificationDecision::ReplannableMismatch,
            0,
            1,
            RecoveryAction::Escalate,
        ),
        (VerificationDecision::Unsafe, 0, 0, RecoveryAction::SafeStop),
        (VerificationDecision::Unsafe, 1, 1, RecoveryAction::SafeStop),
        (VerificationDecision::Unknown, 0, 0, RecoveryAction::Retry),
        (
            VerificationDecision::Unknown,
            1,
            0,
            RecoveryAction::SafeStop,
        ),
    ];

    for (decision, retries, replans, expected) in cases {
        let report = VerificationReport {
            decision: decision.clone(),
            evaluated_sequence: 1,
            observed_paths: vec![],
            reasons: vec![],
            evidence: vec![],
        };
        let action = map_verification_to_recovery(&report, retries, replans);
        assert_eq!(
            action, expected,
            "mismatch for {decision:?} r={retries} p={replans}"
        );
    }
}
