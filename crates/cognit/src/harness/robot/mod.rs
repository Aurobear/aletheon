//! Bounded RobotHarness — Observe, Plan, Authorize, Execute, Verify, Retry, Replan, Recover, Settle, SafeStop.
//! Cognit owns the state machine; it does NOT depend on Executive or Hardware.

pub mod proposal_validator;
pub mod state;

use crate::harness::robot::proposal_validator::validate_proposal;
use crate::harness::robot::state::{RobotHarnessConfig, RobotState, VerificationSignal};
use crate::ports::policy_provider::PolicyProviderPort;
use async_trait::async_trait;
use fabric::types::embodiment::{DeviceId, SkillDescriptor, SkillId, SkillRequest, SkillResult};
use fabric::types::expected_outcome::ExpectedOutcome;
use fabric::types::outcome_verification::VerificationReport;
use fabric::types::world_state::{WorldSnapshot, WorldStatePort};
use std::sync::Arc;

/// Port for executing an embodied skill. Injected by Executive.
#[async_trait]
pub trait EmbodiedExecutionPort: Send + Sync {
    async fn execute(&self, request: SkillRequest) -> Result<SkillResult, String>;
    async fn cancel(&self, device: &DeviceId) -> Result<(), String>;
    async fn safe_stop(&self, device: &DeviceId) -> Result<(), String>;
}

/// Port for verifying outcomes. Injected by Executive (delegates to Metacog).
#[async_trait]
pub trait OutcomeVerifierPort: Send + Sync {
    async fn verify(
        &self,
        expected: &ExpectedOutcome,
        before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        attempt: u32,
    ) -> VerificationReport;
}

/// Port for plan generation. Injected — may use LLM or policy.
#[async_trait]
pub trait PlanPort: Send + Sync {
    async fn plan(
        &self,
        device: &DeviceId,
        snapshot: &WorldSnapshot,
        goal: &str,
    ) -> Result<SkillRequest, String>;
    async fn replan(
        &self,
        device: &DeviceId,
        snapshot: &WorldSnapshot,
        failure_reason: &str,
    ) -> Result<SkillRequest, String>;
}

/// Port for episode persistence. Injected.
#[async_trait]
pub trait EpisodeSink: Send + Sync {
    async fn append_attempt(
        &self,
        episode_id: &str,
        attempt: u32,
        operation_id: &str,
        expected: &ExpectedOutcome,
        before: Option<&WorldSnapshot>,
        after: Option<&WorldSnapshot>,
        result: Option<&SkillResult>,
        verification: Option<&VerificationReport>,
    ) -> Result<(), String>;
    async fn close_episode(&self, episode_id: &str, outcome: &str) -> Result<(), String>;
}

#[derive(Debug, Clone)]
pub struct RobotHarnessState {
    pub state: RobotState,
    pub device: DeviceId,
    pub goal: String,
    pub episode_id: String,
    pub attempt: u32,
    pub retries_used: u32,
    pub replans_used: u32,
    pub latest_snapshot: Option<WorldSnapshot>,
    pub latest_skill_request: Option<SkillRequest>,
    pub latest_skill_result: Option<SkillResult>,
    pub latest_verification: Option<VerificationReport>,
    pub error: Option<String>,
}

pub struct RobotHarness {
    config: RobotHarnessConfig,
    world_state: Arc<dyn WorldStatePort>,
    executor: Arc<dyn EmbodiedExecutionPort>,
    verifier: Arc<dyn OutcomeVerifierPort>,
    planner: Arc<dyn PlanPort>,
    episodes: Arc<dyn EpisodeSink>,
    policy: Arc<dyn PolicyProviderPort>,
    allowed_skills: Vec<SkillDescriptor>,
}

impl RobotHarness {
    pub fn new(
        config: RobotHarnessConfig,
        world_state: Arc<dyn WorldStatePort>,
        executor: Arc<dyn EmbodiedExecutionPort>,
        verifier: Arc<dyn OutcomeVerifierPort>,
        planner: Arc<dyn PlanPort>,
        episodes: Arc<dyn EpisodeSink>,
        policy: Arc<dyn PolicyProviderPort>,
        allowed_skills: Vec<SkillDescriptor>,
    ) -> Self {
        Self {
            config,
            world_state,
            executor,
            verifier,
            planner,
            episodes,
            policy,
            allowed_skills,
        }
    }

    pub fn init(&self, device: DeviceId, goal: String, episode_id: String) -> RobotHarnessState {
        RobotHarnessState {
            state: RobotState::Observe,
            device,
            goal,
            episode_id,
            attempt: 0,
            retries_used: 0,
            replans_used: 0,
            latest_snapshot: None,
            latest_skill_request: None,
            latest_skill_result: None,
            latest_verification: None,
            error: None,
        }
    }

