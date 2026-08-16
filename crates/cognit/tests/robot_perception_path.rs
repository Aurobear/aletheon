use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};

use ::contracts::types::embodiment::{
    DeviceId, RiskClass, SkillDescriptor, SkillId, SkillRequest, SkillResult,
};
use ::contracts::types::episode_report::EpisodeSettlement;
use ::contracts::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use ::contracts::types::frame::FrameRef;
use ::contracts::types::outcome_verification::VerificationReport;
use ::contracts::types::skill_proposal::{PolicyProvenance, SkillProposal};
use ::contracts::types::world_state::{WorldSnapshot, WorldStatePort};
use ::contracts::{MonoDeadline, MonoTime, OperationId};
use async_trait::async_trait;
use cognit::harness::robot::state::{RobotHarnessConfig, RobotState};
use cognit::harness::robot::PerceptionObservation;
use cognit::harness::robot::{
    EmbodiedExecutionPort, EpisodeSink, OutcomeVerifierPort, RobotExecutionError, RobotHarness,
    RobotPerceptionPort,
};
use cognit::ports::policy_provider::{PolicyProviderError, PolicyProviderPort};

struct FreshWorld;

#[async_trait]
impl WorldStatePort for FreshWorld {
    async fn latest(&self, device: &DeviceId, _schema: &str) -> Option<WorldSnapshot> {
        Some(WorldSnapshot {
            device: device.clone(),
            schema: "robot.state".into(),
            schema_version: 1,
            sequence: 1,
            payload: serde_json::json!({"mode": "ready"}),
            observed_at: MonoTime(1),
            valid_until: None,
            stale: false,
        })
    }

    async fn latest_all(&self, device: &DeviceId) -> Vec<WorldSnapshot> {
        vec![
            self.latest(device, "robot.state").await.unwrap(),
            WorldSnapshot {
                device: device.clone(),
                schema: "secondary.state".into(),
                schema_version: 1,
                sequence: 9,
                payload: serde_json::json!({"secondary": true}),
                observed_at: MonoTime(2),
                valid_until: None,
                stale: false,
            },
        ]
    }

    async fn observe_until(
        &self,
        _device: &DeviceId,
        _schema: &str,
        _after_sequence: u64,
        _deadline: MonoDeadline,
    ) -> Option<WorldSnapshot> {
        None
    }
}

struct UnusedExecutor;
#[async_trait]
impl EmbodiedExecutionPort for UnusedExecutor {
    async fn execute(&self, _request: SkillRequest) -> Result<SkillResult, RobotExecutionError> {
        Err(RobotExecutionError::Control("unused".into()))
    }
    async fn cancel(&self, _device: &DeviceId) -> Result<(), RobotExecutionError> {
        Ok(())
    }
    async fn safe_stop(&self, _device: &DeviceId) -> Result<(), RobotExecutionError> {
        Ok(())
    }
}

struct UnusedVerifier;
#[async_trait]
impl OutcomeVerifierPort for UnusedVerifier {
    async fn verify(
        &self,
        _expected: &ExpectedOutcome,
        _device: &DeviceId,
        _before: Option<&WorldSnapshot>,
        _after: Option<&WorldSnapshot>,
        _attempt: u32,
    ) -> VerificationReport {
        unreachable!("Plan-only test")
    }
}

struct UnusedEpisodes;
#[async_trait]
impl EpisodeSink for UnusedEpisodes {
    async fn append_attempt(
        &self,
        _episode_id: &str,
        _attempt: u32,
        _attempt_id: &str,
        _operation_id: Option<&OperationId>,
        _request: &SkillRequest,
        _expected: &ExpectedOutcome,
        _before: Option<&WorldSnapshot>,
        _after: Option<&WorldSnapshot>,
        _result: Option<&SkillResult>,
        _verification: Option<&VerificationReport>,
    ) -> Result<(), String> {
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
    ) -> Result<Vec<::contracts::types::episode_report::AttemptRecord>, String> {
        Ok(Vec::new())
    }
}

struct FixedPerception(Vec<PerceptionObservation>);
#[async_trait]
impl RobotPerceptionPort for FixedPerception {
    async fn latest(
        &self,
        _device: &DeviceId,
        _after_sequence: Option<u64>,
        limit: usize,
    ) -> Result<Vec<PerceptionObservation>, String> {
        Ok(self.0.iter().take(limit).cloned().collect())
    }
}

