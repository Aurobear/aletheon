//! Bounded Goal policy: one durable state advance per explicit call.

use super::{ExternalEventWaitCondition, GoalExternalEvent, GoalStorageAdmissionPort};
use contracts::{GoalId, GoalSnapshot, GoalState, GoalWaitReason, PrincipalId, ProcessId};
use std::sync::Arc;

#[derive(Debug, Clone, PartialEq)]
pub enum GoalTickOutcome {
    Noop { state: GoalState },
    Transitioned { from: GoalState, to: GoalState },
    TurnRequested { goal_id: GoalId, input: String },
    BudgetBlocked { reason: String },
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GoalCoordinationError {
    #[error("goal coordination repository failed: {0}")]
    Persistence(String),
}

/// Synchronous durable operations needed by bounded Goal coordination.
///
/// Implementations retain transaction and optimistic-lock details; policy
/// remains in [`GoalCoordinator`].
pub trait GoalCoordinatorRepository: Send + Sync {
    fn get_goal(&self, goal_id: GoalId) -> Result<Option<GoalSnapshot>, GoalCoordinationError>;
    fn list_goals(
        &self,
        states: &[GoalState],
        limit: usize,
    ) -> Result<Vec<GoalSnapshot>, GoalCoordinationError>;
    fn transition_goal(
        &self,
        goal_id: GoalId,
        expected_version: u64,
        next: GoalState,
        wait_reason: Option<&GoalWaitReason>,
        event_payload: &serde_json::Value,
    ) -> Result<GoalSnapshot, GoalCoordinationError>;
    fn reserve_attempt_budget(
        &self,
        goal_id: GoalId,
        now_ms: i64,
    ) -> Result<(), GoalCoordinationError>;
    fn set_process_link(
        &self,
        goal_id: GoalId,
        expected_version: u64,
        process_id: Option<ProcessId>,
    ) -> Result<GoalSnapshot, GoalCoordinationError>;
}

pub struct GoalCoordinator {
    repository: Arc<dyn GoalCoordinatorRepository>,
    storage_admission: Option<Arc<dyn GoalStorageAdmissionPort>>,
}

impl GoalCoordinator {
    pub fn new(repository: Arc<dyn GoalCoordinatorRepository>) -> Self {
        Self {
            repository,
            storage_admission: None,
        }
    }

    pub fn with_storage_admission(mut self, admission: Arc<dyn GoalStorageAdmissionPort>) -> Self {
        self.storage_admission = Some(admission);
        self
    }

    pub fn admit_attempt_storage(&self, goal_id: GoalId) -> Result<(), String> {
        let Some(admission) = &self.storage_admission else {
            return Ok(());
        };
        admission.admit(goal_id).map_err(|error| error.to_string())
    }

    pub fn release_attempt_storage(&self, goal_id: GoalId) {
        if let Some(admission) = &self.storage_admission {
            admission.release(goal_id);
        }
    }

    pub fn tick(
        &self,
        goal_id: GoalId,
        now_ms: i64,
    ) -> Result<GoalTickOutcome, GoalCoordinationError> {
        let current = match self.repository.get_goal(goal_id)? {
            Some(goal) => goal,
            None => {
                return Ok(GoalTickOutcome::Noop {
                    state: GoalState::Cancelled,
                });
            }
        };

        match current.state {
            GoalState::Completed | GoalState::Failed | GoalState::Cancelled => {
                self.release_attempt_storage(goal_id);
                Ok(GoalTickOutcome::Noop {
                    state: current.state,
                })
            }
            GoalState::Draft
            | GoalState::Suspended
            | GoalState::AwaitingHuman
            | GoalState::Blocked => Ok(GoalTickOutcome::Noop {
                state: current.state,
            }),
            GoalState::Ready => {
                let snapshot = self.repository.transition_goal(
                    goal_id,
                    current.version,
                    GoalState::Running,
                    None,
                    &serde_json::json!({"action": "tick_start"}),
                )?;
                Ok(GoalTickOutcome::Transitioned {
                    from: current.state,
                    to: snapshot.state,
                })
            }
            GoalState::Running => {
                if let Err(reason) = self.admit_attempt_storage(goal_id) {
                    return Ok(GoalTickOutcome::BudgetBlocked { reason });
                }
                match self.repository.reserve_attempt_budget(goal_id, now_ms) {
                    Ok(()) => Ok(GoalTickOutcome::TurnRequested {
                        goal_id,
                        input: format!("Goal #{}: {}", goal_id.0, current.spec.original_intent),
                    }),
                    Err(error) => Ok(GoalTickOutcome::BudgetBlocked {
                        reason: error.to_string(),
                    }),
                }
            }
        }
    }

    pub fn wake_for_external_event(
        &self,
        principal: &PrincipalId,
        event: &GoalExternalEvent,
    ) -> Result<Vec<GoalId>, GoalCoordinationError> {
        let goals = self.repository.list_goals(
            &[
                GoalState::AwaitingHuman,
                GoalState::Suspended,
                GoalState::Blocked,
            ],
            100,
        )?;
        let mut woken = Vec::new();
        for goal in goals {
            if &goal.owner != principal || goal.state.is_terminal() {
                continue;
            }
            let Some(GoalWaitReason::ExternalEvent { key }) = &goal.wait_reason else {
                continue;
            };
            let Ok(condition) = serde_json::from_str::<ExternalEventWaitCondition>(key) else {
                continue;
            };
            if !condition.matches(event) {
                continue;
            }
            self.repository.transition_goal(
                goal.id,
                goal.version,
                GoalState::Ready,
                None,
                &serde_json::json!({
                    "action": "external_event_wake",
                    "event_id": event.event_id,
                    "object_id": event.object_id,
                }),
            )?;
            woken.push(goal.id);
        }
        Ok(woken)
    }

    pub fn set_process_link(
        &self,
        goal_id: GoalId,
        expected_version: u64,
        process_id: Option<ProcessId>,
    ) -> Result<GoalSnapshot, GoalCoordinationError> {
        self.repository
            .set_process_link(goal_id, expected_version, process_id)
    }
}
