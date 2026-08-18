//! Consumer-owned durable boundary for one-shot Goal attempt coordination.

use super::{
    AttemptCoordinatorError, GoalBudgetRequest, PersistedCodingJob, PersistedVerificationReport,
};
use crate::goal_attempt::GoalAttempt;
use contracts::{
    AttemptEvidence, AttemptId, CodingJobId, CodingJobReport, CognitiveRole, GoalBudgetUsage,
    GoalId, GoalSnapshot, GoalState, GoalWaitReason, RuntimeFailure, RuntimeId, RuntimeResult,
    VerificationReport,
};
use std::path::Path;

pub struct AttemptContext {
    pub goal: GoalSnapshot,
    pub previous_attempts: Vec<GoalAttempt>,
    pub existing_attempt: Option<GoalAttempt>,
}

pub struct BeginAttemptCommand {
    pub attempt_id: Option<AttemptId>,
    pub goal_id: GoalId,
    pub expected_version: u64,
    pub sequence: u32,
    pub runtime_id: RuntimeId,
    pub role: CognitiveRole,
    pub input: serde_json::Value,
    pub budget: GoalBudgetRequest,
    pub now_ms: i64,
}

pub struct BegunAttempt {
    pub reservation_id: String,
    pub attempt: GoalAttempt,
}

pub trait GoalAttemptPersistencePort: Send + Sync {
    fn context(
        &self,
        goal_id: GoalId,
        expected_version: u64,
        sequence: u32,
    ) -> Result<AttemptContext, AttemptCoordinatorError>;
    fn begin(&self, command: BeginAttemptCommand) -> Result<BegunAttempt, AttemptCoordinatorError>;
    fn finish_and_settle(
        &self,
        attempt_id: AttemptId,
        reservation_id: &str,
        outcome: Result<RuntimeResult, RuntimeFailure>,
    ) -> Result<GoalAttempt, AttemptCoordinatorError>;
    fn settle_budget(
        &self,
        reservation_id: &str,
        usage: GoalBudgetUsage,
    ) -> Result<(), AttemptCoordinatorError>;
    fn attempt(
        &self,
        attempt_id: AttemptId,
    ) -> Result<Option<GoalAttempt>, AttemptCoordinatorError>;
    fn attempt_count(
        &self,
        goal_id: GoalId,
        role: CognitiveRole,
    ) -> Result<u32, AttemptCoordinatorError>;
    fn append_evidence(
        &self,
        attempt_id: AttemptId,
        evidence: &[AttemptEvidence],
    ) -> Result<GoalAttempt, AttemptCoordinatorError>;
    fn load_coding_job(
        &self,
        job_id: CodingJobId,
    ) -> Result<Option<PersistedCodingJob>, AttemptCoordinatorError>;
    fn persist_coding_job(
        &self,
        report: &CodingJobReport,
        worktree_ref: &Path,
        diff: &[u8],
        now_ms: i64,
    ) -> Result<PersistedCodingJob, AttemptCoordinatorError>;
    fn load_verification(
        &self,
        job_id: CodingJobId,
    ) -> Result<Option<PersistedVerificationReport>, AttemptCoordinatorError>;
    fn persist_verification(
        &self,
        report: &VerificationReport,
        now_ms: i64,
    ) -> Result<PersistedVerificationReport, AttemptCoordinatorError>;
    fn goal(&self, goal_id: GoalId) -> Result<Option<GoalSnapshot>, AttemptCoordinatorError>;
    fn transition_latest(
        &self,
        goal_id: GoalId,
        state: GoalState,
        wait_reason: Option<&GoalWaitReason>,
        payload: &serde_json::Value,
    ) -> Result<GoalSnapshot, AttemptCoordinatorError>;
}