struct RecordingPolicy {
    calls: AtomicUsize,
    snapshots: Mutex<Vec<WorldSnapshot>>,
    visual: Mutex<Vec<PerceptionObservation>>,
}

impl RecordingPolicy {
    fn new() -> Self {
        Self {
            calls: AtomicUsize::new(0),
            snapshots: Mutex::new(Vec::new()),
            visual: Mutex::new(Vec::new()),
        }
    }
}

#[async_trait]
impl PolicyProviderPort for RecordingPolicy {
    async fn propose(
        &self,
        _goal: &str,
        device: &DeviceId,
        snapshots: &[WorldSnapshot],
        visual: &[PerceptionObservation],
        _allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, cognit::ports::policy_provider::PolicyProviderError> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.snapshots
            .lock()
            .unwrap()
            .clone_from(&snapshots.to_vec());
        self.visual.lock().unwrap().clone_from(&visual.to_vec());
        Ok(vec![SkillProposal {
            skill: SkillId("inspect".into()),
            device: device.clone(),
            parameters: serde_json::json!({}),
            goal_alignment: ::contracts::types::skill_proposal::GoalAlignment::Direct,
            expected_outcome: ExpectedOutcome {
                predicate: OutcomePredicate::Equals {
                    path: "mode".into(),
                    value: serde_json::json!("ready"),
                },
                freshness_ms: 500,
                stable_window_ms: 0,
                timeout_ms: 1_000,
            },
            confidence: 0.9,
            // Deliberately empty: RobotHarness must bind host-selected provenance.
            frame_refs: Vec::new(),
            provenance: PolicyProvenance {
                provider: "recording".into(),
                model: "test".into(),
                version: "1".into(),
                protocol_version: "1.0".into(),
                digest: "sha256:model".into(),
            },
        }])
    }

    async fn health(&self) -> Result<String, String> {
        Ok("ready".into())
    }
}

struct InvalidPolicy;

