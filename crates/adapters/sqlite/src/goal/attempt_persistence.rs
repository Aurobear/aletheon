//! SQLite implementation of the Application Goal attempt persistence boundary.

use super::ObjectiveStore;
use application::goal::{
    AttemptContext, AttemptCoordinatorError, BeginAttemptCommand, BegunAttempt,
    GoalAttemptPersistencePort, PersistedCodingJob, PersistedVerificationReport,
};
use application::goal_attempt::GoalAttempt;
use contracts::{
    AttemptEvidence, AttemptId, CodingJobId, CodingJobReport, CognitiveRole, FailureClass,
    GoalBudgetUsage, GoalId, GoalSnapshot, GoalState, GoalWaitReason, RuntimeFailure,
    RuntimeResult, VerificationReport,
};
use std::path::Path;
use std::sync::{Arc, Mutex};

pub struct SqliteGoalAttemptPersistence {
    store: Arc<Mutex<ObjectiveStore>>,
}

impl SqliteGoalAttemptPersistence {
    pub fn new(store: Arc<Mutex<ObjectiveStore>>) -> Self {
        Self { store }
    }
}

fn persistence(error: impl std::fmt::Display) -> AttemptCoordinatorError {
    AttemptCoordinatorError::Persistence(error.to_string())
}

fn validate_goal(
    store: &ObjectiveStore,
    goal_id: GoalId,
    expected_version: u64,
) -> Result<GoalSnapshot, AttemptCoordinatorError> {
    let goal = store
        .get_goal(goal_id)
        .map_err(persistence)?
        .ok_or(AttemptCoordinatorError::GoalNotFound(goal_id))?;
    if goal.state != GoalState::Running {
        return Err(AttemptCoordinatorError::GoalNotRunning(goal.state));
    }
    if goal.version != expected_version {
        return Err(AttemptCoordinatorError::VersionConflict {
            expected: expected_version,
            actual: goal.version,
        });
    }
    Ok(goal)
}

impl GoalAttemptPersistencePort for SqliteGoalAttemptPersistence {
    fn context(
        &self,
        goal_id: GoalId,
        expected_version: u64,
        sequence: u32,
    ) -> Result<AttemptContext, AttemptCoordinatorError> {
        let store = self.store.lock().unwrap_or_else(|error| error.into_inner());
        let goal = validate_goal(&store, goal_id, expected_version)?;
        let previous_attempts = store
            .attempts_for_goal(goal_id, usize::MAX)
            .map_err(persistence)?;
        let existing_attempt = previous_attempts
            .iter()
            .find(|attempt| attempt.sequence == sequence)
            .cloned();
        Ok(AttemptContext {
            goal,
            previous_attempts,
            existing_attempt,
        })
    }

    fn begin(&self, command: BeginAttemptCommand) -> Result<BegunAttempt, AttemptCoordinatorError> {
        let store = self.store.lock().unwrap_or_else(|error| error.into_inner());
        validate_goal(&store, command.goal_id, command.expected_version)?;
        let reservation = store
            .reserve_goal_budget(command.goal_id, command.budget, command.now_ms)
            .map_err(|error| AttemptCoordinatorError::Budget(error.to_string()))?;
        let mut input = command.input;
        input["budget_reservation_id"] =
            serde_json::Value::String(reservation.reservation_id.clone());
        let begun = match command.attempt_id {
            Some(attempt_id) => store.begin_attempt_with_id(
                attempt_id,
                command.goal_id,
                command.sequence,
                &command.runtime_id,
                command.role,
                &input,
            ),
            None => store.begin_attempt(
                command.goal_id,
                command.sequence,
                &command.runtime_id,
                command.role,
                &input,
            ),
        };
        match begun {
            Ok(attempt) => Ok(BegunAttempt {
                reservation_id: reservation.reservation_id,
                attempt,
            }),
            Err(error) => {
                if let Err(revoke_error) = store.revoke_goal_budget(&reservation.reservation_id) {
                    return Err(AttemptCoordinatorError::Persistence(format!(
                        "{error}; budget revoke also failed: {revoke_error}"
                    )));
                }
                Err(persistence(error))
            }
        }
    }

