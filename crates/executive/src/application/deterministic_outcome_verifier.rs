//! Deterministic outcome verifier for embodied skills.
//!
//! Evaluates a policy proposal's `ExpectedOutcome` against before/after world
//! snapshots using fabric's dot-path predicate evaluation. Unsafe predicates
//! (fall detection, estop, provider disconnect) take priority over success
//! predicates. This is the only source of truth for embodied verification —
//! provider RPC success, a lone `SkillResult::Succeeded`, or LLM text never
//! count as evidence here.

use std::sync::Arc;

use async_trait::async_trait;
use cognit::harness::robot::OutcomeVerifierPort;
use fabric::types::expected_outcome::{
    evaluate_expected, evaluate_predicate, OutcomeMatch, OutcomePredicate,
};
use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};
use fabric::types::world_state::WorldSnapshot;
use fabric::Clock;

/// Deterministic verifier implementing `cognit::harness::robot::OutcomeVerifierPort`.
pub struct DeterministicOutcomeVerifier {
    clock: Arc<dyn Clock>,
    /// Predicates that, when matched on the after snapshot, force `Unsafe`
    /// regardless of the expected outcome. Populated from domain/config input.
    unsafe_predicates: Vec<OutcomePredicate>,
}

impl DeterministicOutcomeVerifier {
    pub fn new(clock: Arc<dyn Clock>, unsafe_predicates: Vec<OutcomePredicate>) -> Self {
        Self {
            clock,
            unsafe_predicates,
        }
    }

    fn report(
        &self,
        decision: VerificationDecision,
        reasons: Vec<String>,
        after: Option<&WorldSnapshot>,
    ) -> VerificationReport {
        VerificationReport {
            decision,
            evaluated_sequence: after.map(|snap| snap.sequence).unwrap_or(0),
            observed_paths: vec![],
            reasons,
            evidence: vec![],
        }
    }
}

#[async_trait]
impl OutcomeVerifierPort for DeterministicOutcomeVerifier {
    async fn verify(
        &self,
        expected: &fabric::types::expected_outcome::ExpectedOutcome,
        before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        _attempt: u32,
    ) -> VerificationReport {
        // Unsafe predicates take priority over every success predicate.
        if let Some(after_snap) = after {
            for predicate in &self.unsafe_predicates {
                if evaluate_predicate(predicate, &after_snap.payload, None) {
                    return self.report(
                        VerificationDecision::Unsafe,
                        vec!["unsafe predicate matched after execution".into()],
                        Some(after_snap),
                    );
                }
            }
        }

        let Some(after_snap) = after else {
            return self.report(
                VerificationDecision::Unknown,
                vec!["no after observation available".into()],
                None,
            );
        };

        let now = self.clock.mono_now();
        match evaluate_expected(expected, after_snap, before, now) {
            OutcomeMatch::Stale => self.report(
                VerificationDecision::Unknown,
                vec!["after observation stale or older than freshness window".into()],
                Some(after_snap),
            ),
            OutcomeMatch::Mismatch { reason } => self.report(
                VerificationDecision::ReplannableMismatch,
                vec![reason],
                Some(after_snap),
            ),
            OutcomeMatch::Match => {
                // Stable window: `after` must be at least stable_window_ms later
                // than `before`. A shortfall means we have not yet observed a
                // stable window — retryable, not replan.
                if let Some(before_snap) = before {
                    let delta_ms = after_snap
                        .observed_at
                        .0
                        .saturating_sub(before_snap.observed_at.0);
                    if delta_ms < expected.stable_window_ms {
                        return self.report(
                            VerificationDecision::RetryableMismatch,
                            vec![format!(
                                "stable window not satisfied: after-before delta {delta_ms}ms < required {}ms",
                                expected.stable_window_ms
                            )],
                            Some(after_snap),
                        );
                    }
                }
                self.report(
                    VerificationDecision::Matched,
                    vec!["expected outcome matched within stable window".into()],
                    Some(after_snap),
                )
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::embodiment::DeviceId;
    use fabric::types::expected_outcome::ExpectedOutcome;
    use fabric::MonoTime;
    use kernel::chronos::TestClock;

    fn snapshot(seq: u64, observed_at: u64, stale: bool, payload: serde_json::Value) -> WorldSnapshot {
        WorldSnapshot {
            device: DeviceId("bot".into()),
            schema: "robot.state/v1".into(),
            sequence: seq,
            payload,
            observed_at: MonoTime(observed_at),
            stale,
        }
    }

    fn stance_expected() -> ExpectedOutcome {
        ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("stance"),
            },
            freshness_ms: 500,
            stable_window_ms: 100,
            timeout_ms: 10_000,
        }
    }

    fn verifier(now_ms: u64, unsafe_predicates: Vec<OutcomePredicate>) -> DeterministicOutcomeVerifier {
        DeterministicOutcomeVerifier::new(
            Arc::new(TestClock::new(0, now_ms)),
            unsafe_predicates,
        )
    }

    #[tokio::test]
    async fn missing_after_is_unknown() {
        let v = verifier(100, vec![]);
        let report = v.verify(&stance_expected(), None, None, 1).await;
        assert_eq!(report.decision, VerificationDecision::Unknown);
    }

    #[tokio::test]
    async fn stale_after_is_unknown() {
        let v = verifier(10_000, vec![]);
        let after = snapshot(1, 0, false, serde_json::json!({"mode": "stance"}));
        let report = v.verify(&stance_expected(), None, Some(&after), 1).await;
        assert_eq!(report.decision, VerificationDecision::Unknown);
    }

    #[tokio::test]
    async fn unsafe_predicate_takes_priority() {
        let v = verifier(
            100,
            vec![OutcomePredicate::Equals {
                path: "fall_detected".into(),
                value: serde_json::json!(true),
            }],
        );
        let before = snapshot(0, 0, false, serde_json::json!({"mode": "stance", "fall_detected": false}));
        let after = snapshot(1, 200, false, serde_json::json!({"mode": "stance", "fall_detected": true}));
        let report = v.verify(&stance_expected(), Some(&before), Some(&after), 1).await;
        assert_eq!(report.decision, VerificationDecision::Unsafe);
    }

    #[tokio::test]
    async fn matched_within_stable_window() {
        let v = verifier(300, vec![]);
        let before = snapshot(0, 100, false, serde_json::json!({"mode": "stance"}));
        let after = snapshot(1, 300, false, serde_json::json!({"mode": "stance"}));
        let report = v.verify(&stance_expected(), Some(&before), Some(&after), 1).await;
        assert_eq!(report.decision, VerificationDecision::Matched);
        assert_eq!(report.evaluated_sequence, 1);
    }

    #[tokio::test]
    async fn matched_but_stable_window_shortfall_is_retryable() {
        let v = verifier(300, vec![]);
        let before = snapshot(0, 250, false, serde_json::json!({"mode": "stance"}));
        let after = snapshot(1, 300, false, serde_json::json!({"mode": "stance"}));
        let report = v.verify(&stance_expected(), Some(&before), Some(&after), 1).await;
        assert_eq!(report.decision, VerificationDecision::RetryableMismatch);
    }

    #[tokio::test]
    async fn predicate_mismatch_is_replannable() {
        let v = verifier(300, vec![]);
        let before = snapshot(0, 100, false, serde_json::json!({"mode": "stance"}));
        let after = snapshot(1, 300, false, serde_json::json!({"mode": "walk"}));
        let report = v.verify(&stance_expected(), Some(&before), Some(&after), 2).await;
        assert_eq!(report.decision, VerificationDecision::ReplannableMismatch);
    }
}
