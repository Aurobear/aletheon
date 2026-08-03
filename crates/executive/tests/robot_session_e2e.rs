//! Robot daemon-integration closure test (gap 3).
//!
//! Assembles the production RobotHarness composition and drives it through the
//! `CognitiveSession` turn contract. Uses a synchronous world (no background
//! pump) so the after-execution observation is deterministically visible to
//! Verify — the pump itself is covered by `world_state` unit tests.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;

use async_trait::async_trait;
use cognit::harness::robot::session::RobotCognitiveSession;
use cognit::harness::robot::state::RobotHarnessConfig;
use cognit::harness::robot::{EmbodiedExecutionPort, EpisodeSink, OutcomeVerifierPort};
use cognit::harness::session::CognitiveSession;
use executive::application::deterministic_outcome_verifier::DeterministicOutcomeVerifier;
use executive::application::embodied_execution_adapter::EmbodiedExecutionAdapter;
use executive::application::robot_harness_composition::{
    build_robot_harness, DefaultPlanPort, RobotHarnessDependencies, StubRobotPolicy,
};
use fabric::types::embodiment::{
    DeviceId, EmbodiedObservation, EmbodimentExecutionPort, RiskClass, SkillDescriptor,
    SkillDispatchError, SkillId, SkillOutcome, SkillRequest, SkillResult,
};
use fabric::types::expected_outcome::ExpectedOutcome;
use fabric::types::outcome_verification::VerificationReport;
use fabric::types::world_state::{WorldSnapshot, WorldStatePort};
use fabric::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, MonoDeadline, MonoTime, NoopTurnEventSink,
    OperationId, PermissionProfileId, PrincipalContext, PrincipalId, ProcessId, StubTurnServices,
    ThreadId, TurnRequest, TurnStop, WorkspacePolicy,
};
use kernel::chronos::TestClock;
use tokio_util::sync::CancellationToken;

/// Deterministic stance device: idle until `kuavo.stance` executes, then stance.
struct StanceSim {
    mode: tokio::sync::Mutex<String>,
    sequence: AtomicU64,
}

impl StanceSim {
    fn new() -> Self {
        Self {
            mode: tokio::sync::Mutex::new("idle".into()),
            sequence: AtomicU64::new(0),
        }
    }
}

#[async_trait]
impl EmbodimentExecutionPort for StanceSim {
    async fn observe(
        &self,
        device: &DeviceId,
    ) -> Result<Vec<EmbodiedObservation>, SkillDispatchError> {
        Ok(vec![self.get_state(device).await?.ok_or_else(|| {
            SkillDispatchError::Rejected("no state".into())
        })?])
    }
    async fn get_state(
        &self,
        _device: &DeviceId,
    ) -> Result<Option<EmbodiedObservation>, SkillDispatchError> {
        let mode = self.mode.lock().await.clone();
        let sequence = self.sequence.fetch_add(1, Ordering::SeqCst) + 1;
        let now = MonoTime(sequence);
        Ok(Some(EmbodiedObservation {
            schema: "robot.state/v1".into(),
            schema_version: 1,
            source: "test".into(),
            sequence,
            source_time: now,
            received_at: now,
            valid_until: Some(MonoDeadline::after(now, 2_000)),
            confidence: 1.0,
            frame_ref: None,
            payload: serde_json::json!({
                "mode": mode,
                "base": {"height_m": 0.8},
                "joint_error": {"max_rad": 0.02},
                "fall_detected": false,
            }),
            evidence: vec![],
        }))
    }
    async fn list_skills(
        &self,
        _device: &DeviceId,
    ) -> Result<Vec<SkillDescriptor>, SkillDispatchError> {
        Ok(vec![SkillDescriptor {
            skill: SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            summary: "stance".into(),
            input_schema: serde_json::json!({"type": "object", "required": []}),
            risk: RiskClass::Low,
            timeout_ms: 10_000,
            cancellable: false,
            preconditions: vec![],
            success_criteria: vec![],
        }])
    }
    async fn execute_skill(
        &self,
        request: SkillRequest,
    ) -> Result<SkillResult, SkillDispatchError> {
        *self.mode.lock().await = "stance".into();
        Ok(SkillResult {
            operation_id: OperationId::new(),
            skill: request.skill,
            device: request.device,
            outcome: SkillOutcome::Succeeded,
            duration_ms: 0,
            evidence: vec![],
        })
    }
    async fn cancel(&self, _operation: &OperationId) -> Result<(), SkillDispatchError> {
        Ok(())
    }
    async fn safe_stop(&self, _device: &DeviceId) -> Result<(), SkillDispatchError> {
        Ok(())
    }
}

