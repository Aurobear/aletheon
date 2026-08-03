//! Deterministic outcome verifier for embodied skills.
//!
//! Evaluates a policy proposal's `ExpectedOutcome` against world-state
//! observations using fabric's dot-path predicate evaluation. The verifier
//! WAITS for post-execution observations via `WorldStatePort::observe_until`
//! and requires a continuous stable window — it never settles on a single
//! before/after time delta. Unsafe predicates (fall detection, estop, provider
//! disconnect) take priority over success predicates. This is the only source
//! of truth for embodied verification — provider RPC success, a lone
//! `SkillResult::Succeeded`, or LLM text never count as evidence here.

use std::sync::Arc;

use async_trait::async_trait;
use cognit::harness::robot::OutcomeVerifierPort;
use fabric::types::embodiment::DeviceId;
use fabric::types::expected_outcome::{
    evaluate_expected, evaluate_predicate, OutcomeMatch, OutcomePredicate,
};
use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};
use fabric::types::world_state::{WorldSnapshot, WorldStatePort};
use fabric::{Clock, MonoDeadline};

/// Deterministic verifier implementing `cognit::harness::robot::OutcomeVerifierPort`.
pub struct DeterministicOutcomeVerifier {
    world: Arc<dyn WorldStatePort>,
    clock: Arc<dyn Clock>,
    /// Predicates that, when matched on an observation, force `Unsafe`
    /// regardless of the expected outcome. Populated from domain/config input.
    unsafe_predicates: Vec<OutcomePredicate>,
}

impl DeterministicOutcomeVerifier {
    pub fn new(
        world: Arc<dyn WorldStatePort>,
        clock: Arc<dyn Clock>,
        unsafe_predicates: Vec<OutcomePredicate>,
    ) -> Self {
        Self {
            world,
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
        device: &DeviceId,
        before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        attempt: u32,
    ) -> VerificationReport {
        let start = self.clock.mono_now();
        let deadline = MonoDeadline::after(start, expected.timeout_ms);
        let after_sequence = after
            .as_ref()
            .map(|snap| snap.sequence)
            .or_else(|| before.as_ref().map(|snap| snap.sequence))
            .unwrap_or(0);

        // Wait for post-execution observations and evaluate a CONTINUOUS stable
        // window: `stable_window_ms` of elapsed observation time with the
        // predicate holding on every sample. Any mismatch resets the window.
        let mut window_started_at: Option<u64> = None;
        let mut last_sequence = after_sequence;
        loop {
            let now = self.clock.mono_now();
            if deadline.is_expired_at(now) {
                break;
            }
            let Some(snapshot) = self
                .world
                .observe_until(device, last_sequence, deadline)
                .await
            else {
                break; // no new observation before the deadline
            };
            last_sequence = snapshot.sequence;

            if snapshot.stale || now.0.saturating_sub(snapshot.observed_at.0) > expected.freshness_ms
            {
                return self.report(
                    VerificationDecision::Unknown,
                    vec!["after observation stale or older than freshness window".into()],
                    Some(&snapshot),
                );
            }

            // Unsafe predicates take priority over every success predicate.
            for predicate in &self.unsafe_predicates {
                if evaluate_predicate(predicate, &snapshot.payload, None) {
                    return self.report(
                        VerificationDecision::Unsafe,
                        vec!["unsafe predicate matched after execution".into()],
                        Some(&snapshot),
                    );
                }
            }

            match evaluate_expected(expected, &snapshot, before, now) {
                OutcomeMatch::Match => {
                    let window_start = *window_started_at.get_or_insert(snapshot.observed_at.0);
                    if snapshot.observed_at.0.saturating_sub(window_start) >= expected.stable_window_ms
                    {
                        return self.report(
                            VerificationDecision::Matched,
                            vec!["expected outcome matched within the stable window".into()],
                            Some(&snapshot),
                        );
                    }
                }
                OutcomeMatch::Mismatch { .. } => {
                    window_started_at = None;
                }
                OutcomeMatch::Stale => {
                    return self.report(
                        VerificationDecision::Unknown,
                        vec!["observation became stale during the stable window".into()],
                        Some(&snapshot),
                    );
                }
            }
        }

        // Timed out without satisfying the stable window. A shortfall while the
        // budget remains is retryable; a persistent predicate mismatch that never
        // stabilises is replannable.
        let decision = if attempt <= 1 {
            VerificationDecision::RetryableMismatch
        } else {
            VerificationDecision::ReplannableMismatch
        };
        self.report(
            decision,
            vec!["stable window not satisfied before timeout".into()],
            None,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::embodiment::DeviceId;
    use fabric::types::expected_outcome::ExpectedOutcome;
    use fabric::MonoTime;
    use kernel::chronos::TestClock;
    use std::collections::VecDeque;
    use std::sync::Mutex;

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
            freshness_ms: 5_000,
            stable_window_ms: 100,
            timeout_ms: 10_000,
        }
    }

    /// World that serves a queued sequence of observations then stays silent.
    struct QueueWorld {
        queue: Mutex<VecDeque<WorldSnapshot>>,
    }
    impl QueueWorld {
        fn new(obs: Vec<WorldSnapshot>) -> Self {
            Self {
                queue: Mutex::new(VecDeque::from(obs)),
            }
        }
    }
    #[async_trait::async_trait]
    impl WorldStatePort for QueueWorld {
        async fn latest(&self, _device: &DeviceId) -> Option<WorldSnapshot> {
            self.queue.lock().unwrap().front().cloned()
        }
        async fn observe_until(
            &self,
            _device: &DeviceId,
            after_sequence: u64,
            _deadline: MonoDeadline,
        ) -> Option<WorldSnapshot> {
            self.queue
                .lock()
                .unwrap()
                .iter()
                .find(|s| s.sequence > after_sequence)
                .cloned()
        }
    }

    fn verifier(
        world: Arc<dyn WorldStatePort>,
        now_ms: u64,
        unsafe_predicates: Vec<OutcomePredicate>,
    ) -> DeterministicOutcomeVerifier {
        DeterministicOutcomeVerifier::new(world, Arc::new(TestClock::new(0, now_ms)), unsafe_predicates)
    }

    #[tokio::test]
    async fn missing_observation_before_deadline_is_retryable() {
        let v = verifier(Arc::new(QueueWorld::new(vec![])), 100, vec![]);
        let report = v.verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1).await;
        assert_eq!(report.decision, VerificationDecision::RetryableMismatch);
    }

