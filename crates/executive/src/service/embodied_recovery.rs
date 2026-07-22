//! Embodied recovery mapping — converts VerificationReport decisions
//! into concrete recovery actions (retry, replan, SafeStop).

use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// Proceed — outcome matched.
    Proceed,
    /// Retry the same skill with a new operation ID.
    Retry,
    /// Replan — generate a new plan (uses a new admission).
    Replan,
    /// SafeStop immediately — do not retry.
    SafeStop,
    /// Escalate to human/supervisor.
    Escalate,
}

/// Maximum retries and replans (hard limits for P3).
pub const MAX_RETRIES: u32 = 1;
pub const MAX_REPLANS: u32 = 1;

/// Map a verification report to a recovery action, accounting for
/// remaining retries and replans.
pub fn map_verification_to_recovery(
    report: &VerificationReport,
    retries_used: u32,
    replans_used: u32,
) -> RecoveryAction {
    match report.decision {
        VerificationDecision::Matched => RecoveryAction::Proceed,
        VerificationDecision::RetryableMismatch => {
            if retries_used < MAX_RETRIES {
                RecoveryAction::Retry
            } else {
                RecoveryAction::Replan
            }
        }
        VerificationDecision::ReplannableMismatch => {
            if replans_used < MAX_REPLANS {
                RecoveryAction::Replan
            } else {
                RecoveryAction::Escalate
            }
        }
        VerificationDecision::Unsafe => RecoveryAction::SafeStop,
        VerificationDecision::Unknown => {
            // Unknown with remaining retries -> try once more
            if retries_used < MAX_RETRIES {
                RecoveryAction::Retry
            } else {
                RecoveryAction::SafeStop
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn report(decision: VerificationDecision) -> VerificationReport {
        VerificationReport {
            decision,
            evaluated_sequence: 1,
            observed_paths: vec![],
            reasons: vec![],
            evidence: vec![],
        }
    }

    #[test]
    fn matched_proceeds() {
        assert_eq!(
            map_verification_to_recovery(&report(VerificationDecision::Matched), 0, 0),
            RecoveryAction::Proceed
        );
    }

    #[test]
    fn retryable_with_remaining_retries() {
        assert_eq!(
            map_verification_to_recovery(
                &report(VerificationDecision::RetryableMismatch),
                0,
                0
            ),
            RecoveryAction::Retry
        );
    }

    #[test]
    fn retryable_exhausted_replans() {
        assert_eq!(
            map_verification_to_recovery(
                &report(VerificationDecision::RetryableMismatch),
                1,
                0
            ),
            RecoveryAction::Replan
        );
    }

    #[test]
    fn replannable_with_remaining_replan() {
        assert_eq!(
            map_verification_to_recovery(
                &report(VerificationDecision::ReplannableMismatch),
                0,
                0
            ),
            RecoveryAction::Replan
        );
    }

    #[test]
    fn replannable_exhausted_escalates() {
        assert_eq!(
            map_verification_to_recovery(
                &report(VerificationDecision::ReplannableMismatch),
                0,
                1
            ),
            RecoveryAction::Escalate
        );
    }

    #[test]
    fn unsafe_always_safe_stop() {
        // Even with retries remaining, Unsafe -> SafeStop
        assert_eq!(
            map_verification_to_recovery(&report(VerificationDecision::Unsafe), 0, 0),
            RecoveryAction::SafeStop
        );
        assert_eq!(
            map_verification_to_recovery(&report(VerificationDecision::Unsafe), 1, 1),
            RecoveryAction::SafeStop
        );
    }

    #[test]
    fn unknown_with_retries_retries() {
        assert_eq!(
            map_verification_to_recovery(&report(VerificationDecision::Unknown), 0, 0),
            RecoveryAction::Retry
        );
    }

    #[test]
    fn unknown_exhausted_safe_stops() {
        assert_eq!(
            map_verification_to_recovery(&report(VerificationDecision::Unknown), 1, 0),
            RecoveryAction::SafeStop
        );
    }

    #[test]
    fn exhaustion_across_both_dimensions() {
        // Both retries and replans exhausted + Replannable -> Escalate
        assert_eq!(
            map_verification_to_recovery(
                &report(VerificationDecision::ReplannableMismatch),
                1,
                1
            ),
            RecoveryAction::Escalate
        );
    }

    #[test]
    fn retryable_with_both_exhausted_replans_not_escalates() {
        // RetryableMismatch with both exhausted -> still Replan (not Escalate)
        assert_eq!(
            map_verification_to_recovery(
                &report(VerificationDecision::RetryableMismatch),
                1,
                1
            ),
            RecoveryAction::Replan
        );
    }
}
