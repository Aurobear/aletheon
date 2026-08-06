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

use std::collections::BTreeSet;
use std::sync::Arc;

use async_trait::async_trait;
use cognit::harness::robot::OutcomeVerifierPort;
use fabric::types::embodiment::DeviceId;
use fabric::types::expected_outcome::{
    evaluate_expected, evaluate_predicate, ExpectedOutcome, OutcomeMatch, OutcomePredicate,
};
use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};
use fabric::types::world_state::{WorldSnapshot, WorldStatePort, ANY_SCHEMA};
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
        observed_paths: Vec<String>,
        reasons: Vec<String>,
        after: Option<&WorldSnapshot>,
    ) -> VerificationReport {
        VerificationReport {
            decision,
            evaluated_sequence: after.map(|snap| snap.sequence).unwrap_or(0),
            observed_paths,
            reasons,
            evidence: vec![],
        }
    }

    /// Resolve which observation schema the expected outcome targets.
    ///
    /// A device can expose several schemas (e.g. `base_pose`, `base_twist`,
    /// `ground_truth_pose`) with independent sequence counters, so the verifier
    /// must observe the schema the outcome's predicate path names — the first
    /// path segment of every leaf predicate. When all leaves agree on one
    /// segment and the world actually holds that schema, it is selected;
    /// otherwise the world is single-schema and `ANY_SCHEMA` preserves the
    /// unqualified-path behavior.
    async fn resolve_schema(&self, device: &DeviceId, expected: &ExpectedOutcome) -> String {
        let mut segments = BTreeSet::new();
        collect_first_segments(&expected.predicate, &mut segments);
        let candidate = if segments.len() == 1 {
            segments.iter().next().cloned().filter(|s| !s.is_empty())
        } else {
            None
        };
        match candidate {
            Some(name) => match self.world.latest(device, &name).await {
                Some(snap) if snap.schema == name => name,
                _ => ANY_SCHEMA.to_string(),
            },
            None => ANY_SCHEMA.to_string(),
        }
    }
}

/// Collect the first dot-path segment of every leaf predicate. Empty segments
/// are ignored; `All`/`Any` predicates are walked recursively.
fn collect_first_segments(predicate: &OutcomePredicate, out: &mut BTreeSet<String>) {
    match predicate {
        OutcomePredicate::Equals { path, .. }
        | OutcomePredicate::NotEquals { path, .. }
        | OutcomePredicate::Range { path, .. }
        | OutcomePredicate::Change { path, .. } => {
            if let Some(segment) = path.split('.').next() {
                if !segment.is_empty() {
                    out.insert(segment.to_string());
                }
            }
        }
        OutcomePredicate::All { predicates } | OutcomePredicate::Any { predicates } => {
            for child in predicates {
                collect_first_segments(child, out);
            }
        }
    }
}

/// Collect every dot-path actually evaluated for a predicate tree. A sorted set
/// keeps the report deterministic when a composite predicate repeats a path.
fn predicate_paths(predicate: &OutcomePredicate) -> Vec<String> {
    fn collect(predicate: &OutcomePredicate, out: &mut BTreeSet<String>) {
        match predicate {
            OutcomePredicate::Equals { path, .. }
            | OutcomePredicate::NotEquals { path, .. }
            | OutcomePredicate::Range { path, .. }
            | OutcomePredicate::Change { path, .. } => {
                out.insert(path.clone());
            }
            OutcomePredicate::All { predicates } | OutcomePredicate::Any { predicates } => {
                for child in predicates {
                    collect(child, out);
                }
            }
        }
    }

    let mut paths = BTreeSet::new();
    collect(predicate, &mut paths);
    paths.into_iter().collect()
}

