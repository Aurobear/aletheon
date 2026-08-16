//! R5 bounded typed recovery acceptance for `RobotHarness`.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ::contracts::types::embodiment::{
    DeviceId, RiskClass, SkillDescriptor, SkillId, SkillOutcome, SkillRequest, SkillResult,
};
use ::contracts::types::episode_report::{AttemptRecord, EpisodeSettlement, SafeStopOutcome};
use ::contracts::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use ::contracts::types::outcome_verification::{VerificationDecision, VerificationReport};
use ::contracts::types::robot_failure::RobotFailureClass;
use ::contracts::types::skill_proposal::{PolicyProvenance, SkillProposal};
use ::contracts::types::world_state::{WorldSnapshot, WorldStatePort};
use ::contracts::{MonoDeadline, MonoTime, OperationId};
use async_trait::async_trait;
use cognit::harness::robot::state::{ReplanContext, RobotHarnessConfig, RobotState};
use cognit::harness::robot::PerceptionObservation;
use cognit::harness::robot::{
    EmbodiedExecutionPort, EpisodeSink, OutcomeVerifierPort, RobotExecutionError, RobotHarness,
};
use cognit::ports::policy_provider::{PolicyProviderError, PolicyProviderPort};

struct TestWorld {
    sequence: AtomicU64,
    fixed: bool,
}

impl TestWorld {
    fn new(fixed: bool) -> Self {
        Self {
            sequence: AtomicU64::new(0),
            fixed,
        }
    }
}

#[async_trait]
impl WorldStatePort for TestWorld {
    async fn latest(&self, device: &DeviceId, _schema: &str) -> Option<WorldSnapshot> {
        let sequence = if self.fixed {
            1
        } else {
            self.sequence.fetch_add(1, Ordering::SeqCst) + 1
        };
        Some(WorldSnapshot {
            device: device.clone(),
            schema: "robot.state".into(),
            schema_version: 1,
            sequence,
            payload: serde_json::json!({"mode": "idle"}),
            observed_at: MonoTime(sequence),
            valid_until: None,
            stale: false,
        })
    }

    async fn observe_until(
        &self,
        device: &DeviceId,
        schema: &str,
        _after_sequence: u64,
        _deadline: MonoDeadline,
    ) -> Option<WorldSnapshot> {
        self.latest(device, schema).await
    }
}

#[derive(Clone)]
enum ExecutionResponse {
    Outcome(SkillOutcome),
    Error(RobotExecutionError),
}

struct TestExecutor {
    responses: Mutex<VecDeque<ExecutionResponse>>,
    execute_calls: AtomicUsize,
    cancel_calls: AtomicUsize,
    safe_stop_calls: AtomicUsize,
    safe_stop_fails: bool,
}

impl TestExecutor {
    fn new(responses: Vec<ExecutionResponse>, safe_stop_fails: bool) -> Self {
        Self {
            responses: Mutex::new(responses.into()),
            execute_calls: AtomicUsize::new(0),
            cancel_calls: AtomicUsize::new(0),
            safe_stop_calls: AtomicUsize::new(0),
            safe_stop_fails,
        }
    }
}

#[async_trait]
impl EmbodiedExecutionPort for TestExecutor {
    async fn execute(&self, request: SkillRequest) -> Result<SkillResult, RobotExecutionError> {
        self.execute_calls.fetch_add(1, Ordering::SeqCst);
        match self
            .responses
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(ExecutionResponse::Outcome(SkillOutcome::Succeeded))
        {
            ExecutionResponse::Outcome(outcome) => Ok(SkillResult {
                operation_id: OperationId::new(),
                skill: request.skill,
                device: request.device,
                outcome,
                duration_ms: 1,
                evidence: vec![],
            }),
            ExecutionResponse::Error(error) => Err(error),
        }
    }

    async fn cancel(&self, _device: &DeviceId) -> Result<(), RobotExecutionError> {
        self.cancel_calls.fetch_add(1, Ordering::SeqCst);
        Ok(())
    }

    async fn safe_stop(&self, _device: &DeviceId) -> Result<(), RobotExecutionError> {
        self.safe_stop_calls.fetch_add(1, Ordering::SeqCst);
        if self.safe_stop_fails {
            Err(RobotExecutionError::Control(
                "fixture safe-stop failure".into(),
            ))
        } else {
            Ok(())
        }
    }
}

struct TestVerifier(Mutex<VecDeque<VerificationDecision>>);

