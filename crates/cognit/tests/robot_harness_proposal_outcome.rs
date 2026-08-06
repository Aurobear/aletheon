//! PR1 `robot-harness-carries-proposal-outcome` closure test.
//!
//! Drives the RobotHarness state machine end-to-end with a policy proposal that
//! carries a NON-stance expected outcome, and asserts that Execute/Verify use the
//! proposal's outcome — not a hardcoded `mode == stance`.
//! PR2 companion assertions: every recorded attempt carries a real UUID
//! `OperationId`, never the legacy `"op"` placeholder.

use cognit::harness::robot::state::RobotHarnessConfig;
use cognit::harness::robot::{
    EmbodiedExecutionPort, EpisodeSink, OutcomeVerifierPort, RobotExecutionError, RobotHarness,
};
use cognit::ports::policy_provider::PolicyProviderPort;
use fabric::types::embodiment::{
    DeviceId, RiskClass, SkillDescriptor, SkillId, SkillOutcome, SkillRequest, SkillResult,
};
use fabric::types::episode_report::EpisodeSettlement;
use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};
use fabric::types::perception_observation::PerceptionObservation;
use fabric::types::robot_failure::RobotFailureClass;
use fabric::types::skill_proposal::{GoalAlignment, PolicyProvenance, SkillProposal};
use fabric::types::world_state::{WorldSnapshot, WorldStatePort};
use fabric::{MonoDeadline, OperationId};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

struct FakeWorldState;
impl FakeWorldState {
    fn snapshot() -> WorldSnapshot {
        WorldSnapshot {
            device: DeviceId("bot".into()),
            schema: "robot.state/v1".into(),
            schema_version: 1,
            sequence: 1,
            payload: serde_json::json!({"mode": "stance2"}),
            observed_at: fabric::MonoTime(1),
            valid_until: None,
            stale: false,
        }
    }
}
#[async_trait::async_trait]
impl WorldStatePort for FakeWorldState {
    async fn latest(&self, _device: &DeviceId, _schema: &str) -> Option<WorldSnapshot> {
        Some(Self::snapshot())
    }
    async fn observe_until(
        &self,
        _device: &DeviceId,
        _schema: &str,
        _after_sequence: u64,
        _deadline: MonoDeadline,
    ) -> Option<WorldSnapshot> {
        Some(Self::snapshot())
    }
}

#[derive(Default)]
struct FakeExecutor {
    executions: AtomicUsize,
    safe_stops: AtomicUsize,
}

#[derive(Default)]
struct MismatchedExecutor {
    safe_stops: AtomicUsize,
}

#[async_trait::async_trait]
impl EmbodiedExecutionPort for MismatchedExecutor {
    async fn execute(&self, request: SkillRequest) -> Result<SkillResult, RobotExecutionError> {
        Ok(SkillResult {
            operation_id: OperationId::new(),
            skill: SkillId("kuavo.stop".into()),
            device: request.device,
            outcome: SkillOutcome::Succeeded,
            duration_ms: 10,
            evidence: vec![],
        })
    }

    async fn cancel(&self, _device: &DeviceId) -> Result<(), RobotExecutionError> {
        Ok(())
    }