#[async_trait]
impl PolicyProviderPort for InvalidPolicy {
    async fn propose(
        &self,
        _goal: &str,
        device: &DeviceId,
        _snapshots: &[WorldSnapshot],
        _visual: &[PerceptionObservation],
        _allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, PolicyProviderError> {
        let mut unknown = valid_proposal(device.clone());
        unknown.skill = SkillId("unregistered.raw_action".into());
        let mut wrong_device = valid_proposal(DeviceId("other".into()));
        wrong_device.confidence = f32::NAN;
        Ok(vec![unknown, wrong_device])
    }

    async fn health(&self) -> Result<String, String> {
        Ok("ready".into())
    }
}

struct TimeoutPolicy(AtomicUsize);

#[async_trait]
impl PolicyProviderPort for TimeoutPolicy {
    async fn propose(
        &self,
        _goal: &str,
        _device: &DeviceId,
        _snapshots: &[WorldSnapshot],
        _visual: &[PerceptionObservation],
        _allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, PolicyProviderError> {
        self.0.fetch_add(1, Ordering::SeqCst);
        Err(PolicyProviderError::Timeout)
    }

    async fn health(&self) -> Result<String, String> {
        Ok("ready".into())
    }
}

fn valid_proposal(device: DeviceId) -> SkillProposal {
    SkillProposal {
        skill: SkillId("inspect".into()),
        device,
        parameters: serde_json::json!({}),
        goal_alignment: ::contracts::types::skill_proposal::GoalAlignment::Direct,
        expected_outcome: ExpectedOutcome {
            predicate: OutcomePredicate::Equals {
                path: "mode".into(),
                value: serde_json::json!("ready"),
            },
            freshness_ms: 500,
            stable_window_ms: 0,
            timeout_ms: 1_000,
        },
        confidence: 0.9,
        frame_refs: vec![],
        provenance: PolicyProvenance {
            provider: "fixture".into(),
            model: "fixture".into(),
            version: "1".into(),
            protocol_version: "1.0".into(),
            digest: "sha256:fixture".into(),
        },
    }
}

fn descriptor() -> SkillDescriptor {
    SkillDescriptor {
        skill: SkillId("inspect".into()),
        device: DeviceId("bot".into()),
        summary: "inspect current scene".into(),
        input_schema: serde_json::json!({
            "type": "object",
            "properties": {},
            "required": [],
            "additionalProperties": false
        }),
        risk: RiskClass::Read,
        timeout_ms: 1_000,
        cancellable: true,
        preconditions: vec!["ready".into()],
        success_criteria: vec!["inspected".into()],
    }
}

fn perception(device: &str) -> PerceptionObservation {
    let digest = "a".repeat(64);
    PerceptionObservation {
        device: DeviceId(device.into()),
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
    }
}

fn harness(
    config: RobotHarnessConfig,
    policy: Arc<dyn PolicyProviderPort>,
    perception: Arc<dyn RobotPerceptionPort>,
) -> RobotHarness {
    RobotHarness::new(
        config,
        Arc::new(FreshWorld),
        Arc::new(UnusedExecutor),
        Arc::new(UnusedVerifier),
        Arc::new(UnusedEpisodes),
        policy,
        perception,
        vec![descriptor()],
    )
}

async fn observe_then_plan(harness: &RobotHarness) -> cognit::harness::robot::RobotHarnessState {
    let state = harness.init(DeviceId("bot".into()), "inspect".into(), "episode".into());
    let state = harness.step(state).await;
    assert_eq!(state.state, RobotState::Plan);
    harness.step(state).await
}

#[tokio::test]
async fn plan_passes_nonempty_visual_and_records_host_owned_frame_provenance() {
    let policy = Arc::new(RecordingPolicy::new());
    let harness = harness(
        RobotHarnessConfig::default(),
        policy.clone(),
        Arc::new(FixedPerception(vec![perception("bot")])),
    );
    let state = observe_then_plan(&harness).await;
    assert_eq!(state.state, RobotState::Authorize);
    assert_eq!(policy.snapshots.lock().unwrap().len(), 2);
    assert_eq!(policy.visual.lock().unwrap().len(), 1);
    assert_eq!(state.latest_frame_refs.len(), 1);
    assert_eq!(state.latest_frame_refs[0].frame_id, 7);
}

#[tokio::test]
async fn required_perception_missing_fails_closed_before_policy_call() {
    let policy = Arc::new(RecordingPolicy::new());
    let mut config = RobotHarnessConfig::default();
    config
        .required_perception
        .insert(SkillId("inspect".into()), vec![("camera.rgb".into(), 1)]);
    let harness = harness(
        config,
        policy.clone(),
        Arc::new(FixedPerception(Vec::new())),
    );
    let state = observe_then_plan(&harness).await;
    assert_eq!(state.state, RobotState::SafeStop);
    assert!(state.failures[0].detail.contains("required perception"));
    assert_eq!(policy.calls.load(Ordering::SeqCst), 0);
}

#[tokio::test]
async fn nonvisual_skill_and_device_isolation_are_both_enforced() {
    let policy = Arc::new(RecordingPolicy::new());
    let no_visual = harness(
        RobotHarnessConfig::default(),
        policy.clone(),
        Arc::new(FixedPerception(Vec::new())),
    );
    assert_eq!(
        observe_then_plan(&no_visual).await.state,
        RobotState::Authorize
    );

    let wrong_device = harness(
        RobotHarnessConfig::default(),
        policy,
        Arc::new(FixedPerception(vec![perception("other")])),
    );
    let state = observe_then_plan(&wrong_device).await;
    assert_eq!(state.state, RobotState::SafeStop);
    assert!(state.failures[0]
        .detail
        .contains("perception device mismatch"));
}

#[tokio::test]
async fn multiple_invalid_proposals_fail_with_bounded_rejection_evidence() {
    let harness = harness(
        RobotHarnessConfig::default(),
        Arc::new(InvalidPolicy),
        Arc::new(FixedPerception(Vec::new())),
    );
    let state = observe_then_plan(&harness).await;
    assert_eq!(state.state, RobotState::SafeStop);
    let error = &state.failures[0].detail;
    assert!(error.starts_with("policy_all_proposals_rejected:"));
    assert!(error.contains("proposal[0]"));
    assert!(error.contains("proposal[1]"));
    assert!(state.latest_skill_request.is_none());
}

#[tokio::test]
async fn policy_timeout_is_terminal_for_plan_and_not_retried() {
    let policy = Arc::new(TimeoutPolicy(AtomicUsize::new(0)));
    let harness = harness(
        RobotHarnessConfig::default(),
        policy.clone(),
        Arc::new(FixedPerception(Vec::new())),
    );
    let state = observe_then_plan(&harness).await;
    assert_eq!(state.state, RobotState::SafeStop);
    assert_eq!(
        state.failures[0].detail,
        "policy_timeout: gateway request exceeded the configured deadline"
    );
    assert_eq!(policy.0.load(Ordering::SeqCst), 1);
}
