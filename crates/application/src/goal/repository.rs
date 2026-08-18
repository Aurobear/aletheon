//! Consumer-owned durable Goal repository port.

use crate::objective::{Objective, ObjectiveSummary};
use async_trait::async_trait;
use contracts::{GoalId, GoalSnapshot, GoalSpec, GoalState, GoalWaitReason, PrincipalId};

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum GoalRepositoryError {
    #[error("goal not found")]
    NotFound,
    #[error("invalid goal transition: {0}")]
    InvalidTransition(String),
    #[error("goal version conflict: {0}")]
    Conflict(String),
    #[error("goal repository operation failed: {0}")]
    Store(String),
}

#[async_trait]
pub trait GoalRepository: Send + Sync {
    async fn create_legacy(
        &self,
        description: &str,
        session_id: &str,
        scope: &str,
    ) -> Result<i64, GoalRepositoryError>;
    async fn get_legacy(&self, id: i64) -> Result<Option<Objective>, GoalRepositoryError>;
    async fn sub_goals(&self, parent: i64) -> Result<Vec<Objective>, GoalRepositoryError>;
    async fn set_legacy_status(&self, id: i64, status: &str) -> Result<bool, GoalRepositoryError>;
    async fn list_legacy(
        &self,
        status: Option<&str>,
        limit: usize,
    ) -> Result<Vec<Objective>, GoalRepositoryError>;
    async fn resume_legacy(
        &self,
    ) -> Result<Option<(Objective, Vec<Objective>)>, GoalRepositoryError>;
    async fn create_goal(
        &self,
        owner: &PrincipalId,
        session_id: &str,
        scope: &str,
        spec: &GoalSpec,
    ) -> Result<GoalSnapshot, GoalRepositoryError>;
    async fn get_goal(&self, id: GoalId) -> Result<Option<GoalSnapshot>, GoalRepositoryError>;
    async fn list_goals(
        &self,
        states: &[GoalState],
        limit: usize,
    ) -> Result<Vec<GoalSnapshot>, GoalRepositoryError>;
    async fn transition_goal(
        &self,
        id: GoalId,
        expected_version: u64,
        next: GoalState,
        wait_reason: Option<&GoalWaitReason>,
        event_payload: &serde_json::Value,
    ) -> Result<GoalSnapshot, GoalRepositoryError>;
}

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
