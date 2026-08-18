//! Request-safe Goal command/query use cases.

use super::repository::{GoalRepository, GoalRepositoryError, LegacyObjectiveDetail, LegacyResume};
use crate::objective::{Objective, ObjectiveSummary};
use async_trait::async_trait;
use contracts::{GoalId, GoalSnapshot, GoalSpec, GoalState, PrincipalId};
use std::sync::Arc;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalAction {
    Pause,
    Run,
    Cancel,
}

pub type GoalServiceError = GoalRepositoryError;

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
    repository: Arc<dyn GoalRepository>,
}

impl GoalService {
    pub fn new(repository: Arc<dyn GoalRepository>) -> Self {
        Self { repository }
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
        self.repository
            .create_legacy(&description, &session_id, &scope)
            .await
    }

    async fn show_legacy(&self, id: i64) -> Result<LegacyObjectiveDetail, GoalServiceError> {
        let objective = self
            .repository
            .get_legacy(id)
            .await?
            .ok_or(GoalServiceError::NotFound)?;
        let sub_goals = self
            .repository
            .sub_goals(id)
            .await?
            .iter()
            .map(Objective::to_summary)
            .collect();
        Ok(LegacyObjectiveDetail {
            objective,
            sub_goals,
        })
    }

    async fn set_legacy_status(&self, id: i64, status: String) -> Result<bool, GoalServiceError> {
        self.repository.set_legacy_status(id, &status).await
    }

    async fn list_legacy(
        &self,
        status: Option<String>,
    ) -> Result<Vec<ObjectiveSummary>, GoalServiceError> {
        Ok(self
            .repository
            .list_legacy(status.as_deref(), 50)
            .await?
            .iter()
            .map(Objective::to_summary)
            .collect())
    }

    async fn resume_legacy(&self) -> Result<Option<LegacyResume>, GoalServiceError> {
        Ok(self
            .repository
            .resume_legacy()
            .await?
            .map(|(objective, sub_goals)| LegacyResume {
                objective,
                sub_goals: sub_goals.iter().map(Objective::to_summary).collect(),
            }))
    }

    async fn create_goal(
        &self,
        owner: PrincipalId,
        session_id: String,
        scope: String,
        spec: GoalSpec,
    ) -> Result<GoalSnapshot, GoalServiceError> {
        self.repository
            .create_goal(&owner, &session_id, &scope, &spec)
            .await
    }

    async fn list_goals(&self, limit: usize) -> Result<Vec<GoalSnapshot>, GoalServiceError> {
        self.repository.list_goals(&[], limit.min(100)).await
    }

    async fn act(
        &self,
        id: GoalId,
        action: GoalAction,
        expected_version: Option<u64>,
    ) -> Result<GoalSnapshot, GoalServiceError> {
        let current = self
            .repository
            .get_goal(id)
            .await?
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
        self.repository
            .transition_goal(
                id,
                expected_version.unwrap_or(current.version),
                next,
                None,
                &serde_json::json!({"action": action_name}),
            )
            .await
    }
}
