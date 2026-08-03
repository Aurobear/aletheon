//! PR1 `robot-harness-carries-proposal-outcome` closure test.
//!
//! Drives the RobotHarness state machine end-to-end with a policy proposal that
//! carries a NON-stance expected outcome, and asserts that Execute/Verify use the
//! proposal's outcome — not a hardcoded `mode == stance`.
//! PR2 companion assertions: every recorded attempt carries a real UUID
//! `OperationId`, never the legacy `"op"` placeholder.

use cognit::harness::robot::state::RobotHarnessConfig;
use cognit::harness::robot::{
    EmbodiedExecutionPort, EpisodeSink, OutcomeVerifierPort, PlanPort, RobotHarness,
};
use cognit::ports::policy_provider::PolicyProviderPort;
use fabric::types::embodiment::{
    DeviceId, RiskClass, SkillDescriptor, SkillId, SkillOutcome, SkillRequest, SkillResult,
};
use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};
use fabric::types::perception_observation::PerceptionObservation;
use fabric::types::skill_proposal::{PolicyProvenance, SkillProposal};
use fabric::types::world_state::{WorldSnapshot, WorldStatePort};
use fabric::{MonoDeadline, OperationId};
use std::sync::{Arc, Mutex};

struct FakeWorldState;
#[async_trait::async_trait]
impl WorldStatePort for FakeWorldState {
    async fn latest(&self, _device: &DeviceId) -> Option<WorldSnapshot> {
        None
    }
    async fn observe_until(
        &self,
        _device: &DeviceId,
        _after_sequence: u64,
        _deadline: MonoDeadline,
    ) -> Option<WorldSnapshot> {
        None
    }
}

struct FakeExecutor;
#[async_trait::async_trait]
impl EmbodiedExecutionPort for FakeExecutor {
    async fn execute(&self, request: SkillRequest) -> Result<SkillResult, String> {
        Ok(SkillResult {
            operation_id: OperationId::new(),
            skill: request.skill,
            device: request.device,
            outcome: SkillOutcome::Succeeded,
            duration_ms: 10,
            evidence: vec![],
        })
    }
    async fn cancel(&self, _device: &DeviceId) -> Result<(), String> {
        Ok(())
    }
    async fn safe_stop(&self, _device: &DeviceId) -> Result<(), String> {
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

struct UnusedPlanner;
#[async_trait::async_trait]
impl PlanPort for UnusedPlanner {
    async fn plan(
        &self,
        _device: &DeviceId,
        _snapshot: &WorldSnapshot,
        _goal: &str,
    ) -> Result<SkillRequest, String> {
        Err("plan not used in this test".into())
    }
    async fn replan(
        &self,
        _device: &DeviceId,
        _snapshot: &WorldSnapshot,
        _failure_reason: &str,
    ) -> Result<SkillRequest, String> {
        Err("replan not used in this test".into())
    }
}

#[derive(Default)]
struct RecordingEpisodes {
    operation_ids: Mutex<Vec<String>>,
}
#[async_trait::async_trait]
impl EpisodeSink for RecordingEpisodes {
    async fn append_attempt(
        &self,
        _episode_id: &str,
        _attempt: u32,
        operation_id: &str,
        _expected: &ExpectedOutcome,
        _before: Option<&WorldSnapshot>,
        _after: Option<&WorldSnapshot>,
        _result: Option<&SkillResult>,
        _verification: Option<&VerificationReport>,
    ) -> Result<(), String> {
        self.operation_ids
            .lock()
            .unwrap()
            .push(operation_id.to_string());
        Ok(())
    }
    async fn close_episode(&self, _episode_id: &str, _outcome: &str) -> Result<(), String> {
        Ok(())
    }
}

/// Policy returns a proposal whose expected outcome is `mode == "stance2"`.
struct Stance2Policy;
#[async_trait::async_trait]
impl PolicyProviderPort for Stance2Policy {
    async fn propose(
        &self,
        _goal: &str,
        device: &DeviceId,
        _snapshots: &[WorldSnapshot],
        _visual: &[PerceptionObservation],
        _allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, String> {
        Ok(vec![SkillProposal {
            skill: SkillId("kuavo.stance".into()),
            device: device.clone(),
            parameters: serde_json::json!({}),
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
        input_schema: serde_json::json!({"type": "object", "required": []}),
        risk: RiskClass::Low,
        timeout_ms: 10000,
        cancellable: false,
        preconditions: vec![],
        success_criteria: vec![],
    }]
}

#[tokio::test]
async fn verify_uses_proposal_expected_outcome_not_hardcoded_stance() {
    let verifier = Arc::new(RecordingVerifier::default());
    let episodes = Arc::new(RecordingEpisodes::default());

    let harness = RobotHarness::new(
        RobotHarnessConfig::default(),
        Arc::new(FakeWorldState),
        Arc::new(FakeExecutor),
        verifier.clone(),
        Arc::new(UnusedPlanner),
        episodes.clone(),
        Arc::new(Stance2Policy),
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
        .expect("verifier.verify was never called");
    assert_eq!(
        received.predicate,
        OutcomePredicate::Equals {
            path: "mode".into(),
            value: serde_json::json!("stance2"),
        },
        "verify must use the proposal's expected outcome, not a hardcoded stance"
    );

    // PR2: every recorded attempt carries a real UUID operation id, never "op".
    let ops = episodes.operation_ids.lock().unwrap();
    assert!(!ops.is_empty(), "at least one attempt should be recorded");
    assert!(
        !ops.iter().any(|id| id == "op"),
        "legacy \"op\" placeholder must not be recorded"
    );
    for id in ops.iter() {
        assert!(
            id.parse::<OperationId>().is_ok(),
            "recorded operation id must be a valid OperationId: {id}"
        );
    }
}