    pub async fn step(&self, mut harness_state: RobotHarnessState) -> RobotHarnessState {
        match harness_state.state {
            RobotState::Observe => {
                let snap = self.world_state.latest(&harness_state.device).await;
                harness_state.latest_snapshot = snap;
                harness_state.state = harness_state.state.next(&VerificationSignal::Matched);
            }
            RobotState::Plan => {
                let snapshots: Vec<WorldSnapshot> =
                    harness_state.latest_snapshot.iter().cloned().collect();
                match self
                    .policy
                    .propose(
                        &harness_state.goal,
                        &harness_state.device,
                        &snapshots,
                        &[],
                        &self.allowed_skills,
                    )
                    .await
                {
                    Ok(proposals) => {
                        let mut valid_request = None;
                        for proposal in &proposals {
                            if validate_proposal(proposal, &self.allowed_skills, 0, 0).is_ok() {
                                valid_request = Some(SkillRequest {
                                    skill: proposal.skill.clone(),
                                    device: proposal.device.clone(),
                                    parameters: proposal.parameters.clone(),
                                });
                                break;
                            }
                        }
                        match valid_request {
                            Some(req) => {
                                harness_state.latest_skill_request = Some(req);
                                harness_state.state =
                                    harness_state.state.next(&VerificationSignal::Matched);
                            }
                            None => {
                                harness_state.error = Some("no valid policy proposal".into());
                                harness_state.state = RobotState::Failed;
                            }
                        }
                    }
                    Err(e) => {
                        harness_state.error = Some(e);
                        harness_state.state = RobotState::Failed;
                    }
                }
            }
            RobotState::Authorize => {
                // Authorization is a no-op here — Executive's Kernel handles permit/lease.
                // RobotHarness just transitions.
                harness_state.state = harness_state.state.next(&VerificationSignal::Matched);
            }
            RobotState::Execute => {
                harness_state.attempt += 1;
                let request = harness_state
                    .latest_skill_request
                    .clone()
                    .unwrap_or_else(|| SkillRequest {
                        skill: SkillId("kuavo.stance".into()),
                        device: harness_state.device.clone(),
                        parameters: serde_json::json!({}),
                    });
                let before_snap = harness_state.latest_snapshot.clone();
                match self.executor.execute(request).await {
                    Ok(result) => {
                        harness_state.latest_skill_result = Some(result);
                        let _ = self
                            .episodes
                            .append_attempt(
                                &harness_state.episode_id,
                                harness_state.attempt,
                                "op",
                                &ExpectedOutcome {
                                    predicate:
                                        fabric::types::expected_outcome::OutcomePredicate::Equals {
                                            path: "mode".into(),
                                            value: serde_json::json!("stance"),
                                        },
                                    freshness_ms: 500,
                                    stable_window_ms: 0,
                                    timeout_ms: 5000,
                                },
                                before_snap.as_ref(),
                                None,
                                harness_state.latest_skill_result.as_ref(),
                                None,
                            )
                            .await;
                        harness_state.state =
                            harness_state.state.next(&VerificationSignal::Matched);
                    }
                    Err(e) => {
                        harness_state.error = Some(e);
                        harness_state.state = RobotState::Failed;
                    }
                }
            }
            RobotState::Verify => {
                let after_snap = self.world_state.latest(&harness_state.device).await;
                // Use a simple default expected outcome — in production this comes from the plan
                let expected = ExpectedOutcome {
                    predicate: fabric::types::expected_outcome::OutcomePredicate::Equals {
                        path: "mode".into(),
                        value: serde_json::json!("stance"),
                    },
                    freshness_ms: 500,
                    stable_window_ms: 0,
                    timeout_ms: 5000,
                };
                let report = self
                    .verifier
                    .verify(
                        &expected,
                        harness_state.latest_snapshot.as_ref(),
                        after_snap.as_ref(),
                        harness_state.attempt,
                    )
                    .await;
                harness_state.latest_verification = Some(report.clone());

                let remaining_retries = self
                    .config
                    .max_retries
                    .saturating_sub(harness_state.retries_used);
                let remaining_replans = self
                    .config
                    .max_replans
                    .saturating_sub(harness_state.replans_used);

                let signal = match report.decision {
                    fabric::types::outcome_verification::VerificationDecision::Matched => VerificationSignal::Matched,
                    fabric::types::outcome_verification::VerificationDecision::RetryableMismatch => {
                        harness_state.retries_used += 1;
                        VerificationSignal::Retryable { remaining_retries }
                    }
                    fabric::types::outcome_verification::VerificationDecision::ReplannableMismatch => {
                        harness_state.replans_used += 1;
                        VerificationSignal::Replannable { remaining_replans }
                    }
                    fabric::types::outcome_verification::VerificationDecision::Unsafe => VerificationSignal::Unsafe,
                    fabric::types::outcome_verification::VerificationDecision::Unknown => {
                        VerificationSignal::Unknown { remaining_retries }
                    }
                };
                harness_state.state = harness_state.state.next(&signal);
            }
            RobotState::Retry => {
                harness_state.state = harness_state.state.next(&VerificationSignal::Matched);
            }
            RobotState::Replan => {
                match self
                    .planner
                    .replan(
                        &harness_state.device,
                        harness_state.latest_snapshot.as_ref().unwrap(),
                        harness_state.error.as_deref().unwrap_or("unknown"),
                    )
                    .await
                {
                    Ok(req) => {
                        harness_state.latest_skill_request = Some(req);
                        harness_state.state =
                            harness_state.state.next(&VerificationSignal::Matched);
                    }
                    Err(e) => {
                        harness_state.error = Some(e);
                        harness_state.state = RobotState::Failed;
                    }
                }
            }
            RobotState::Recover => {
                harness_state.state = harness_state.state.next(&VerificationSignal::Matched);
            }
            RobotState::Settle => {
                let _ = self
                    .episodes
                    .close_episode(&harness_state.episode_id, "completed")
                    .await;
                harness_state.state = harness_state.state.next(&VerificationSignal::Matched);
            }
            RobotState::SafeStop => {
                let _ = self.executor.safe_stop(&harness_state.device).await;
                let _ = self
                    .episodes
                    .close_episode(&harness_state.episode_id, "failed")
                    .await;
                harness_state.state = harness_state.state.next(&VerificationSignal::Matched);
            }
            RobotState::Completed | RobotState::Failed => {}
        }
        harness_state
    }
}