/// Nest a snapshot's payload under its schema so schema-qualified predicate
/// paths (e.g. `base_twist.linear_velocity.x`) resolve against it. `ANY_SCHEMA`
/// worlds use unqualified paths and are passed through untouched.
fn wrap_snapshot(snapshot: &WorldSnapshot, schema: &str) -> WorldSnapshot {
    if schema == ANY_SCHEMA {
        return snapshot.clone();
    }
    let mut wrapped = snapshot.clone();
    wrapped.payload = serde_json::json!({ schema: snapshot.payload });
    wrapped
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

        // Resolve the observation schema this outcome targets. The harness's
        // `after`/`before` snapshots are the device-freshest across schemas, so
        // for a schema-qualified outcome the sequence seed must come from that
        // schema's own latest — seeding from the freshest would wait forever on
        // a slower schema whose counter trails a faster one.
        let schema = self.resolve_schema(device, expected).await;
        let after_sequence = if schema == ANY_SCHEMA {
            after
                .as_ref()
                .map(|snap| snap.sequence)
                .or_else(|| before.as_ref().map(|snap| snap.sequence))
                .unwrap_or(0)
        } else {
            self.world
                .latest(device, &schema)
                .await
                .map(|snap| snap.sequence)
                .unwrap_or(0)
        };
        let before_wrapped = before.map(|snap| wrap_snapshot(snap, &schema));
        let expected_paths = predicate_paths(&expected.predicate);

        // Wait for post-execution observations and evaluate a CONTINUOUS stable
        // window: `stable_window_ms` of elapsed observation time with the
        // predicate holding on every sample. Any mismatch resets the window.
        let mut window_started_at: Option<u64> = None;
        let mut last_sequence = after_sequence;
        let mut evaluated_expected = false;
        loop {
            let now = self.clock.mono_now();
            if deadline.is_expired_at(now) {
                break;
            }
            let Some(snapshot) = self
                .world
                .observe_until(device, &schema, last_sequence, deadline)
                .await
            else {
                break; // no new observation before the deadline
            };
            last_sequence = snapshot.sequence;

            if snapshot.stale
                || now.0.saturating_sub(snapshot.observed_at.0) > expected.freshness_ms
            {
                return self.report(
                    VerificationDecision::Unknown,
                    vec![],
                    vec!["after observation stale or older than freshness window".into()],
                    Some(&snapshot),
                );
            }

            // Unsafe predicates take priority over every success predicate.
            for predicate in &self.unsafe_predicates {
                if evaluate_predicate(predicate, &snapshot.payload, None) {
                    return self.report(
                        VerificationDecision::Unsafe,
                        predicate_paths(predicate),
                        vec!["unsafe predicate matched after execution".into()],
                        Some(&snapshot),
                    );
                }
            }

            let snapshot_wrapped = wrap_snapshot(&snapshot, &schema);
            evaluated_expected = true;
            match evaluate_expected(expected, &snapshot_wrapped, before_wrapped.as_ref(), now) {
                OutcomeMatch::Match => {
                    let window_start = *window_started_at.get_or_insert(snapshot.observed_at.0);
                    if snapshot.observed_at.0.saturating_sub(window_start)
                        >= expected.stable_window_ms
                    {
                        return self.report(
                            VerificationDecision::Matched,
                            expected_paths,
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
                        expected_paths,
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
            if evaluated_expected {
                expected_paths
            } else {
                vec![]
            },
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

    fn snapshot(
        seq: u64,
        observed_at: u64,
        stale: bool,
        payload: serde_json::Value,
    ) -> WorldSnapshot {
        schema_snapshot("robot.state/v1", seq, observed_at, stale, payload)
    }

    fn schema_snapshot(
        schema: &str,
        seq: u64,
        observed_at: u64,
        stale: bool,
        payload: serde_json::Value,
    ) -> WorldSnapshot {
        WorldSnapshot {
            device: DeviceId("bot".into()),
            schema: schema.into(),
            schema_version: 1,
            sequence: seq,
            payload,
            observed_at: MonoTime(observed_at),
            valid_until: None,
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
        async fn latest(&self, _device: &DeviceId, _schema: &str) -> Option<WorldSnapshot> {
            self.queue.lock().unwrap().front().cloned()
        }
        async fn observe_until(
            &self,
            _device: &DeviceId,
            _schema: &str,
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
        DeterministicOutcomeVerifier::new(
            world,
            Arc::new(TestClock::new(0, now_ms)),
            unsafe_predicates,
        )
    }

    #[tokio::test]
    async fn missing_observation_before_deadline_is_retryable() {
        let v = verifier(Arc::new(QueueWorld::new(vec![])), 100, vec![]);
        let report = v
            .verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1)
            .await;
        assert_eq!(report.decision, VerificationDecision::RetryableMismatch);
    }

    #[tokio::test]
    async fn stale_or_freshness_expired_observation_is_unknown() {
        let world = Arc::new(QueueWorld::new(vec![snapshot(
            1,
            0,
            false,
            serde_json::json!({"mode": "stance"}),
        )]));
        // now = 60_000 >> freshness 5_000 -> too old.
        let v = verifier(world.clone(), 60_000, vec![]);
        let report = v
            .verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1)
            .await;
        assert_eq!(report.decision, VerificationDecision::Unknown);
        // Explicitly stale snapshot.
        let stale_world = Arc::new(QueueWorld::new(vec![snapshot(
            1,
            0,
            true,
            serde_json::json!({"mode": "stance"}),
        )]));
        let v2 = verifier(stale_world, 100, vec![]);
        let report2 = v2
            .verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1)
            .await;
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
        let report = v
            .verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1)
            .await;
        assert_eq!(report.decision, VerificationDecision::Unsafe);
        assert_eq!(report.observed_paths, ["fall_detected"]);
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
        let report = v
            .verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1)
            .await;
        assert_eq!(report.decision, VerificationDecision::Matched);
        assert_eq!(report.evaluated_sequence, 3);
        assert_eq!(report.observed_paths, ["mode"]);
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
        let report = v
            .verify(&stance_expected(), &DeviceId("bot".into()), None, None, 1)
            .await;
        // Window reset at t=50; only t=100 matches after reset -> 50ms < 100ms.
        assert_eq!(report.decision, VerificationDecision::RetryableMismatch);
    }

    #[tokio::test]
    async fn schema_qualified_outcome_matches_on_target_schema() {
        // A multi-schema world (the predicate path names the `base_twist`
        // schema) must be verified against base_twist's own sequence counter,
        // seeded from that schema's latest rather than a device-freshest
        // snapshot from another schema.
        let world = Arc::new(QueueWorld::new(vec![
            schema_snapshot(
                "base_twist",
                1,
                0,
                false,
                serde_json::json!({"linear_velocity": {"x": 0.0, "y": 0.0}}),
            ),
            schema_snapshot(
                "base_twist",
                2,
                50,
                false,
                serde_json::json!({"linear_velocity": {"x": 0.0, "y": 0.0}}),
            ),
            schema_snapshot(
                "base_twist",
                3,
                100,
                false,
                serde_json::json!({"linear_velocity": {"x": 0.0, "y": 0.0}}),
            ),
            schema_snapshot(
                "base_twist",
                4,
                150,
                false,
                serde_json::json!({"linear_velocity": {"x": 0.0, "y": 0.0}}),
            ),
        ]));
        let expected = ExpectedOutcome {
            predicate: OutcomePredicate::Range {
                path: "base_twist.linear_velocity.x".into(),
                min: Some(-0.01),
                max: Some(0.01),
            },
            freshness_ms: 5_000,
            stable_window_ms: 100,
            timeout_ms: 10_000,
        };
        let v = verifier(world, 1_000, vec![]);
        let report = v
            .verify(&expected, &DeviceId("bot".into()), None, None, 1)
            .await;
        assert_eq!(report.decision, VerificationDecision::Matched);
    }
}