#[async_trait]
impl OutcomeVerifierPort for TestVerifier {
    async fn verify(
        &self,
        _expected: &ExpectedOutcome,
        _device: &DeviceId,
        _before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        _attempt: u32,
    ) -> VerificationReport {
        let decision = self
            .0
            .lock()
            .unwrap()
            .pop_front()
            .unwrap_or(VerificationDecision::Matched);
        VerificationReport {
            decision,
            evaluated_sequence: after.map(|snapshot| snapshot.sequence).unwrap_or(0),
            observed_paths: vec!["mode".into()],
            reasons: vec!["fixture verification".into()],
            evidence: vec![],
        }
    }
}

struct TestPolicy {
    initial: SkillProposal,
    replanned: SkillProposal,
    replan_calls: AtomicUsize,
    contexts: Mutex<Vec<ReplanContext>>,
}

#[async_trait]
impl PolicyProviderPort for TestPolicy {
    async fn propose(
        &self,
        _goal: &str,
        _device: &DeviceId,
        _snapshots: &[WorldSnapshot],
        _visual: &[PerceptionObservation],
        _allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, PolicyProviderError> {
        Ok(vec![self.initial.clone()])
    }

    async fn replan(
        &self,
        context: &ReplanContext,
        _visual: &[PerceptionObservation],
    ) -> Result<Vec<SkillProposal>, PolicyProviderError> {
        self.replan_calls.fetch_add(1, Ordering::SeqCst);
        self.contexts.lock().unwrap().push(context.clone());
        Ok(vec![self.replanned.clone()])
    }

    async fn health(&self) -> Result<String, String> {
        Ok("ready".into())
    }
}

#[derive(Default)]
struct TestEpisodes {
    attempts: Mutex<Vec<AttemptRecord>>,
    settlements: Mutex<Vec<String>>,
}

#[async_trait]
impl EpisodeSink for TestEpisodes {
    async fn append_attempt(
        &self,
        _episode_id: &str,
        attempt: u32,
        attempt_id: &str,
        operation_id: Option<&OperationId>,
        request: &SkillRequest,
        expected: &ExpectedOutcome,
        _before: Option<&WorldSnapshot>,
        _after: Option<&WorldSnapshot>,
        result: Option<&SkillResult>,
        verification: Option<&VerificationReport>,
    ) -> Result<(), String> {
        let mut record = AttemptRecord::from_verification(
            attempt,
            attempt_id.to_owned(),
            operation_id.map(|operation| operation.0.to_string()),
            Some(request.clone()),
            expected.clone(),
            result.map(|result| format!("{:?}", result.outcome)),
            verification,
            None,
            None,
            None,
        );
        if let Some(result) = result {
            record.evidence_refs.extend(result.evidence.clone());
        }
        self.attempts.lock().unwrap().push(record);
        Ok(())
    }

    async fn close_episode(
        &self,
        _episode_id: &str,
        outcome: EpisodeSettlement,
    ) -> Result<(), String> {
        self.settlements
            .lock()
            .unwrap()
            .push(outcome.as_str().to_owned());
        Ok(())
    }

    async fn update_verification(
        &self,
        _episode_id: &str,
        attempt_id: &str,
        after: Option<&WorldSnapshot>,
        verification: &VerificationReport,
    ) -> Result<(), String> {
        if let Some(attempt) = self
            .attempts
            .lock()
            .unwrap()
            .iter_mut()
            .find(|attempt| attempt.attempt_id == attempt_id)
        {
            attempt.verification_decision = Some(verification.decision.clone());
            attempt.verification_observed_paths = verification.observed_paths.clone();
            attempt.verification_reasons = verification.reasons.clone();
            attempt.after_sequence = after.map(|snapshot| snapshot.sequence);
            attempt.verified_sequence = Some(verification.evaluated_sequence);
            attempt.evidence_refs.extend(verification.evidence.clone());
        }
        Ok(())
    }

    async fn load_attempts(&self, _episode_id: &str) -> Result<Vec<AttemptRecord>, String> {
        Ok(self.attempts.lock().unwrap().clone())
    }
}

fn descriptor() -> SkillDescriptor {
    SkillDescriptor {
        skill: SkillId("move".into()),
        device: DeviceId("bot".into()),
        summary: "move to a semantic target".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {"target": {"type": "string"}},
            "required": ["target"],
            "additionalProperties": false
        }),
        risk: RiskClass::Low,
        timeout_ms: 2_000,
        cancellable: true,
        preconditions: vec!["ready".into()],
        success_criteria: vec!["target reached".into()],
    }
}

