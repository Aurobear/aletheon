//! SQLite adapter for bounded Application Goal coordination.

use super::{GoalBudgetRequest, ObjectiveStore};
use application::goal::{GoalCoordinationError, GoalCoordinatorRepository};
use contracts::{GoalId, GoalSnapshot, GoalState, GoalWaitReason, ProcessId};
use std::sync::{Arc, Mutex};

#[derive(Clone)]
pub struct SqliteGoalCoordinatorRepository {
    store: Arc<Mutex<ObjectiveStore>>,
}

impl SqliteGoalCoordinatorRepository {
    pub fn new(store: Arc<Mutex<ObjectiveStore>>) -> Self {
        Self { store }
    }
}

fn repository(error: impl std::fmt::Display) -> GoalCoordinationError {
    GoalCoordinationError::Persistence(error.to_string())
}

impl GoalCoordinatorRepository for SqliteGoalCoordinatorRepository {
    fn get_goal(&self, goal_id: GoalId) -> Result<Option<GoalSnapshot>, GoalCoordinationError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_goal(goal_id)
            .map_err(repository)
    }

    fn list_goals(
        &self,
        states: &[GoalState],
        limit: usize,
    ) -> Result<Vec<GoalSnapshot>, GoalCoordinationError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .list_goals(states, limit)
            .map_err(repository)
    }

    fn transition_goal(
        &self,
        goal_id: GoalId,
        expected_version: u64,
        next: GoalState,
        wait_reason: Option<&GoalWaitReason>,
        event_payload: &serde_json::Value,
    ) -> Result<GoalSnapshot, GoalCoordinationError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .transition_goal(goal_id, expected_version, next, wait_reason, event_payload)
            .map_err(repository)
    }

    fn reserve_attempt_budget(
        &self,
        goal_id: GoalId,
        now_ms: i64,
    ) -> Result<(), GoalCoordinationError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .reserve_goal_budget(
                goal_id,
                GoalBudgetRequest {
                    input_tokens: 0,
                    output_tokens: 0,
                    cost_usd: 0.0,
                    attempts: 1,
                },
                now_ms,
            )
            .map(|_| ())
            .map_err(repository)
    }

    fn set_process_link(
        &self,
        goal_id: GoalId,
        expected_version: u64,
        process_id: Option<ProcessId>,
    ) -> Result<GoalSnapshot, GoalCoordinationError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .set_process_link(goal_id, expected_version, process_id)
            .map_err(repository)
    }
}
