//! SQLite adapter for single-step Goal advancement.

use super::ObjectiveStore;
use application::goal::GoalAdvanceRepository;
use application::goal_attempt::GoalAttempt;
use contracts::{GoalId, GoalSnapshot, GoalState};
use std::sync::{Arc, Mutex};

pub struct SqliteGoalAdvanceRepository {
    store: Arc<Mutex<ObjectiveStore>>,
}

impl SqliteGoalAdvanceRepository {
    pub fn new(store: Arc<Mutex<ObjectiveStore>>) -> Self {
        Self { store }
    }
}

impl GoalAdvanceRepository for SqliteGoalAdvanceRepository {
    fn list_candidates(&self) -> Result<Vec<GoalSnapshot>, String> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .list_goals(
                &[GoalState::Running, GoalState::Blocked, GoalState::Ready],
                100,
            )
            .map_err(|error| error.to_string())
    }

    fn transition_ready(&self, goal: &GoalSnapshot) -> Result<GoalSnapshot, String> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .transition_goal(
                goal.id,
                goal.version,
                GoalState::Ready,
                None,
                &serde_json::json!({"action": "scheduler_wake"}),
            )
            .map_err(|error| error.to_string())
    }

    fn get_goal(&self, goal_id: GoalId) -> Result<Option<GoalSnapshot>, String> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_goal(goal_id)
            .map_err(|error| error.to_string())
    }

    fn latest_attempt(&self, goal_id: GoalId) -> Result<Option<GoalAttempt>, String> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .attempts_for_goal(goal_id, 1)
            .map(|attempts| attempts.into_iter().next())
            .map_err(|error| error.to_string())
    }
}
