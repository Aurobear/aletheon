//! Application-owned request, outcome, and verification seam for one Goal attempt.

use crate::goal_attempt::GoalAttempt;
use crate::goal_retry::RetryDecision;
use crate::verification::VerificationContext;
use crate::verification::VerificationService;
use async_trait::async_trait;
use contracts::{AttemptUsage, CognitiveRole, GoalId, GoalSnapshot, RuntimeId, VerificationReport};
use std::path::{Path, PathBuf};
use tokio_util::sync::CancellationToken;

/// Resolves a validated relative coding worktree through a host-owned boundary.
pub trait CodingWorktreePort: Send + Sync {
    fn resolve(&self, relative: &Path) -> Result<PathBuf, String>;
}

#[async_trait]
pub trait GoalProgressPort: Send + Sync {
    async fn publish(&self, outcome: &AttemptCoordinationOutcome);
}

#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum AttemptCoordinatorError {
    #[error("goal {0} not found")]
    GoalNotFound(GoalId),
    #[error("goal is not running: {0}")]
    GoalNotRunning(contracts::GoalState),
    #[error("version conflict: expected {expected}, actual {actual}")]
    VersionConflict { expected: u64, actual: u64 },
    #[error("runtime unavailable: {0:?}")]
    RuntimeUnavailable(RuntimeId),
    #[error("goal budget operation failed: {0}")]
    Budget(String),
    #[error("attempt persistence failed: {0}")]
    Persistence(String),
    #[error("goal transition failed: {0}")]
    Transition(String),
}

#[derive(Debug, Clone)]
pub struct AttemptRequest {
    pub goal_id: GoalId,
    pub expected_version: u64,
    pub sequence: u32,
    pub runtime_id: RuntimeId,
    pub escalation_runtime_id: Option<RuntimeId>,
    pub role: CognitiveRole,
    pub task: String,
    pub estimated_usage: AttemptUsage,
}

#[derive(Debug, Clone, PartialEq)]
pub enum AttemptCoordinationOutcome {
    Succeeded {
        attempt: GoalAttempt,
        goal: GoalSnapshot,
    },
    Failed {
        attempt: GoalAttempt,
        decision: RetryDecision,
        goal: GoalSnapshot,
    },
}

#[async_trait]
pub trait CodingVerifier: Send + Sync {
    async fn verify_coding_attempt(
        &self,
        context: &VerificationContext,
        cancel: CancellationToken,
    ) -> Result<VerificationReport, String>;
}

#[async_trait]
impl CodingVerifier for VerificationService {
    async fn verify_coding_attempt(
        &self,
        context: &VerificationContext,
        cancel: CancellationToken,
    ) -> Result<VerificationReport, String> {
        self.verify(context, cancel)
            .await
            .map_err(|error| error.to_string())
    }
}
