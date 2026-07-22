//! Bounded daemon scheduler for durable Goals.

use super::{
    AttemptCoordinationOutcome, AttemptCoordinator, AttemptRequest, GoalCoordinator,
    ObjectiveStore, RegistryAttemptExecutor, RetryDecision, RetryPolicy,
};
use crate::core::runtime_registry::RuntimeRegistry;
use crate::application::storage_quota::StorageQuota;
use anyhow::Context;
use fabric::{AttemptUsage, Clock, CognitiveRole, GoalState, GoalWaitReason, RuntimeId};
use gateway::dispatcher::{GoalProgress, GoalProgressKind};
use kernel::chronos::SystemClock;
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

/// Convert an executive-side [`AttemptCoordinationOutcome`] into the
/// gateway-neutral [`GoalProgress`] notification. This is the sole point
/// where the neutral channel engine learns about Goal attempt outcomes,
/// keeping `gateway` free of executive's Goal types.
pub fn goal_progress_from_outcome(outcome: &AttemptCoordinationOutcome) -> GoalProgress {
    match outcome {
        AttemptCoordinationOutcome::Succeeded { attempt, .. } => GoalProgress {
            goal_id: attempt.goal_id,
            attempt_id: attempt.id,
            kind: GoalProgressKind::Succeeded,
        },
        AttemptCoordinationOutcome::Failed {
            attempt, decision, ..
        } => GoalProgress {
            goal_id: attempt.goal_id,
            attempt_id: attempt.id,
            kind: match decision {
                RetryDecision::RetrySame { .. } => GoalProgressKind::RetryBackoff,
                RetryDecision::Escalate { .. } => GoalProgressKind::Escalated,
                RetryDecision::AwaitHuman { .. } => GoalProgressKind::AwaitingHuman,
                RetryDecision::Fail { .. } => GoalProgressKind::Failed,
                RetryDecision::Cancel => GoalProgressKind::Cancelled,
            },
        },
    }
}

/// Runs at most one provider attempt per scheduler cycle.
pub struct GoalWorker {
    store: Arc<Mutex<ObjectiveStore>>,
    coordinator: GoalCoordinator,
    attempts: AttemptCoordinator,
    worker_runtime: RuntimeId,
    reviewer_runtime: RuntimeId,
    clock: Arc<dyn Clock>,
    progress_tx: mpsc::Sender<GoalProgress>,
}

impl GoalWorker {
    pub fn new(
        store: Arc<Mutex<ObjectiveStore>>,
        registry: Arc<RuntimeRegistry>,
        worker_runtime: RuntimeId,
        reviewer_runtime: RuntimeId,
        progress_tx: mpsc::Sender<GoalProgress>,
    ) -> Self {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::new());
        let coordinator = GoalCoordinator::new(store.clone());
        let attempts = AttemptCoordinator::new(
            store.clone(),
            Arc::new(RegistryAttemptExecutor::new(registry)),
            clock.clone(),
            RetryPolicy::default(),
        );
        Self {
            store,
            coordinator,
            attempts,
            worker_runtime,
            reviewer_runtime,
            clock,
            progress_tx,
        }
    }

    /// Apply the same production admission boundary used by other Goal entry points.
    pub fn with_storage_quota(mut self, quota: StorageQuota, expected_attempt_bytes: u64) -> Self {
        self.coordinator = self
            .coordinator
            .with_storage_quota(quota, expected_attempt_bytes);
        self
    }

    /// Advance one Goal and invoke no more than one configured runtime.
    pub async fn tick_once(&self, cancel: CancellationToken) -> anyhow::Result<bool> {
        let now_ms = self.clock.wall_now().0;
        let goal = self
            .store
            .lock()
            .unwrap()
            .list_goals(
                &[GoalState::Running, GoalState::Blocked, GoalState::Ready],
                100,
            )?
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
                self.coordinator.tick(goal.id, self.clock.wall_now().0)?;
                return Ok(true);
            }
            GoalState::Blocked => {
                selected = match &goal.wait_reason {
                    Some(GoalWaitReason::Backoff { until_ms })
                        if *until_ms <= self.clock.wall_now().0 =>
                    {
                        self.latest_route(goal.id)?
                    }
                    Some(GoalWaitReason::ExternalEvent { key }) => key
                        .strip_prefix("runtime:")
                        .map(|id| (RuntimeId(id.to_owned()), CognitiveRole::Reviewer)),
                    _ => return Ok(false),
                };
                goal = self.store.lock().unwrap().transition_goal(
                    goal.id,
                    goal.version,
                    GoalState::Ready,
                    None,
                    &serde_json::json!({"action": "scheduler_wake"}),
                )?;
                self.coordinator.tick(goal.id, self.clock.wall_now().0)?;
                goal = self
                    .store
                    .lock()
                    .unwrap()
                    .get_goal(goal.id)?
                    .context("goal disappeared after scheduler wake")?;
            }
            GoalState::Running => {}
            _ => return Ok(false),
        }

        let (runtime_id, role) = selected
            .or(self.latest_route(goal.id)?)
            .unwrap_or_else(|| (self.worker_runtime.clone(), CognitiveRole::Worker));
        let sequence = self
            .store
            .lock()
            .unwrap()
            .attempts_for_goal(goal.id, 1)?
            .first()
            .map_or(1, |attempt| attempt.sequence.saturating_add(1));
        self.coordinator
            .admit_attempt_storage(goal.id)
            .map_err(anyhow::Error::msg)?;
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
            super::AttemptCoordinationOutcome::Succeeded { goal, .. }
            | super::AttemptCoordinationOutcome::Failed { goal, .. } => goal.state.is_terminal(),
        };
        if terminal {
            self.coordinator.release_attempt_storage(goal.id);
        }
        let _ = self
            .progress_tx
            .send(goal_progress_from_outcome(&outcome))
            .await;
        Ok(true)
    }

    fn latest_route(
        &self,
        goal_id: fabric::GoalId,
    ) -> anyhow::Result<Option<(RuntimeId, CognitiveRole)>> {
        Ok(self
            .store
            .lock()
            .unwrap()
            .attempts_for_goal(goal_id, 1)?
            .first()
            .map(|attempt| (attempt.runtime_id.clone(), attempt.role)))
    }

    pub async fn run(self, cancel: CancellationToken) {
        let mut interval = tokio::time::interval(Duration::from_secs(1));
        interval.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        loop {
            tokio::select! {
                _ = cancel.cancelled() => {
                    info!("Goal worker stopped");
                    break;
                }
                _ = interval.tick() => {
                    if let Err(error) = self.tick_once(cancel.child_token()).await {
                        warn!(error = %error, "Goal worker tick failed");
                    }
                }
            }
        }
    }
}