    fn finish_and_settle(
        &self,
        attempt_id: AttemptId,
        reservation_id: &str,
        outcome: Result<RuntimeResult, RuntimeFailure>,
    ) -> Result<GoalAttempt, AttemptCoordinatorError> {
        let store = self.store.lock().unwrap_or_else(|error| error.into_inner());
        let persisted = match &outcome {
            Err(failure) if failure.class == FailureClass::Cancelled => {
                store.cancel_attempt(attempt_id, failure.clone())
            }
            _ => store.finish_attempt(attempt_id, outcome),
        };
        let attempt = match persisted {
            Ok(attempt) => attempt,
            Err(error) => {
                if let Err(revoke_error) = store.revoke_goal_budget(reservation_id) {
                    return Err(AttemptCoordinatorError::Persistence(format!(
                        "{error}; budget revoke also failed: {revoke_error}"
                    )));
                }
                return Err(persistence(error));
            }
        };
        store
            .settle_goal_budget(reservation_id, usage_for_budget(&attempt.usage))
            .map_err(|error| AttemptCoordinatorError::Budget(error.to_string()))?;
        Ok(attempt)
    }

    fn settle_budget(
        &self,
        reservation_id: &str,
        usage: GoalBudgetUsage,
    ) -> Result<(), AttemptCoordinatorError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .settle_goal_budget(reservation_id, usage)
            .map_err(|error| AttemptCoordinatorError::Budget(error.to_string()))
    }

    fn attempt(
        &self,
        attempt_id: AttemptId,
    ) -> Result<Option<GoalAttempt>, AttemptCoordinatorError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .attempt(attempt_id)
            .map_err(persistence)
    }

    fn attempt_count(
        &self,
        goal_id: GoalId,
        role: CognitiveRole,
    ) -> Result<u32, AttemptCoordinatorError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .attempts_for_goal(goal_id, usize::MAX)
            .map(|attempts| {
                attempts
                    .into_iter()
                    .filter(|attempt| attempt.role == role)
                    .count() as u32
            })
            .map_err(persistence)
    }

    fn append_evidence(
        &self,
        attempt_id: AttemptId,
        evidence: &[AttemptEvidence],
    ) -> Result<GoalAttempt, AttemptCoordinatorError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .append_attempt_evidence(attempt_id, evidence)
            .map_err(persistence)
    }

    fn load_coding_job(
        &self,
        job_id: CodingJobId,
    ) -> Result<Option<PersistedCodingJob>, AttemptCoordinatorError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .load_coding_job(job_id)
            .map_err(persistence)
    }

    fn persist_coding_job(
        &self,
        report: &CodingJobReport,
        worktree_ref: &Path,
        diff: &[u8],
        now_ms: i64,
    ) -> Result<PersistedCodingJob, AttemptCoordinatorError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .persist_coding_job(report, worktree_ref, diff, now_ms)
            .map_err(persistence)
    }

    fn load_verification(
        &self,
        job_id: CodingJobId,
    ) -> Result<Option<PersistedVerificationReport>, AttemptCoordinatorError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .load_verification_report(job_id)
            .map_err(persistence)
    }

    fn persist_verification(
        &self,
        report: &VerificationReport,
        now_ms: i64,
    ) -> Result<PersistedVerificationReport, AttemptCoordinatorError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .persist_verification_report(report, now_ms)
            .map_err(persistence)
    }

    fn goal(&self, goal_id: GoalId) -> Result<Option<GoalSnapshot>, AttemptCoordinatorError> {
        self.store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_goal(goal_id)
            .map_err(persistence)
    }

    fn transition_latest(
        &self,
        goal_id: GoalId,
        state: GoalState,
        wait_reason: Option<&GoalWaitReason>,
        payload: &serde_json::Value,
    ) -> Result<GoalSnapshot, AttemptCoordinatorError> {
        let store = self.store.lock().unwrap_or_else(|error| error.into_inner());
        let current = store
            .get_goal(goal_id)
            .map_err(persistence)?
            .ok_or(AttemptCoordinatorError::GoalNotFound(goal_id))?;
        store
            .transition_goal(goal_id, current.version, state, wait_reason, payload)
            .map_err(|error| AttemptCoordinatorError::Transition(error.to_string()))
    }
}

fn usage_for_budget(usage: &contracts::AttemptUsage) -> GoalBudgetUsage {
    GoalBudgetUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cost_usd: usage.cost_usd.unwrap_or_default(),
        attempts: 1,
    }
}