fn proposal(target: &str) -> SkillProposal {
    SkillProposal {
        skill: SkillId("move".into()),
        device: DeviceId("bot".into()),
        parameters: serde_json::json!({"target": target}),
        goal_alignment: ::contracts::types::skill_proposal::GoalAlignment::Direct,
        expected_outcome: ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("done"),
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 1_000,
        },
        confidence: 0.9,
        frame_refs: vec![],
        provenance: PolicyProvenance {
            provider: "fixture-policy".into(),
            model: "fixture-model".into(),
            version: "1".into(),
            protocol_version: "1.0".into(),
            digest: "sha256:fixture".into(),
        },
    }
}

struct Fixture {
    harness: RobotHarness,
    executor: Arc<TestExecutor>,
    policy: Arc<TestPolicy>,
    episodes: Arc<TestEpisodes>,
}

fn fixture(
    config: RobotHarnessConfig,
    fixed_world: bool,
    decisions: Vec<VerificationDecision>,
    responses: Vec<ExecutionResponse>,
    replan_target: &str,
    safe_stop_fails: bool,
) -> Fixture {
    let executor = Arc::new(TestExecutor::new(responses, safe_stop_fails));
    let policy = Arc::new(TestPolicy {
        initial: proposal("A"),
        replanned: proposal(replan_target),
        replan_calls: AtomicUsize::new(0),
        contexts: Mutex::new(Vec::new()),
    });
    let episodes = Arc::new(TestEpisodes::default());
    let harness = RobotHarness::new(
        config,
        Arc::new(TestWorld::new(fixed_world)),
        executor.clone(),
        Arc::new(TestVerifier(Mutex::new(decisions.into()))),
        episodes.clone(),
        policy.clone(),
        Arc::new(cognit::harness::robot::NoopRobotPerception),
        vec![descriptor()],
    );
    Fixture {
        harness,
        executor,
        policy,
        episodes,
    }
}

async fn drive(harness: &RobotHarness) -> cognit::harness::robot::RobotHarnessState {
    let mut state = harness.init(
        DeviceId("bot".into()),
        "reach target".into(),
        "episode".into(),
    );
    for _ in 0..32 {
        if state.state.is_terminal() {
            return state;
        }
        state = harness.step(state).await;
    }
    panic!("robot harness did not terminate: {:?}", state.state);
}

#[tokio::test]
async fn retry_then_typed_replan_revalidates_and_preserves_all_attempts() {
    let config = RobotHarnessConfig {
        max_retries: 1,
        max_replans: 1,
        ..RobotHarnessConfig::default()
    };
    let fixture = fixture(
        config,
        false,
        vec![
            VerificationDecision::RetryableMismatch,
            VerificationDecision::ReplannableMismatch,
            VerificationDecision::Matched,
        ],
        vec![],
        "B",
        false,
    );
    let state = drive(&fixture.harness).await;

    assert_eq!(state.state, RobotState::Completed);
    assert_eq!(state.retries_used, 1);
    assert_eq!(state.replans_used, 1);
    assert_eq!(state.attempt, 3);
    assert_eq!(fixture.executor.execute_calls.load(Ordering::SeqCst), 3);
    assert_eq!(fixture.policy.replan_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.episodes.attempts.lock().unwrap().len(), 3);
    assert_eq!(
        fixture.episodes.settlements.lock().unwrap().as_slice(),
        ["completed"]
    );

    let contexts = fixture.policy.contexts.lock().unwrap();
    assert_eq!(contexts.len(), 1);
    assert_eq!(
        contexts[0].failure_class,
        RobotFailureClass::VerificationMismatch
    );
    assert_eq!(contexts[0].completed_attempts.len(), 2);
    assert_eq!(contexts[0].retries_remaining, 0);
    assert_eq!(contexts[0].replans_remaining, 0);
}

#[tokio::test]
async fn unchanged_repeated_replan_is_stopped_before_second_execution() {
    let config = RobotHarnessConfig {
        max_retries: 0,
        max_replans: 1,
        ..RobotHarnessConfig::default()
    };
    let fixture = fixture(
        config,
        true,
        vec![VerificationDecision::ReplannableMismatch],
        vec![],
        "A",
        false,
    );
    let state = drive(&fixture.harness).await;

    assert_eq!(state.state, RobotState::Failed);
    assert_eq!(fixture.executor.execute_calls.load(Ordering::SeqCst), 1);
    assert_eq!(fixture.policy.replan_calls.load(Ordering::SeqCst), 1);
    assert!(state
        .failures
        .iter()
        .any(|failure| failure.class == RobotFailureClass::RepeatedFailure));
    assert_eq!(fixture.executor.safe_stop_calls.load(Ordering::SeqCst), 1);
}