    async fn safe_stop(&self, _device: &DeviceId) -> Result<(), RobotExecutionError> {
        self.safe_stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}
#[async_trait::async_trait]
impl EmbodiedExecutionPort for FakeExecutor {
    async fn execute(&self, request: SkillRequest) -> Result<SkillResult, RobotExecutionError> {
        self.executions.fetch_add(1, Ordering::SeqCst);
        Ok(SkillResult {
            operation_id: OperationId::new(),
            skill: request.skill,
            device: request.device,
            outcome: SkillOutcome::Succeeded,
            duration_ms: 10,
            evidence: vec![],
        })
    }
    async fn cancel(&self, _device: &DeviceId) -> Result<(), RobotExecutionError> {
        Ok(())
    }
    async fn safe_stop(&self, _device: &DeviceId) -> Result<(), RobotExecutionError> {
        self.safe_stops.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }
}

#[derive(Default)]
struct RecordingVerifier {
    received_expected: Mutex<Option<ExpectedOutcome>>,
}
#[async_trait::async_trait]
impl OutcomeVerifierPort for RecordingVerifier {
    async fn verify(
        &self,
        expected: &ExpectedOutcome,
        _device: &DeviceId,
        _before: Option<&WorldSnapshot>,
        _after: Option<&WorldSnapshot>,
        _attempt: u32,
    ) -> VerificationReport {
        *self.received_expected.lock().unwrap() = Some(expected.clone());
        VerificationReport {
            decision: VerificationDecision::Matched,
            evaluated_sequence: 0,
            observed_paths: vec![],
            reasons: vec![],
            evidence: vec![],
        }
    }
}

#[derive(Default)]
struct RecordingEpisodes {
    attempt_ids: Mutex<Vec<String>>,
    operation_ids: Mutex<Vec<Option<String>>>,
}
#[async_trait::async_trait]
impl EpisodeSink for RecordingEpisodes {
    async fn append_attempt(
        &self,
        _episode_id: &str,
        _attempt: u32,
        attempt_id: &str,
        operation_id: Option<&OperationId>,
        _request: &SkillRequest,
        _expected: &ExpectedOutcome,
        _before: Option<&WorldSnapshot>,
        _after: Option<&WorldSnapshot>,
        _result: Option<&SkillResult>,
        _verification: Option<&VerificationReport>,
    ) -> Result<(), String> {
        self.attempt_ids
            .lock()
            .unwrap()
            .push(attempt_id.to_string());
        self.operation_ids
            .lock()
            .unwrap()
            .push(operation_id.map(|id| id.0.to_string()));
        Ok(())
    }
    async fn close_episode(
        &self,
        _episode_id: &str,
        _outcome: EpisodeSettlement,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn update_verification(
        &self,
        _episode_id: &str,
        _attempt_id: &str,
        _after: Option<&WorldSnapshot>,
        _verification: &VerificationReport,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn load_attempts(
        &self,
        _episode_id: &str,
    ) -> Result<Vec<fabric::types::episode_report::AttemptRecord>, String> {
        Ok(vec![])
    }
}

/// Policy returns a proposal whose expected outcome is `mode == "stance2"`.
struct Stance2Policy {
    goal_alignment: GoalAlignment,
}
#[async_trait::async_trait]
impl PolicyProviderPort for Stance2Policy {
    async fn propose(
        &self,
        _goal: &str,
        device: &DeviceId,
        _snapshots: &[WorldSnapshot],
        _visual: &[PerceptionObservation],
        _allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, cognit::ports::policy_provider::PolicyProviderError> {
        Ok(vec![SkillProposal {
            skill: SkillId("kuavo.stance".into()),
            device: device.clone(),
            parameters: serde_json::json!({}),
            goal_alignment: self.goal_alignment,
            expected_outcome: ExpectedOutcome {
                predicate: OutcomePredicate::Equals {
                    path: "mode".into(),
                    value: serde_json::json!("stance2"),
                },
                freshness_ms: 500,
                stable_window_ms: 0,
                timeout_ms: 5000,
            },
            confidence: 0.9,
            frame_refs: vec![],
            provenance: PolicyProvenance {
                provider: "test".into(),
                model: "m".into(),
                version: "1".into(),
                protocol_version: "1.0".into(),
                digest: "sha256:test".into(),
            },
        }])
    }
    async fn health(&self) -> Result<String, String> {
        Ok("ready".into())
    }
}

fn allowed_skills() -> Vec<SkillDescriptor> {
    vec![SkillDescriptor {
        skill: SkillId("kuavo.stance".into()),
        device: DeviceId("bot".into()),
        summary: "stance".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }),
        risk: RiskClass::Low,
        timeout_ms: 10000,
        cancellable: false,
        preconditions: vec!["ready".into()],
        success_criteria: vec!["stance2".into()],
    }]
}

#[tokio::test]
async fn verify_uses_proposal_expected_outcome_not_hardcoded_stance() {
    let verifier = Arc::new(RecordingVerifier::default());
    let episodes = Arc::new(RecordingEpisodes::default());

    let harness = RobotHarness::new(
        RobotHarnessConfig::default(),
        Arc::new(FakeWorldState),
        Arc::new(FakeExecutor::default()),
        verifier.clone(),
        episodes.clone(),
        Arc::new(Stance2Policy {
            goal_alignment: GoalAlignment::Direct,
        }),
        Arc::new(cognit::harness::robot::NoopRobotPerception),
        allowed_skills(),
    );

    let mut state = harness.init(DeviceId("bot".into()), "stand".into(), "ep-1".into());
    for _ in 0..12 {
        if state.state.is_terminal() {
            break;
        }
        state = harness.step(state).await;
    }

    assert!(
        state.state.is_terminal(),
        "harness did not reach a terminal state: {:?}",
        state.state
    );

    // PR1: the verifier must have received the proposal's outcome, not mode==stance.
    let received = verifier
        .received_expected
        .lock()
        .unwrap()
        .clone()
        .unwrap_or_else(|| {
            panic!(
                "verifier.verify was never called; terminal={:?} failures={:?}",
                state.state, state.failures
            )
        });
    assert_eq!(
        received.predicate,
        OutcomePredicate::Equals {
            path: "mode".into(),
            value: serde_json::json!("stance2"),
        },
        "verify must use the proposal's expected outcome, not a hardcoded stance"
    );

    // PR2: every successful attempt carries a real typed operation id, never
    // "op"; the attempt is also recorded under an independent attempt id.
    let attempt_ids = episodes.attempt_ids.lock().unwrap();
    assert!(
        !attempt_ids.is_empty(),
        "at least one attempt should be recorded"
    );
    let ops = episodes.operation_ids.lock().unwrap();
    assert!(
        ops.iter().all(|op| op.is_some()),
        "successful attempts must carry a typed operation id"
    );
    for op in ops.iter().flatten() {
        assert_ne!(op, "op", "legacy \"op\" placeholder must not be recorded");
        assert!(
            op.parse::<OperationId>().is_ok(),
            "recorded operation id must be a valid OperationId: {op}"
        );
    }
}

#[tokio::test]
async fn safety_fallback_never_executes_or_settles_the_goal_completed() {
    let executor = Arc::new(FakeExecutor::default());
    let episodes = Arc::new(RecordingEpisodes::default());
    let harness = RobotHarness::new(
        RobotHarnessConfig::default(),
        Arc::new(FakeWorldState),
        executor.clone(),
        Arc::new(RecordingVerifier::default()),
        episodes.clone(),
        Arc::new(Stance2Policy {
            goal_alignment: GoalAlignment::SafetyFallback,
        }),
        Arc::new(cognit::harness::robot::NoopRobotPerception),
        allowed_skills(),
    );

    let mut state = harness.init(
        DeviceId("bot".into()),
        "request outside the available skill contract".into(),
        "ep-fallback".into(),
    );
    for _ in 0..12 {
        if state.state.is_terminal() {
            break;
        }
        state = harness.step(state).await;
    }

    assert!(state.state.is_terminal());
    assert_eq!(state.settlement, Some(EpisodeSettlement::Failed));
    assert_eq!(executor.executions.load(Ordering::SeqCst), 0);
    assert_eq!(executor.safe_stops.load(Ordering::SeqCst), 1);
    assert!(episodes.attempt_ids.lock().unwrap().is_empty());
    assert_eq!(
        state
            .latest_policy_provenance
            .as_ref()
            .map(|provenance| provenance.provider.as_str()),
        Some("test")
    );
    assert_eq!(
        state
            .latest_policy_provenance
            .as_ref()
            .map(|p| p.model.as_str()),
        Some("m")
    );
    assert!(state.failures.iter().any(|failure| {
        failure.class == RobotFailureClass::ProposalRejected
            && failure.detail.contains("goal_alignment")
    }));
}

#[tokio::test]
async fn mismatched_execution_result_identity_fails_closed() {
    let executor = Arc::new(MismatchedExecutor::default());
    let episodes = Arc::new(RecordingEpisodes::default());
    let harness = RobotHarness::new(
        RobotHarnessConfig::default(),
        Arc::new(FakeWorldState),
        executor.clone(),
        Arc::new(RecordingVerifier::default()),
        episodes.clone(),
        Arc::new(Stance2Policy {
            goal_alignment: GoalAlignment::Direct,
        }),
        Arc::new(cognit::harness::robot::NoopRobotPerception),
        allowed_skills(),
    );

    let mut state = harness.init(
        DeviceId("bot".into()),
        "stand".into(),
        "ep-result-mismatch".into(),
    );
    for _ in 0..12 {
        if state.state.is_terminal() {
            break;
        }
        state = harness.step(state).await;
    }

    assert_eq!(state.settlement, Some(EpisodeSettlement::Failed));
    assert_eq!(executor.safe_stops.load(Ordering::SeqCst), 1);
    assert_eq!(episodes.attempt_ids.lock().unwrap().len(), 1);
    assert!(state.failures.iter().any(|failure| {
        failure.class == RobotFailureClass::ProviderDisconnected
            && failure.detail.contains("result identity mismatch")
    }));
}
