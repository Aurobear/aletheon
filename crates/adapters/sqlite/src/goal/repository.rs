//! Application Goal repository port backed by the canonical ObjectiveStore.

use super::{GoalTransitionError, ObjectiveStore};
use application::goal::{GoalRepository, GoalRepositoryError};
use application::objective::Objective;
use async_trait::async_trait;
use contracts::{GoalId, GoalSnapshot, GoalSpec, GoalState, GoalWaitReason, PrincipalId};
use std::sync::Arc;
use tokio::sync::Mutex;

#[derive(Clone)]
pub struct SqliteGoalRepository {
    store: Arc<Mutex<ObjectiveStore>>,
}

impl SqliteGoalRepository {
    pub fn new(store: Arc<Mutex<ObjectiveStore>>) -> Self {
        Self { store }
    }
    pub fn store(&self) -> Arc<Mutex<ObjectiveStore>> {
        self.store.clone()
    }
}

fn storage(error: anyhow::Error) -> GoalRepositoryError {
    GoalRepositoryError::Store(error.to_string())
}

fn transition(error: GoalTransitionError) -> GoalRepositoryError {
    match error {
        GoalTransitionError::NotFound(_) => GoalRepositoryError::NotFound,
        GoalTransitionError::Illegal { .. } => {
            GoalRepositoryError::InvalidTransition(error.to_string())
        }
        GoalTransitionError::VersionConflict { .. } => {
            GoalRepositoryError::Conflict(error.to_string())
        }
        GoalTransitionError::Storage(_) => GoalRepositoryError::Store(error.to_string()),
    }
}

#[async_trait]
impl GoalRepository for SqliteGoalRepository {
    async fn create_legacy(
        &self,
        description: &str,
        session_id: &str,
        scope: &str,
    ) -> Result<i64, GoalRepositoryError> {
        self.store
            .lock()
            .await
            .create(description, None, session_id, scope)
            .map_err(storage)
    }
    async fn get_legacy(&self, id: i64) -> Result<Option<Objective>, GoalRepositoryError> {
        self.store.lock().await.get(id).map_err(storage)
    }
    async fn sub_goals(&self, parent: i64) -> Result<Vec<Objective>, GoalRepositoryError> {
        self.store.lock().await.sub_goals(parent).map_err(storage)
    }
    async fn set_legacy_status(&self, id: i64, status: &str) -> Result<bool, GoalRepositoryError> {
        self.store
            .lock()
            .await
            .set_status(id, status)
            .map_err(storage)
    }
    async fn list_legacy(
        &self,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Objective>, GoalRepositoryError> {
        self.store.lock().await.list(status, limit).map_err(storage)
    }
    async fn resume_legacy(
        &self,
    ) -> Result<Option<(Objective, Vec<Objective>)>, GoalRepositoryError> {
        self.store.lock().await.resume().map_err(storage)
    }
    async fn create_goal(
        &self,
        owner: &PrincipalId,
        session_id: &str,
        scope: &str,
        spec: &GoalSpec,
    ) -> Result<GoalSnapshot, GoalRepositoryError> {
        self.store
            .lock()
            .await
            .create_goal(owner, session_id, scope, spec)
            .map_err(storage)
    }
    async fn get_goal(&self, id: GoalId) -> Result<Option<GoalSnapshot>, GoalRepositoryError> {
        self.store.lock().await.get_goal(id).map_err(storage)
    }
    async fn list_goals(
        &self,
        states: &[GoalState],
        limit: usize,
    ) -> Result<Vec<GoalSnapshot>, GoalRepositoryError> {
        self.store
            .lock()
            .await
            .list_goals(states, limit)
            .map_err(storage)
    }
    async fn transition_goal(
        &self,
        id: GoalId,
        expected_version: u64,
        next: GoalState,
        wait_reason: Option<&GoalWaitReason>,
        event_payload: &serde_json::Value,
    ) -> Result<GoalSnapshot, GoalRepositoryError> {
        self.store
            .lock()
            .await
            .transition_goal(id, expected_version, next, wait_reason, event_payload)
            .map_err(transition)
    }
}
