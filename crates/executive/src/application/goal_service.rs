//! Request-safe objective and goal use cases.

use std::sync::Arc;

use async_trait::async_trait;
use fabric::{GoalId, GoalSnapshot, GoalSpec, GoalState, Objective, ObjectiveSummary, PrincipalId};
use thiserror::Error;
use tokio::sync::Mutex;

use crate::application::goal::ObjectiveStore;

#[derive(Debug, Clone)]
pub struct LegacyObjectiveDetail {
    pub objective: Objective,
    pub sub_goals: Vec<ObjectiveSummary>,
}

#[derive(Debug, Clone)]
pub struct LegacyResume {
    pub objective: Objective,
    pub sub_goals: Vec<ObjectiveSummary>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalAction {
    Pause,
    Run,
    Cancel,
}

#[derive(Debug, Error, Clone, PartialEq, Eq)]
pub enum GoalServiceError {
    #[error("goal not found")]
    NotFound,
    #[error("invalid goal transition: {0}")]
    InvalidTransition(String),
    #[error("goal version conflict: {0}")]
    Conflict(String),
    #[error("goal store operation failed: {0}")]
    Store(String),
}

#[async_trait]
pub trait GoalUseCases: Send + Sync {
    async fn create_legacy(
        &self,
        description: String,
        session_id: String,
        scope: String,
    ) -> Result<i64, GoalServiceError>;
    async fn show_legacy(&self, id: i64) -> Result<LegacyObjectiveDetail, GoalServiceError>;
    async fn set_legacy_status(&self, id: i64, status: String) -> Result<bool, GoalServiceError>;
    async fn list_legacy(
        &self,
        status: Option<String>,
    ) -> Result<Vec<ObjectiveSummary>, GoalServiceError>;
    async fn resume_legacy(&self) -> Result<Option<LegacyResume>, GoalServiceError>;
    async fn create_goal(
        &self,
        owner: PrincipalId,
        session_id: String,
        scope: String,
        spec: GoalSpec,
    ) -> Result<GoalSnapshot, GoalServiceError>;
    async fn list_goals(&self, limit: usize) -> Result<Vec<GoalSnapshot>, GoalServiceError>;
    async fn act(
        &self,
        id: GoalId,
        action: GoalAction,
        expected_version: Option<u64>,
    ) -> Result<GoalSnapshot, GoalServiceError>;
}

pub struct GoalService {
    store: Arc<Mutex<ObjectiveStore>>,
}

impl GoalService {
    pub fn new(store: Arc<Mutex<ObjectiveStore>>) -> Self {
        Self { store }
    }

    fn store_error(error: anyhow::Error) -> GoalServiceError {
        GoalServiceError::Store(error.to_string())
    }

    fn transition_error(error: impl std::fmt::Display) -> GoalServiceError {
        let message = error.to_string();
        if message.contains("not found") {
            GoalServiceError::NotFound
        } else if message.contains("version conflict") {
            GoalServiceError::Conflict(message)
        } else if message.contains("illegal transition") {
            GoalServiceError::InvalidTransition(message)
        } else {
            GoalServiceError::Store(message)
        }
    }
}

#[async_trait]
impl GoalUseCases for GoalService {
    async fn create_legacy(
        &self,
        description: String,
        session_id: String,
        scope: String,
    ) -> Result<i64, GoalServiceError> {
        self.store
            .lock()
            .await
            .create(&description, None, &session_id, &scope)
            .map_err(Self::store_error)
    }

    async fn show_legacy(&self, id: i64) -> Result<LegacyObjectiveDetail, GoalServiceError> {
        let store = self.store.lock().await;
        let objective = store
            .get(id)
            .map_err(Self::store_error)?
            .ok_or(GoalServiceError::NotFound)?;
        let sub_goals = store
            .sub_goals(id)
            .map_err(Self::store_error)?
            .iter()
            .map(Objective::to_summary)
            .collect();
        Ok(LegacyObjectiveDetail {
            objective,
            sub_goals,
        })
    }

    async fn set_legacy_status(&self, id: i64, status: String) -> Result<bool, GoalServiceError> {
        self.store
            .lock()
            .await
            .set_status(id, &status)
            .map_err(Self::store_error)
    }

    async fn list_legacy(
        &self,
        status: Option<String>,
    ) -> Result<Vec<ObjectiveSummary>, GoalServiceError> {
        self.store
            .lock()
            .await
            .list(status.as_deref(), 50)
            .map(|rows| rows.iter().map(Objective::to_summary).collect())
            .map_err(Self::store_error)
    }

    async fn resume_legacy(&self) -> Result<Option<LegacyResume>, GoalServiceError> {
        self.store
            .lock()
            .await
            .resume()
            .map(|resume| {
                resume.map(|(objective, sub_goals)| LegacyResume {
                    objective,
                    sub_goals: sub_goals.iter().map(Objective::to_summary).collect(),
                })
            })
            .map_err(Self::store_error)
    }

    async fn create_goal(
        &self,
        owner: PrincipalId,
        session_id: String,
        scope: String,
        spec: GoalSpec,
    ) -> Result<GoalSnapshot, GoalServiceError> {
        self.store
            .lock()
            .await
            .create_goal(&owner, &session_id, &scope, &spec)
            .map_err(Self::store_error)
    }

    async fn list_goals(&self, limit: usize) -> Result<Vec<GoalSnapshot>, GoalServiceError> {
        self.store
            .lock()
            .await
            .list_goals(&[], limit.min(100))
            .map_err(Self::store_error)
    }

    async fn act(
        &self,
        id: GoalId,
        action: GoalAction,
        expected_version: Option<u64>,
    ) -> Result<GoalSnapshot, GoalServiceError> {
        let store = self.store.lock().await;
        let current = store
            .get_goal(id)
            .map_err(Self::store_error)?
            .ok_or(GoalServiceError::NotFound)?;
        let next = match action {
            GoalAction::Pause => GoalState::Suspended,
            GoalAction::Run => match current.state {
                GoalState::Suspended | GoalState::Blocked => GoalState::Ready,
                state => {
                    return Err(GoalServiceError::InvalidTransition(format!(
                        "cannot run from state {state}"
                    )))
                }
            },
            GoalAction::Cancel => {
                if current.state.is_terminal() {
                    return Err(GoalServiceError::InvalidTransition(
                        "goal already terminal".into(),
                    ));
                }
                GoalState::Cancelled
            }
        };
        let action_name = match action {
            GoalAction::Pause => "pause",
            GoalAction::Run => "run",
            GoalAction::Cancel => "cancel",
        };
        store
            .transition_goal(
                id,
                expected_version.unwrap_or(current.version),
                next,
                None,
                &serde_json::json!({"action": action_name}),
            )
            .map_err(Self::transition_error)
    }
}