#[tokio::test]
async fn exhausted_budgets_safe_stop_and_settle_failed() {
    let config = RobotHarnessConfig {
        max_retries: 0,
        max_replans: 0,
        ..RobotHarnessConfig::default()
    };
    let fixture = fixture(
        config,
        false,
        vec![VerificationDecision::RetryableMismatch],
        vec![],
        "B",
        false,
    );
    let state = drive(&fixture.harness).await;

    assert_eq!(state.state, RobotState::Failed);
    assert_eq!(state.retries_used, 0);
    assert_eq!(state.replans_used, 0);
    let safe_stop = state.safe_stop.as_ref().expect("safe-stop receipt");
    assert_eq!(safe_stop.attempted_after_attempt, 1);
    assert_eq!(safe_stop.outcome, SafeStopOutcome::Succeeded);
    assert_eq!(
        safe_stop.trigger,
        Some(RobotFailureClass::ReplanBudgetExhausted)
    );
    assert_eq!(fixture.policy.replan_calls.load(Ordering::SeqCst), 0);
    for class in [
        RobotFailureClass::VerificationTimeout,
        RobotFailureClass::RetryBudgetExhausted,
        RobotFailureClass::ReplanBudgetExhausted,
    ] {
        assert!(state.failures.iter().any(|failure| failure.class == class));
    }
    assert_eq!(
        fixture.episodes.settlements.lock().unwrap().as_slice(),
        ["failed"]
    );
}

#[tokio::test]
async fn provider_disconnect_and_unsafe_both_bypass_replan() {
    let disconnect = fixture(
        RobotHarnessConfig::default(),
        false,
        vec![],
        vec![ExecutionResponse::Error(
            RobotExecutionError::ProviderDisconnected("bridge lost".into()),
        )],
        "B",
        false,
    );
    let disconnected = drive(&disconnect.harness).await;
    assert_eq!(disconnected.state, RobotState::Failed);
    assert_eq!(disconnect.policy.replan_calls.load(Ordering::SeqCst), 0);
    assert_eq!(
        disconnected.failures[0].class,
        RobotFailureClass::ProviderDisconnected
    );

    let unsafe_fixture = fixture(
        RobotHarnessConfig::default(),
        false,
        vec![VerificationDecision::Unsafe],
        vec![],
        "B",
        false,
    );
    let unsafe_state = drive(&unsafe_fixture.harness).await;
    assert_eq!(unsafe_state.state, RobotState::Failed);
    assert_eq!(unsafe_fixture.policy.replan_calls.load(Ordering::SeqCst), 0);
    assert_eq!(unsafe_state.failures[0].class, RobotFailureClass::Unsafe);
}

#[tokio::test]
async fn safe_stop_failure_is_secondary_and_cancellation_is_recovered() {
    let unsafe_fixture = fixture(
        RobotHarnessConfig::default(),
        false,
        vec![VerificationDecision::Unsafe],
        vec![],
        "B",
        true,
    );
    let state = drive(&unsafe_fixture.harness).await;
    assert_eq!(state.failures[0].class, RobotFailureClass::Unsafe);
    assert!(state
        .failures
        .iter()
        .skip(1)
        .any(|failure| failure.class == RobotFailureClass::SafeStopFailure));
    assert_eq!(
        state.safe_stop.as_ref().unwrap().outcome,
        SafeStopOutcome::Failed
    );
    assert_eq!(
        state.safe_stop.as_ref().unwrap().trigger,
        Some(RobotFailureClass::Unsafe)
    );

    let cancelled_fixture = fixture(
        RobotHarnessConfig::default(),
        false,
        vec![],
        vec![],
        "B",
        false,
    );
    let state = cancelled_fixture.harness.init(
        DeviceId("bot".into()),
        "cancel me".into(),
        "cancelled".into(),
    );
    let state = cancelled_fixture.harness.cancel_and_safe_stop(state).await;
    assert_eq!(state.state, RobotState::Failed);
    assert_eq!(state.failures[0].class, RobotFailureClass::Cancelled);
    assert_eq!(
        state.safe_stop.as_ref().unwrap().outcome,
        SafeStopOutcome::Succeeded
    );
    assert_eq!(
        state.safe_stop.as_ref().unwrap().trigger,
        Some(RobotFailureClass::Cancelled)
    );
    assert_eq!(
        cancelled_fixture
            .executor
            .cancel_calls
            .load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        cancelled_fixture
            .executor
            .safe_stop_calls
            .load(Ordering::SeqCst),
        1
    );
    assert_eq!(
        cancelled_fixture
            .episodes
            .settlements
            .lock()
            .unwrap()
            .as_slice(),
        ["cancelled"]
    );
}
