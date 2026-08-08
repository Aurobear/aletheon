//! Robot daemon-integration closure test (gap 3).
//!
//! Assembles the production RobotHarness composition and drives it through the
//! `CognitiveSession` turn contract. Uses a synchronous world (no background
//! pump) so the after-execution observation is deterministically visible to
//! Verify — the pump itself is covered by `world_state` unit tests.

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use cognit::harness::robot::session::RobotCognitiveSession;
use cognit::harness::robot::state::RobotHarnessConfig;
use cognit::harness::robot::{
    EmbodiedExecutionPort, EpisodeAuditPort, EpisodeSink, OutcomeVerifierPort, RobotPerceptionPort,
};
use cognit::harness::session::CognitiveSession;
use cognit::ports::policy_provider::{PolicyProviderError, PolicyProviderPort};
use executive::application::deterministic_outcome_verifier::DeterministicOutcomeVerifier;
use executive::application::embodied_execution_adapter::EmbodiedExecutionAdapter;
use executive::application::robot_harness_composition::{
    build_robot_harness, RobotHarnessDependencies,
};
use fabric::types::embodiment::{
    DeviceId, EmbodiedObservation, EmbodimentExecutionPort, RiskClass, SkillDescriptor,
    SkillDispatchError, SkillId, SkillOutcome, SkillRequest, SkillResult,
};
use fabric::types::episode_report::EpisodeSettlement;
use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::frame::FrameRef;
use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};
use fabric::types::perception_observation::PerceptionObservation;
use fabric::types::skill_proposal::{PolicyProvenance, SkillProposal};
use fabric::types::world_state::{WorldSnapshot, WorldStatePort};
use fabric::{
    ApprovalPolicy, ConnectionId, LocalOsPrincipal, MonoDeadline, MonoTime, NoopTurnEventSink,
    OperationId, PermissionProfileId, PrincipalContext, PrincipalId, ProcessId, StubTurnServices,
    ThreadId, TurnEvent, TurnEventSink, TurnRequest, TurnStop, WorkspacePolicy,
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

struct SessionPolicy;

struct OneFramePerception;

#[derive(Default)]
struct RecordingTurnEvents(Mutex<Vec<TurnEvent>>);

#[async_trait]
impl TurnEventSink for RecordingTurnEvents {
    async fn emit(&self, event: TurnEvent) {
        self.0.lock().unwrap().push(event);
    }
}

#[async_trait]
impl RobotPerceptionPort for OneFramePerception {
    async fn latest(
        &self,
        device: &DeviceId,
        _after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<PerceptionObservation>, String> {
        if limit == 0 {
            return Ok(Vec::new());
        }
        let digest = "a".repeat(64);
        Ok(vec![PerceptionObservation {
            device: device.clone(),
            schema: "camera.rgb".into(),
            schema_version: 1,
            frame: FrameRef {
                uri: format!("artifact://sha256/{digest}"),
                sha256: digest,
                mime_type: "image/jpeg".into(),
                width: 640,
                height: 480,
                byte_len: 32_000,
                source_time_ms: 1_000,
                camera_id: "front".into(),
                frame_id: 7,
            },
            labels: vec!["robot".into()],
            summary: "robot ahead".into(),
            confidence: 0.9,
            received_ms: 1_001,
        }])
    }
}

#[async_trait]
impl PolicyProviderPort for SessionPolicy {
    async fn propose(
        &self,
        _goal: &str,
        device: &DeviceId,
        _snapshots: &[WorldSnapshot],
        _visual: &[PerceptionObservation],
        _allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, PolicyProviderError> {
        Ok(vec![SkillProposal {
            skill: SkillId("kuavo.stance".into()),
            device: device.clone(),
            parameters: serde_json::json!({}),
            goal_alignment: fabric::types::skill_proposal::GoalAlignment::Direct,
            expected_outcome: ExpectedOutcome {
                predicate: OutcomePredicate::Equals {
                    path: "mode".into(),
                    value: serde_json::json!("stance"),
                },
                freshness_ms: 500,
                stable_window_ms: 0,
                timeout_ms: 5_000,
            },
            confidence: 0.9,
            frame_refs: vec![],
            provenance: PolicyProvenance {
                provider: "session-fixture".into(),
                model: "deterministic".into(),
                version: "1".into(),
                protocol_version: "1.0".into(),
                digest: "sha256:session-fixture".into(),
            },
        }])
    }

    async fn health(&self) -> Result<String, String> {
        Ok("ready".into())
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
            source_unix_ms: sequence as i64,
            received_unix_ms: sequence as i64,
            valid_until: Some(MonoDeadline::after(now, 2_000)),
            confidence: 1.0,
            reference_frame: None,
            frame: None,
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
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
            risk: RiskClass::Low,
            timeout_ms: 10_000,
            cancellable: false,
            preconditions: vec!["ready".into()],
            success_criteria: vec!["stable".into()],
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

struct QueueVerifier(Mutex<VecDeque<VerificationDecision>>);

#[async_trait]
impl OutcomeVerifierPort for QueueVerifier {
    async fn verify(
        &self,
        _expected: &ExpectedOutcome,
        _device: &DeviceId,
        _before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        _attempt: u32,
    ) -> VerificationReport {
        VerificationReport {
            decision: self
                .0
                .lock()
                .unwrap()
                .pop_front()
                .unwrap_or(VerificationDecision::Matched),
            evaluated_sequence: after.map(|snapshot| snapshot.sequence).unwrap_or(0),
            observed_paths: vec!["mode".into()],
            reasons: vec!["fixture verification".into()],
            evidence: vec![],
        }
    }
}

#[async_trait]
impl WorldStatePort for DirectWorld {
    async fn latest(&self, device: &DeviceId, _schema: &str) -> Option<WorldSnapshot> {
        let observation = self.executor.get_state(device).await.ok()??;
        Some(WorldSnapshot {
            device: device.clone(),
            schema: observation.schema,
            schema_version: observation.schema_version,
            sequence: observation.sequence,
            payload: observation.payload,
            observed_at: observation.source_time,
            valid_until: None,
            stale: false,
        })
    }
    async fn observe_until(
        &self,
        device: &DeviceId,
        _schema: &str,
        _after_sequence: u64,
        _deadline: MonoDeadline,
    ) -> Option<WorldSnapshot> {
        self.latest(device, fabric::types::world_state::ANY_SCHEMA)
            .await
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
        execution_target: fabric::ExecutionTargetSelection::default(),
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
    let verifier: Arc<dyn OutcomeVerifierPort> = Arc::new(DeterministicOutcomeVerifier::new(
        world.clone(),
        clock.clone(),
        vec![],
    ));
    let episodes: Arc<dyn EpisodeSink> = Arc::new(RecordingEpisodes::default());
    let allowed_skills = vec![SkillDescriptor {
        skill: SkillId("kuavo.stance".into()),
        device: device.clone(),
        summary: "stance".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }),
        risk: RiskClass::Low,
        timeout_ms: 10_000,
        cancellable: false,
        preconditions: vec!["ready".into()],
        success_criteria: vec!["stable".into()],
    }];
    let harness = build_robot_harness(RobotHarnessDependencies {
        config: RobotHarnessConfig::default(),
        world_state: world.clone(),
        executor: adapter,
        verifier,
        episodes: episodes.clone(),
        policy: Arc::new(SessionPolicy),
        perception: Arc::new(OneFramePerception),
        allowed_skills,
    })
    .unwrap();

    let audit = Arc::new(RecordingAudit::default());
    let mut session = RobotCognitiveSession::new(
        harness,
        clock,
        CancellationToken::new(),
        device,
        "mujoco-v1",
        "abc123",
        "sha256:proto",
        "sha256:skills",
        None,
    )
    .with_auditor(audit.clone());
    let turn_events = RecordingTurnEvents::default();
    let result = session
        .run_turn(turn_request(), &StubTurnServices, &turn_events)
        .await
        .expect("robot turn should succeed");
    assert_eq!(
        result.stop,
        TurnStop::Completed,
        "expected completed stop, output: {}",
        result.output
    );
    // The output is the structured EpisodeReport — parse it and verify the
    // settlement is authoritative.
    let report: fabric::types::episode_report::EpisodeReport = serde_json::from_str(&result.output)
        .expect("turn output must be a structured EpisodeReport");
    assert_eq!(report.settlement, EpisodeSettlement::Completed);
    assert_eq!(report.sim_scene_version, "mujoco-v1");
    assert_eq!(report.bridge_protocol_digest, "sha256:proto");
    assert_eq!(report.skill_descriptor_digest, "sha256:skills");
    let provenance = report.policy_provenance.as_ref().unwrap();
    assert_eq!(provenance.provider, "session-fixture");
    assert_eq!(provenance.model, "deterministic");
    assert_eq!(provenance.version, "1");
    assert_eq!(provenance.protocol_version, "1.0");
    assert_eq!(provenance.digest, "sha256:session-fixture");
    assert_eq!(report.attempts[0].verification_observed_paths, ["mode"]);
    assert!(
        report.can_promote(),
        "matched + settled episode should promote"
    );
    assert_eq!(report.selected_frames.len(), 1);
    assert_eq!(report.artifacts.len(), 1);
    assert_eq!(report.artifacts[0].kind, "frame");
    let emitted = turn_events
        .0
        .lock()
        .unwrap()
        .iter()
        .find_map(|event| match event {
            TurnEvent::RobotEpisodeSettled { receipt } => {
                receipt
                    .verify_integrity()
                    .expect("emitted receipt integrity");
                Some(receipt.clone())
            }
            _ => None,
        })
        .expect("Robot session must emit its immutable settled receipt");
    assert_eq!(emitted.report(), &report);
    assert_eq!(
        audit.report_digests.lock().unwrap().as_slice(),
        [emitted.report_sha256()]
    );
    assert_eq!(
        report.artifacts[0].digest.as_deref(),
        Some(report.selected_frames[0].sha256.as_str())
    );
    assert_eq!(
        report.artifacts[0].producer.as_deref(),
        Some("camera:front")
    );
    assert!(result.metrics.completed_normally);
    assert_eq!(audit.report_digests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn episode_report_contains_every_durable_retry_attempt() {
    let clock = Arc::new(TestClock::default());
    let executor: Arc<dyn EmbodimentExecutionPort> = Arc::new(StanceSim::new());
    let device = DeviceId("bot".into());
    let world: Arc<dyn WorldStatePort> = Arc::new(DirectWorld {
        executor: executor.clone(),
    });
    let adapter: Arc<dyn EmbodiedExecutionPort> = Arc::new(EmbodiedExecutionAdapter::new(executor));
    let verifier: Arc<dyn OutcomeVerifierPort> =
        Arc::new(QueueVerifier(Mutex::new(VecDeque::from([
            VerificationDecision::RetryableMismatch,
            VerificationDecision::Matched,
        ]))));
    let episodes = Arc::new(RecordingEpisodes::default());
    let harness = build_robot_harness(RobotHarnessDependencies {
        config: RobotHarnessConfig {
            max_retries: 1,
            max_replans: 0,
            ..RobotHarnessConfig::default()
        },
        world_state: world,
        executor: adapter,
        verifier,
        episodes: episodes.clone(),
        policy: Arc::new(SessionPolicy),
        perception: Arc::new(cognit::harness::robot::NoopRobotPerception),
        allowed_skills: vec![SkillDescriptor {
            skill: SkillId("kuavo.stance".into()),
            device: device.clone(),
            summary: "stance".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
            risk: RiskClass::Low,
            timeout_ms: 10_000,
            cancellable: false,
            preconditions: vec!["ready".into()],
            success_criteria: vec!["stable".into()],
        }],
    })
    .unwrap();
    let mut session = RobotCognitiveSession::new(
        harness,
        clock,
        CancellationToken::new(),
        device,
        "mujoco-v1",
        "abc123",
        "sha256:proto",
        "sha256:skills",
        None,
    );

    let result = session
        .run_turn(turn_request(), &StubTurnServices, &NoopTurnEventSink)
        .await
        .unwrap();
    let report: fabric::types::episode_report::EpisodeReport =
        serde_json::from_str(&result.output).unwrap();
    assert_eq!(result.stop, TurnStop::Completed);
    assert_eq!(report.attempts.len(), 2);
    assert_eq!(report.attempts[0].attempt, 1);
    assert_eq!(report.attempts[1].attempt, 2);
    assert_ne!(
        report.attempts[0].operation_id, report.attempts[1].operation_id,
        "every attempt must retain its own operation identity"
    );
    assert!(report
        .attempts
        .iter()
        .all(|attempt| attempt.before_sequence.is_some()
            && attempt.after_sequence.is_some()
            && attempt.verified_sequence.is_some()));
    assert_eq!(episodes.attempts.lock().unwrap().len(), 2);
}

#[tokio::test]
async fn pre_cancelled_robot_turn_safe_stops_and_returns_typed_report() {
    let clock = Arc::new(TestClock::default());
    let executor: Arc<dyn EmbodimentExecutionPort> = Arc::new(StanceSim::new());
    let device = DeviceId("bot".into());
    let world: Arc<dyn WorldStatePort> = Arc::new(DirectWorld {
        executor: executor.clone(),
    });
    let adapter: Arc<dyn EmbodiedExecutionPort> = Arc::new(EmbodiedExecutionAdapter::new(executor));
    let episodes = Arc::new(RecordingEpisodes::default());
    let harness = build_robot_harness(RobotHarnessDependencies {
        config: RobotHarnessConfig::default(),
        world_state: world,
        executor: adapter,
        verifier: Arc::new(QueueVerifier(Mutex::new(VecDeque::new()))),
        episodes: episodes.clone(),
        policy: Arc::new(SessionPolicy),
        perception: Arc::new(cognit::harness::robot::NoopRobotPerception),
        allowed_skills: vec![SkillDescriptor {
            skill: SkillId("kuavo.stance".into()),
            device: device.clone(),
            summary: "stance".into(),
            input_schema: serde_json::json!({
                "type": "object",
                "properties": {},
                "required": [],
                "additionalProperties": false
            }),
            risk: RiskClass::Low,
            timeout_ms: 10_000,
            cancellable: false,
            preconditions: vec!["ready".into()],
            success_criteria: vec!["stable".into()],
        }],
    })
    .unwrap();
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let mut session = RobotCognitiveSession::new(
        harness,
        clock,
        cancellation,
        device,
        "mujoco-v1",
        "abc123",
        "sha256:proto",
        "sha256:skills",
        None,
    );

    let result = session
        .run_turn(turn_request(), &StubTurnServices, &NoopTurnEventSink)
        .await
        .unwrap();
    let report: fabric::types::episode_report::EpisodeReport =
        serde_json::from_str(&result.output).unwrap();
    assert_eq!(result.stop, TurnStop::Cancelled);
    assert_eq!(report.settlement, EpisodeSettlement::Cancelled);
    assert_eq!(
        report.safe_stop.as_ref().unwrap().outcome,
        fabric::types::episode_report::SafeStopOutcome::Succeeded
    );
    assert_eq!(
        report.safe_stop.as_ref().unwrap().attempted_after_attempt,
        0
    );
    assert_eq!(
        report.failures[0].class,
        fabric::types::robot_failure::RobotFailureClass::Cancelled
    );
    assert_eq!(
        episodes.settlements.lock().unwrap().as_slice(),
        ["cancelled"]
    );
}

#[derive(Default)]
struct RecordingEpisodes {
    attempts: Mutex<Vec<fabric::types::episode_report::AttemptRecord>>,
    settlements: Mutex<Vec<String>>,
}

#[derive(Default)]
struct RecordingAudit {
    report_digests: Mutex<Vec<String>>,
}

#[async_trait]
impl EpisodeAuditPort for RecordingAudit {
    async fn record(
        &self,
        report: &fabric::types::episode_report::SettledEpisodeReport,
    ) -> Result<(), String> {
        report.verify_integrity()?;
        self.report_digests
            .lock()
            .unwrap()
            .push(report.report_sha256().to_owned());
        Ok(())
    }
}

#[async_trait]
impl EpisodeSink for RecordingEpisodes {
    async fn append_attempt(
        &self,
        _episode_id: &str,
        attempt: u32,
        attempt_id: &str,
        operation_id: Option<&OperationId>,
        request: &SkillRequest,
        expected: &ExpectedOutcome,
        before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        result: Option<&SkillResult>,
        verification: Option<&VerificationReport>,
    ) -> Result<(), String> {
        let mut record = fabric::types::episode_report::AttemptRecord::from_verification(
            attempt,
            attempt_id.to_string(),
            operation_id.map(|op| op.0.to_string()),
            Some(request.clone()),
            expected.clone(),
            result.map(|r| format!("{:?}", r.outcome)),
            verification,
            None,
            before.map(|snapshot| snapshot.sequence),
            after.map(|snapshot| snapshot.sequence),
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
        let mut attempts = self.attempts.lock().unwrap();
        if let Some(record) = attempts.iter_mut().find(|a| a.attempt_id == attempt_id) {
            record.verification_decision = Some(verification.decision.clone());
            record.verification_observed_paths = verification.observed_paths.clone();
            record.verification_reasons = verification.reasons.clone();
            record.after_sequence = after.map(|snapshot| snapshot.sequence);
            record.verified_sequence = Some(verification.evaluated_sequence);
            record.evidence_refs.extend(verification.evidence.clone());
        }
        Ok(())
    }
    async fn load_attempts(
        &self,
        _episode_id: &str,
    ) -> Result<Vec<fabric::types::episode_report::AttemptRecord>, String> {
        Ok(self.attempts.lock().unwrap().clone())
    }
}