/// Synchronous world: serves the executor's current state directly (no pump
/// latency) so Verify deterministically observes the post-execution stance.
struct DirectWorld {
    executor: Arc<dyn EmbodimentExecutionPort>,
}

#[async_trait]
impl WorldStatePort for DirectWorld {
    async fn latest(&self, device: &DeviceId) -> Option<WorldSnapshot> {
        let observation = self.executor.get_state(device).await.ok()??;
        Some(WorldSnapshot {
            device: device.clone(),
            schema: observation.schema,
            sequence: observation.sequence,
            payload: observation.payload,
            observed_at: observation.source_time,
            stale: false,
        })
    }
    async fn observe_until(
        &self,
        device: &DeviceId,
        _after_sequence: u64,
        _deadline: MonoDeadline,
    ) -> Option<WorldSnapshot> {
        self.latest(device).await
    }
}

fn turn_request() -> TurnRequest {
    let context = PrincipalContext::new(
        PrincipalId("test".into()),
        LocalOsPrincipal { uid: 0, gid: 0 },
        ConnectionId::default(),
        ThreadId("test".into()),
        WorkspacePolicy::from_resolved_roots("/tmp/robot-ws".into(), Vec::new()).unwrap(),
        PermissionProfileId("exec".into()),
        ApprovalPolicy::Never,
    );
    TurnRequest {
        operation_id: OperationId::new(),
        process_id: ProcessId::new(),
        context,
        input: "stand and stay stable".into(),
        model_policy: None,
        deadline: None,
        requirements: vec![],
        requested_task_kind: None,
        evaluation_contract: None,
    }
}

#[tokio::test]
async fn robot_turn_drives_harness_to_completion() {
    let clock = Arc::new(TestClock::default());
    let executor: Arc<dyn EmbodimentExecutionPort> = Arc::new(StanceSim::new());
    let device = DeviceId("bot".into());
    let world: Arc<dyn WorldStatePort> = Arc::new(DirectWorld {
        executor: executor.clone(),
    });
    let adapter: Arc<dyn EmbodiedExecutionPort> = Arc::new(EmbodiedExecutionAdapter::new(executor));
    let verifier: Arc<dyn OutcomeVerifierPort> =
        Arc::new(DeterministicOutcomeVerifier::new(clock.clone(), vec![]));
    let episodes: Arc<dyn EpisodeSink> = Arc::new(RecordingEpisodes);
    let allowed_skills = vec![SkillDescriptor {
        skill: SkillId("kuavo.stance".into()),
        device: device.clone(),
        summary: "stance".into(),
        input_schema: serde_json::json!({"type": "object", "required": []}),
        risk: RiskClass::Low,
        timeout_ms: 10_000,
        cancellable: false,
        preconditions: vec![],
        success_criteria: vec![],
    }];
    let harness = build_robot_harness(RobotHarnessDependencies {
        config: RobotHarnessConfig::default(),
        world_state: world.clone(),
        executor: adapter,
        verifier,
        planner: Arc::new(DefaultPlanPort),
        episodes: episodes.clone(),
        policy: Arc::new(StubRobotPolicy),
        allowed_skills,
    })
    .unwrap();

    let mut session = RobotCognitiveSession::new(
        harness,
        clock,
        CancellationToken::new(),
        device,
    );
    let result = session
        .run_turn(turn_request(), &StubTurnServices, &NoopTurnEventSink)
        .await
        .expect("robot turn should succeed");
    assert_eq!(
        result.stop,
        TurnStop::Completed,
        "expected completed stop, output: {}",
        result.output
    );
    assert!(result.output.contains("Completed"), "output: {}", result.output);
    assert!(result.metrics.completed_normally);
}

struct RecordingEpisodes;
#[async_trait]
impl EpisodeSink for RecordingEpisodes {
    async fn append_attempt(
        &self,
        _episode_id: &str,
        _attempt: u32,
        _attempt_id: &str,
        _operation_id: Option<&OperationId>,
        _expected: &ExpectedOutcome,
        _before: Option<&WorldSnapshot>,
        _after: Option<&WorldSnapshot>,
        _result: Option<&SkillResult>,
        _verification: Option<&VerificationReport>,
    ) -> Result<(), String> {
        Ok(())
    }
    async fn close_episode(&self, _episode_id: &str, _outcome: &str) -> Result<(), String> {
        Ok(())
    }
}
