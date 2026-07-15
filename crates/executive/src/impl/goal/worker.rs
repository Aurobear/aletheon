//! Bounded daemon scheduler for durable Goals.

use super::{
    AttemptCoordinator, AttemptRequest, GoalCoordinator, ObjectiveStore, RegistryAttemptExecutor,
    RetryPolicy,
};
use crate::core::runtime_registry::RuntimeRegistry;
use crate::r#impl::channel::router::GoalProgress;
use aletheon_kernel::chronos::SystemClock;
use anyhow::Context;
use fabric::{AttemptUsage, Clock, CognitiveRole, GoalState, GoalWaitReason, RuntimeId};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio::sync::mpsc;
use tokio_util::sync::CancellationToken;
use tracing::{info, warn};

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
            .await?;
        let _ = self
            .progress_tx
            .send(GoalProgress::from_outcome(&outcome))
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