    #[tokio::test]
    async fn stale_or_freshness_expired_observation_is_unknown() {
        let world = Arc::new(QueueWorld::new(vec![snapshot(1, 0, false, serde_json::json!({"mode": "stance"}))]));
        // now = 60_000 >> freshness 5_000 -> too old.
        let v = verifier(world.clone(), 60_000, vec![]);
        let report = v.verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1).await;
        assert_eq!(report.decision, VerificationDecision::Unknown);
        // Explicitly stale snapshot.
        let stale_world = Arc::new(QueueWorld::new(vec![snapshot(1, 0, true, serde_json::json!({"mode": "stance"}))]));
        let v2 = verifier(stale_world, 100, vec![]);
        let report2 = v2.verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1).await;
        assert_eq!(report2.decision, VerificationDecision::Unknown);
    }

    #[tokio::test]
    async fn unsafe_predicate_takes_priority() {
        let world = Arc::new(QueueWorld::new(vec![snapshot(
            1,
            0,
            false,
            serde_json::json!({"mode": "stance", "fall_detected": true}),
        )]));
        let v = verifier(
            world,
            100,
            vec![OutcomePredicate::Equals {
                path: "fall_detected".into(),
                value: serde_json::json!(true),
            }],
        );
        let report = v.verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1).await;
        assert_eq!(report.decision, VerificationDecision::Unsafe);
    }

    #[tokio::test]
    async fn matched_within_continuous_stable_window() {
        // Samples at t=0,50,100 all match stance -> window (100ms) satisfied at t=100.
        let world = Arc::new(QueueWorld::new(vec![
            snapshot(1, 0, false, serde_json::json!({"mode": "stance"})),
            snapshot(2, 50, false, serde_json::json!({"mode": "stance"})),
            snapshot(3, 100, false, serde_json::json!({"mode": "stance"})),
        ]));
        let v = verifier(world, 1_000, vec![]);
        let report = v.verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1).await;
        assert_eq!(report.decision, VerificationDecision::Matched);
        assert_eq!(report.evaluated_sequence, 3);
    }

    #[tokio::test]
    async fn mismatch_resets_the_stable_window() {
        // A "walk" sample in the middle resets the window; without enough
        // post-reset samples the window is never satisfied before timeout.
        let world = Arc::new(QueueWorld::new(vec![
            snapshot(1, 0, false, serde_json::json!({"mode": "stance"})),
            snapshot(2, 50, false, serde_json::json!({"mode": "walk"})),
            snapshot(3, 100, false, serde_json::json!({"mode": "stance"})),
        ]));
        let v = verifier(world, 1_000, vec![]);
        let report = v.verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1).await;
        // Window reset at t=50; only t=100 matches after reset -> 50ms < 100ms.
        assert_eq!(report.decision, VerificationDecision::RetryableMismatch);
    }
}
