//! Single-step Goal advancement use case.

use super::{
    AttemptCoordinationOutcome, AttemptCoordinatorError, AttemptRequest, GoalCoordinator,
    GoalProgressPort,
};
use crate::goal_attempt::GoalAttempt;
use async_trait::async_trait;
use contracts::{
    AttemptUsage, CognitiveRole, GoalId, GoalSnapshot, GoalState, GoalWaitReason, RuntimeId,
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, thiserror::Error)]
pub enum GoalAdvanceError {
    #[error("goal advancement persistence failed: {0}")]
    Persistence(String),
    #[error(transparent)]
    Attempt(#[from] AttemptCoordinatorError),
}

pub trait GoalAdvanceRepository: Send + Sync {
    fn list_candidates(&self) -> Result<Vec<GoalSnapshot>, String>;
    fn transition_ready(&self, goal: &GoalSnapshot) -> Result<GoalSnapshot, String>;
    fn get_goal(&self, goal_id: GoalId) -> Result<Option<GoalSnapshot>, String>;
    fn latest_attempt(&self, goal_id: GoalId) -> Result<Option<GoalAttempt>, String>;
}

#[async_trait]
pub trait GoalAttemptUseCase: Send + Sync {
    async fn execute_one(
        &self,
        request: AttemptRequest,
        cancel: CancellationToken,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError>;
}

pub struct GoalAdvanceService {
    repository: Arc<dyn GoalAdvanceRepository>,
    coordinator: Arc<GoalCoordinator>,
    attempts: Arc<dyn GoalAttemptUseCase>,
    worker_runtime: RuntimeId,
    reviewer_runtime: RuntimeId,
    progress: Arc<dyn GoalProgressPort>,
}

impl GoalAdvanceService {
    pub fn new(
        repository: Arc<dyn GoalAdvanceRepository>,
        coordinator: Arc<GoalCoordinator>,
        attempts: Arc<dyn GoalAttemptUseCase>,
        worker_runtime: RuntimeId,
        reviewer_runtime: RuntimeId,
        progress: Arc<dyn GoalProgressPort>,
    ) -> Self {
        Self {
            repository,
            coordinator,
            attempts,
            worker_runtime,
            reviewer_runtime,
            progress,
        }
    }

    /// Advances at most one Goal and invokes at most one configured runtime.
    pub async fn advance_once(
        &self,
        now_ms: i64,
        cancel: CancellationToken,
    ) -> Result<bool, GoalAdvanceError> {
        let goal = self
            .repository
            .list_candidates()
            .map_err(GoalAdvanceError::Persistence)?
            .into_iter()
            .find(|goal| match &goal.wait_reason {
                Some(GoalWaitReason::Backoff { until_ms }) => *until_ms <= now_ms,
                Some(GoalWaitReason::ExternalEvent { key }) => key.starts_with("runtime:"),
                Some(GoalWaitReason::HumanInput { .. }) => false,
                None => goal.state != GoalState::Blocked,
            });
        let Some(mut goal) = goal else {
            return Ok(false);
        };

        let mut selected = None;
        match goal.state {
            GoalState::Ready => {
                self.coordinator
                    .tick(goal.id, now_ms)
                    .map_err(|error| GoalAdvanceError::Persistence(error.to_string()))?;
                return Ok(true);
            }
            GoalState::Blocked => {
                selected = match &goal.wait_reason {
                    Some(GoalWaitReason::Backoff { until_ms }) if *until_ms <= now_ms => self
                        .repository
                        .latest_attempt(goal.id)
                        .map_err(GoalAdvanceError::Persistence)?
                        .map(|attempt| (attempt.runtime_id, attempt.role)),
                    Some(GoalWaitReason::ExternalEvent { key }) => key
                        .strip_prefix("runtime:")
                        .map(|id| (RuntimeId(id.to_owned()), CognitiveRole::Reviewer)),
                    _ => return Ok(false),
                };
                goal = self
                    .repository
                    .transition_ready(&goal)
                    .map_err(GoalAdvanceError::Persistence)?;
                self.coordinator
                    .tick(goal.id, now_ms)
                    .map_err(|error| GoalAdvanceError::Persistence(error.to_string()))?;
                goal = self
                    .repository
                    .get_goal(goal.id)
                    .map_err(GoalAdvanceError::Persistence)?
                    .ok_or_else(|| {
                        GoalAdvanceError::Persistence(
                            "goal disappeared after scheduler wake".into(),
                        )
                    })?;
            }
            GoalState::Running => {}
            _ => return Ok(false),
        }

        let latest = self
            .repository
            .latest_attempt(goal.id)
            .map_err(GoalAdvanceError::Persistence)?;
        let (runtime_id, role) = selected
            .or_else(|| {
                latest
                    .as_ref()
                    .map(|attempt| (attempt.runtime_id.clone(), attempt.role))
            })
            .unwrap_or_else(|| (self.worker_runtime.clone(), CognitiveRole::Worker));
        let sequence = latest.map_or(1, |attempt| attempt.sequence.saturating_add(1));
        self.coordinator
            .admit_attempt_storage(goal.id)
            .map_err(GoalAdvanceError::Persistence)?;
        let outcome = self
            .attempts
            .execute_one(
                AttemptRequest {
                    goal_id: goal.id,
                    expected_version: goal.version,
                    sequence,
                    runtime_id,
                    escalation_runtime_id: (role == CognitiveRole::Worker)
                        .then(|| self.reviewer_runtime.clone()),
                    role,
                    task: goal.spec.original_intent.clone(),
                    estimated_usage: AttemptUsage::default(),
                },
                cancel,
            )
            .await;
        let outcome = match outcome {
            Ok(outcome) => outcome,
            Err(error) => {
                self.coordinator.release_attempt_storage(goal.id);
                return Err(error.into());
            }
        };
        let terminal = match &outcome {
            AttemptCoordinationOutcome::Succeeded { goal, .. }
            | AttemptCoordinationOutcome::Failed { goal, .. } => goal.state.is_terminal(),
        };
        if terminal {
            self.coordinator.release_attempt_storage(goal.id);
        }
        self.progress.publish(&outcome).await;
        Ok(true)
    }
}
