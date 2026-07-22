//! One-shot durable Goal attempt coordination.
//!
//! A call performs exactly one runtime invocation. Any retry or escalation is
//! represented as durable Goal state for a later scheduler tick.

use super::budget::{GoalBudgetError, GoalBudgetRequest};
use super::transition::GoalTransitionError;
use super::{GoalAttempt, GoalFrame, ObjectiveStore, RetryDecision, RetryPolicy};
use crate::application::verification::{
    CapabilityAuditSummary, VerificationContext, VerificationSelection, VerificationService,
};
use crate::core::runtime_registry::RuntimeRegistry;
use crate::r#impl::approval::{ApprovalCreate, ApprovalRepository};
use crate::application::coding_runtime::CodingAttemptRequest;
use async_trait::async_trait;
use base64::Engine;
use fabric::{
    ApprovalArtifactRef, ApprovalCategory, ApprovalRisk, ApprovalSubject, AttemptId, AttemptStatus,
    AttemptUsage, Clock, CodingJobReport, CognitiveRole, FailureClass, GoalBudgetUsage, GoalId,
    GoalSnapshot, GoalState, GoalWaitReason, RuntimeFailure, RuntimeId, RuntimeResult,
    VerificationReport,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::fmt;
use std::path::{Component, Path, PathBuf};
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

/// Immutable input for exactly one runtime invocation.
#[derive(Debug, Clone)]
pub struct AttemptRequest {
    pub goal_id: GoalId,
    pub expected_version: u64,
    pub sequence: u32,
    pub runtime_id: RuntimeId,
    pub escalation_runtime_id: Option<RuntimeId>,
    pub role: CognitiveRole,
    pub task: String,
    /// Admission estimate; actual metering replaces it during settlement.
    pub estimated_usage: AttemptUsage,
}

/// Result of one coordinator call. It never contains a second invocation.
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

#[derive(Debug)]
pub enum AttemptCoordinatorError {
    GoalNotFound(GoalId),
    GoalNotRunning(GoalState),
    VersionConflict { expected: u64, actual: u64 },
    RuntimeUnavailable(RuntimeId),
    Budget(GoalBudgetError),
    Persistence(String),
    Transition(GoalTransitionError),
}

impl fmt::Display for AttemptCoordinatorError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::GoalNotFound(id) => write!(f, "goal {id} not found"),
            Self::GoalNotRunning(state) => write!(f, "goal is not running: {state}"),
            Self::VersionConflict { expected, actual } => {
                write!(f, "version conflict: expected {expected}, actual {actual}")
            }
            Self::RuntimeUnavailable(id) => write!(f, "runtime unavailable: {}", id.0),
            Self::Budget(error) => write!(f, "budget error: {error}"),
            Self::Persistence(error) => write!(f, "attempt persistence error: {error}"),
            Self::Transition(error) => write!(f, "goal transition error: {error}"),
        }
    }
}

impl std::error::Error for AttemptCoordinatorError {}

/// Boundary used to resolve and invoke a configured runtime once.
#[async_trait]
pub trait AttemptExecutor: Send + Sync {
    /// Must not create a process or invoke a provider.
    fn is_available(&self, runtime_id: &RuntimeId) -> bool;

    async fn run_once(
        &self,
        runtime_id: &RuntimeId,
        task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure>;
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

#[derive(Clone)]
struct CodingVerification {
    verifier: Arc<dyn CodingVerifier>,
    worktree_base: PathBuf,
    approvals: Option<Arc<Mutex<ApprovalRepository>>>,
}

/// Production executor backed by the configured RuntimeRegistry.
pub struct RegistryAttemptExecutor {
    registry: Arc<RuntimeRegistry>,
}

impl RegistryAttemptExecutor {
    pub fn new(registry: Arc<RuntimeRegistry>) -> Self {
        Self { registry }
    }
}

#[async_trait]
impl AttemptExecutor for RegistryAttemptExecutor {
    fn is_available(&self, runtime_id: &RuntimeId) -> bool {
        self.registry.contains(runtime_id)
    }

    async fn run_once(
        &self,
        runtime_id: &RuntimeId,
        task: &str,
        cancel: CancellationToken,
    ) -> Result<RuntimeResult, RuntimeFailure> {
        let runtime = self
            .registry
            .resolve(runtime_id)
            .map_err(|error| RuntimeFailure {
                class: FailureClass::MissingDependency,
                message: error.to_string(),
                retryable: false,
                usage: AttemptUsage::default(),
                evidence: vec![],
            })?;
        runtime.run_attempt(task, cancel).await
    }
}

pub struct AttemptCoordinator {
    store: Arc<Mutex<ObjectiveStore>>,
    executor: Arc<dyn AttemptExecutor>,
    clock: Arc<dyn Clock>,
    retry_policy: RetryPolicy,
    coding_verification: Option<CodingVerification>,
}

impl AttemptCoordinator {
    pub fn new(
        store: Arc<Mutex<ObjectiveStore>>,
        executor: Arc<dyn AttemptExecutor>,
        clock: Arc<dyn Clock>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            store,
            executor,
            clock,
            retry_policy,
            coding_verification: None,
        }
    }

    pub fn with_coding_verification(
        mut self,
        verifier: Arc<dyn CodingVerifier>,
        worktree_base: impl AsRef<Path>,
    ) -> Result<Self, AttemptCoordinatorError> {
        let base = worktree_base
            .as_ref()
            .canonicalize()
            .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?;
        if !base.is_dir() {
            return Err(AttemptCoordinatorError::Persistence(
                "coding worktree base is not a directory".into(),
            ));
        }
        self.coding_verification = Some(CodingVerification {
            verifier,
            worktree_base: base,
            approvals: None,
        });
        Ok(self)
    }

    pub fn with_approval_repository(
        mut self,
        approvals: Arc<Mutex<ApprovalRepository>>,
    ) -> Result<Self, AttemptCoordinatorError> {
        let coding = self.coding_verification.as_mut().ok_or_else(|| {
            AttemptCoordinatorError::Persistence(
                "coding verification must be configured before approvals".into(),
            )
        })?;
        coding.approvals = Some(approvals);
        Ok(self)
    }

    /// Execute exactly one durable attempt and persist the next Goal state.
    pub async fn execute_one(
        &self,
        request: AttemptRequest,
        cancel: CancellationToken,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError> {
        let request_value = serde_json::from_str::<serde_json::Value>(&request.task).ok();
        let is_coding = request_value.as_ref().is_some_and(|value| {
            value.get("job").is_some() && value.get("task_input").is_some()
        });
        let mut coding_request = if is_coding {
            Some(
                serde_json::from_str::<CodingAttemptRequest>(&request.task).map_err(|error| {
                    AttemptCoordinatorError::Persistence(format!(
                        "invalid coding attempt request: {error}"
                    ))
                })?,
            )
        } else {
            None
        };
        if is_coding && self.coding_verification.is_none() {
            return Err(AttemptCoordinatorError::Persistence(
                "coding verification lifecycle is not configured".into(),
            ));
        }

        if let Some(coding_request) = coding_request.as_ref() {
            if let Some(outcome) = self
                .resume_coding_attempt(&request, coding_request, cancel.clone())
                .await?
            {
                return Ok(outcome);
            }
        }

        // Settlement happens after terminal attempt persistence so runtime
        // evidence survives a transient ledger error. A repeated request must
        // recover that exact attempt rather than invoke the runtime again.
        let existing_attempt = {
            let store = self.store.lock().unwrap();
            let goal = store
                .get_goal(request.goal_id)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
                .ok_or(AttemptCoordinatorError::GoalNotFound(request.goal_id))?;
            if goal.state != GoalState::Running {
                return Err(AttemptCoordinatorError::GoalNotRunning(goal.state));
            }
            if goal.version != request.expected_version {
                return Err(AttemptCoordinatorError::VersionConflict {
                    expected: request.expected_version,
                    actual: goal.version,
                });
            }
            store
                .attempts_for_goal(request.goal_id, usize::MAX)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
                .into_iter()
                .find(|attempt| attempt.sequence == request.sequence)
        };
        if let Some(attempt) = existing_attempt {
            return self.recover_settlement(&request, attempt, cancel).await;
        }

        // Resolve before budget reservation or attempt creation.
        if !self.executor.is_available(&request.runtime_id) {
            return Err(AttemptCoordinatorError::RuntimeUnavailable(
                request.runtime_id,
            ));
        }

        let reservation_id;
        let running_attempt;
        let rendered_task;
        {
            let store = self.store.lock().unwrap();
            let goal = store
                .get_goal(request.goal_id)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
                .ok_or(AttemptCoordinatorError::GoalNotFound(request.goal_id))?;
            if goal.state != GoalState::Running {
                return Err(AttemptCoordinatorError::GoalNotRunning(goal.state));
            }
            if goal.version != request.expected_version {
                return Err(AttemptCoordinatorError::VersionConflict {
                    expected: request.expected_version,
                    actual: goal.version,
                });
            }

            let previous_attempts = store
                .attempts_for_goal(request.goal_id, usize::MAX)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?;
            let frame_task = coding_request
                .as_ref()
                .map(|coding| coding.task_input.as_str())
                .unwrap_or(request.task.as_str());
            let frame = GoalFrame::build(&goal, &previous_attempts, frame_task);
            rendered_task = if let Some(coding) = coding_request.as_mut() {
                coding.job.goal_id = request.goal_id;
                coding.job.attempt_id = AttemptId::new();
                coding.task_input = frame.render();
                serde_json::to_string(coding)
                    .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
            } else {
                frame.render()
            };

            let reservation = store
                .reserve_goal_budget(
                    request.goal_id,
                    GoalBudgetRequest {
                        input_tokens: request.estimated_usage.input_tokens,
                        output_tokens: request.estimated_usage.output_tokens,
                        cost_usd: request.estimated_usage.cost_usd.unwrap_or_default(),
                        attempts: 1,
                    },
                    self.clock.wall_now().0,
                )
                .map_err(AttemptCoordinatorError::Budget)?;
            reservation_id = reservation.reservation_id;

            let input = serde_json::json!({
                "task": request.task,
                "goal_version": request.expected_version,
                "goal_frame": frame,
                "runtime_request": coding_request,
                "budget_reservation_id": reservation_id.clone(),
            });
            let begun = if let Some(coding) = coding_request.as_ref() {
                store.begin_attempt_with_id(
                    coding.job.attempt_id,
                    request.goal_id,
                    request.sequence,
                    &request.runtime_id,
                    request.role,
                    &input,
                )
            } else {
                store.begin_attempt(
                    request.goal_id,
                    request.sequence,
                    &request.runtime_id,
                    request.role,
                    &input,
                )
            };
            match begun {
                Ok(attempt) => running_attempt = attempt,
                Err(error) => {
                    if let Err(revoke_error) = store.revoke_goal_budget(&reservation_id) {
                        return Err(AttemptCoordinatorError::Persistence(format!(
                            "{error}; budget revoke also failed: {revoke_error}"
                        )));
                    }
                    return Err(AttemptCoordinatorError::Persistence(error.to_string()));
                }
            }
        }

        let runtime_outcome = tokio::select! {
            outcome = self.executor.run_once(&request.runtime_id, &rendered_task, cancel.clone()) => outcome,
            _ = cancel.cancelled() => Err(cancelled_failure()),
        };

        let terminal_attempt = {
            let store = self.store.lock().unwrap();
            let persisted = match &runtime_outcome {
                Err(failure) if failure.class == FailureClass::Cancelled => {
                    store.cancel_attempt(running_attempt.id, failure.clone())
                }
                _ => store.finish_attempt(running_attempt.id, runtime_outcome.clone()),
            };
            let attempt = match persisted {
                Ok(attempt) => attempt,
                Err(error) => {
                    if let Err(revoke_error) = store.revoke_goal_budget(&reservation_id) {
                        return Err(AttemptCoordinatorError::Persistence(format!(
                            "{error}; budget revoke also failed: {revoke_error}"
                        )));
                    }
                    return Err(AttemptCoordinatorError::Persistence(error.to_string()));
                }
            };
            store
                .settle_goal_budget(&reservation_id, usage_for_budget(&attempt.usage))
                .map_err(AttemptCoordinatorError::Budget)?;
            attempt
        };

        if let Some(coding_request) = coding_request.as_ref() {
            return self
                .finish_coding_attempt(
                    &request,
                    coding_request,
                    runtime_outcome,
                    terminal_attempt,
                    cancel,
                )
                .await;
        }

        self.finish_non_coding_attempt(&request, runtime_outcome, terminal_attempt)
    }

    async fn recover_settlement(
        &self,
        request: &AttemptRequest,
        attempt: GoalAttempt,
        cancel: CancellationToken,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError> {
        if attempt.runtime_id != request.runtime_id || attempt.role != request.role {
            return Err(AttemptCoordinatorError::Persistence(
                "attempt sequence retry conflicts with persisted runtime identity".into(),
            ));
        }
        if attempt.status == AttemptStatus::Running {
            return Err(AttemptCoordinatorError::Persistence(
                "attempt sequence is already running".into(),
            ));
        }
        let reservation_id = attempt
            .input
            .get("budget_reservation_id")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| {
                AttemptCoordinatorError::Persistence(
                    "terminal attempt has no budget reservation identity".into(),
                )
            })?;
        self.store
            .lock()
            .unwrap()
            .settle_goal_budget(reservation_id, usage_for_budget(&attempt.usage))
            .map_err(AttemptCoordinatorError::Budget)?;

        let runtime_outcome = match attempt.status {
            AttemptStatus::Succeeded => Ok(attempt.output.clone().ok_or_else(|| {
                AttemptCoordinatorError::Persistence(
                    "successful attempt has no persisted runtime output".into(),
                )
            })?),
            AttemptStatus::Failed | AttemptStatus::Cancelled => {
                Err(attempt.failure.clone().ok_or_else(|| {
                    AttemptCoordinatorError::Persistence(
                        "failed attempt has no persisted runtime failure".into(),
                    )
                })?)
            }
            AttemptStatus::Running => unreachable!("running attempt rejected above"),
        };

        if attempt.input.get("runtime_request").is_some() {
            let persisted_request =
                attempt
                    .input
                    .get("runtime_request")
                    .cloned()
                    .ok_or_else(|| {
                        AttemptCoordinatorError::Persistence(
                            "coding attempt has no persisted runtime request".into(),
                        )
                    })?;
            let coding_request: CodingAttemptRequest = serde_json::from_value(persisted_request)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?;
            return self
                .finish_coding_attempt(request, &coding_request, runtime_outcome, attempt, cancel)
                .await;
        }
        self.finish_non_coding_attempt(request, runtime_outcome, attempt)
    }

    fn finish_non_coding_attempt(
        &self,
        request: &AttemptRequest,
        runtime_outcome: Result<RuntimeResult, RuntimeFailure>,
        terminal_attempt: GoalAttempt,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError> {
        match runtime_outcome {
            Ok(_) => {
                let goal = self.transition_after(
                    request.goal_id,
                    GoalState::Completed,
                    None,
                    serde_json::json!({
                        "action": "attempt_succeeded",
                        "attempt_id": terminal_attempt.id.0,
                    }),
                )?;
                Ok(AttemptCoordinationOutcome::Succeeded {
                    attempt: terminal_attempt,
                    goal,
                })
            }
            Err(failure) => self.failure_outcome(request, terminal_attempt, failure),
        }
    }

    async fn resume_coding_attempt(
        &self,
        request: &AttemptRequest,
        coding_request: &CodingAttemptRequest,
        cancel: CancellationToken,
    ) -> Result<Option<AttemptCoordinationOutcome>, AttemptCoordinatorError> {
        let (persisted, verification, attempt, goal) = {
            let store = self.store.lock().unwrap();
            let Some(persisted) = store
                .load_coding_job(coding_request.job.job_id)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
            else {
                return Ok(None);
            };
            if persisted.report.goal_id != request.goal_id {
                return Err(AttemptCoordinatorError::Persistence(
                    "persisted coding job belongs to another goal".into(),
                ));
            }
            let verification = store
                .load_verification_report(coding_request.job.job_id)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?;
            let attempt = store
                .attempt(persisted.report.attempt_id)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
                .ok_or_else(|| {
                    AttemptCoordinatorError::Persistence(
                        "persisted coding attempt is missing".into(),
                    )
                })?;
            let goal = store
                .get_goal(request.goal_id)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
                .ok_or(AttemptCoordinatorError::GoalNotFound(request.goal_id))?;
            (persisted, verification, attempt, goal)
        };

        if let Some(verification) = verification {
            if goal.state == GoalState::Running {
                return self
                    .outcome_for_verification(
                        request,
                        coding_request,
                        &persisted,
                        attempt,
                        verification.report,
                    )
                    .map(Some);
            }
            return Ok(Some(AttemptCoordinationOutcome::Succeeded {
                attempt,
                goal,
            }));
        }
        let report = match self
            .run_and_persist_verification(coding_request, &persisted, &attempt.evidence, cancel)
            .await
        {
            Ok(report) => report,
            Err(error) => {
                return self
                    .block_coding_service_error(request.goal_id, attempt, &error.to_string())
                    .map(Some)
            }
        };
        let outcome =
            self.outcome_for_verification(request, coding_request, &persisted, attempt, report)?;
        Ok(Some(outcome))
    }

    async fn finish_coding_attempt(
        &self,
        request: &AttemptRequest,
        coding_request: &CodingAttemptRequest,
        runtime_outcome: Result<RuntimeResult, RuntimeFailure>,
        attempt: GoalAttempt,
        cancel: CancellationToken,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError> {
        let evidence = match &runtime_outcome {
            Ok(result) => &result.evidence,
            Err(failure) => &failure.evidence,
        };
        let bundle = match parse_coding_evidence(evidence) {
            Ok(bundle) => bundle,
            Err(error) => return self.block_coding_service_error(request.goal_id, attempt, &error),
        };
        if bundle.report.goal_id != request.goal_id
            || bundle.report.attempt_id != attempt.id
            || bundle.report.job_id != coding_request.job.job_id
        {
            return self.block_coding_service_error(
                request.goal_id,
                attempt,
                "Coding evidence identity mismatch",
            );
        }
        let persisted = {
            let store = self.store.lock().unwrap();
            match store
                .load_coding_job(bundle.report.job_id)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
            {
                Some(existing) => existing,
                None => store
                    .persist_coding_job(
                        &bundle.report,
                        &bundle.worktree_ref,
                        &bundle.diff,
                        self.clock.wall_now().0,
                    )
                    .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?,
            }
        };

        if let Err(failure) = runtime_outcome {
            return self.failure_outcome(request, attempt, failure);
        }

        let report = match self
            .run_and_persist_verification(coding_request, &persisted, evidence, cancel)
            .await
        {
            Ok(report) => report,
            Err(error) => {
                return self.block_coding_service_error(
                    request.goal_id,
                    attempt,
                    &error.to_string(),
                )
            }
        };
        self.outcome_for_verification(request, coding_request, &persisted, attempt, report)
    }

    async fn run_and_persist_verification(
        &self,
        coding_request: &CodingAttemptRequest,
        persisted: &super::PersistedCodingJob,
        evidence: &[fabric::AttemptEvidence],
        cancel: CancellationToken,
    ) -> Result<VerificationReport, AttemptCoordinatorError> {
        let coding = self.coding_verification.as_ref().ok_or_else(|| {
            AttemptCoordinatorError::Persistence("coding verification is not configured".into())
        })?;
        let worktree = resolve_worktree(&coding.worktree_base, &persisted.worktree_ref)
            .map_err(AttemptCoordinatorError::Persistence)?;
        let audit = capability_audit(evidence).map_err(AttemptCoordinatorError::Persistence)?;
        let context = VerificationContext {
            job_id: persisted.report.job_id,
            goal_id: persisted.report.goal_id,
            attempt_id: persisted.report.attempt_id,
            worktree,
            base_commit: persisted.report.base_commit.clone(),
            changed_files: persisted.report.changed_files.clone(),
            allowed_paths: coding_request.job.workspace.allowed_paths().to_vec(),
            forbidden_paths: coding_request.job.workspace.forbidden_paths().to_vec(),
            capability_audit: audit,
            selection: VerificationSelection::default(),
        };
        let report = coding
            .verifier
            .verify_coding_attempt(&context, cancel)
            .await
            .map_err(AttemptCoordinatorError::Persistence)?;
        let store = self.store.lock().unwrap();
        store
            .persist_verification_report(&report, self.clock.wall_now().0)
            .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?;
        Ok(report)
    }

    fn outcome_for_verification(
        &self,
        request: &AttemptRequest,
        coding_request: &CodingAttemptRequest,
        persisted: &super::PersistedCodingJob,
        attempt: GoalAttempt,
        report: VerificationReport,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError> {
        let verification_evidence: Vec<_> = report
            .checks
            .iter()
            .filter(|check| !check.passed || !check.evidence.is_empty())
            .map(|check| fabric::AttemptEvidence {
                kind: format!("verification_{}", check.name),
                summary: check.summary.clone(),
                content: check.evidence.join("\n"),
            })
            .collect();
        let attempt = if verification_evidence.is_empty() {
            attempt
        } else {
            self.store
                .lock()
                .unwrap()
                .append_attempt_evidence(attempt.id, &verification_evidence)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
        };
        if report.passed {
            if let Some(approvals) = self
                .coding_verification
                .as_ref()
                .and_then(|coding| coding.approvals.as_ref())
            {
                let approval = create_apply_approval(
                    approvals,
                    coding_request,
                    persisted,
                    &report,
                    self.clock.wall_now().0,
                )?;
                let goal = self.transition_after(
                    request.goal_id,
                    GoalState::AwaitingHuman,
                    Some(GoalWaitReason::HumanInput {
                        prompt: format!("approval:{}", approval.id),
                    }),
                    serde_json::json!({
                        "action": "coding_approval_requested",
                        "attempt_id": attempt.id.0,
                        "job_id": report.job_id.0,
                        "approval_id": approval.id.0,
                        "subject_hash": approval.subject_hash,
                    }),
                )?;
                return Ok(AttemptCoordinationOutcome::Succeeded { attempt, goal });
            }
            let goal = self.transition_after(
                request.goal_id,
                GoalState::Blocked,
                Some(GoalWaitReason::ExternalEvent {
                    key: "approval required".into(),
                }),
                serde_json::json!({
                    "action": "coding_verification_passed",
                    "attempt_id": attempt.id.0,
                    "job_id": report.job_id.0,
                }),
            )?;
            return Ok(AttemptCoordinationOutcome::Succeeded { attempt, goal });
        }
        let failure = RuntimeFailure {
            class: FailureClass::ToolFailure,
            message: "required coding verification failed".into(),
            retryable: true,
            usage: AttemptUsage::default(),
            evidence: verification_evidence,
        };
        self.failure_outcome(request, attempt, failure)
    }

    fn failure_outcome(
        &self,
        request: &AttemptRequest,
        attempt: GoalAttempt,
        failure: RuntimeFailure,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError> {
        let attempt_count = {
            let store = self.store.lock().unwrap();
            store
                .attempts_for_goal(request.goal_id, usize::MAX)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
                .into_iter()
                .filter(|candidate| candidate.role == request.role)
                .count() as u32
        };
        let decision = self.retry_policy.decide(
            request.role,
            attempt_count,
            &failure,
            request.escalation_runtime_id.as_ref(),
        );
        let goal = self.persist_decision(request.goal_id, attempt.id.0.to_string(), &decision)?;
        Ok(AttemptCoordinationOutcome::Failed {
            attempt,
            decision,
            goal,
        })
    }

    fn block_coding_service_error(
        &self,
        goal_id: GoalId,
        attempt: GoalAttempt,
        error: &str,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError> {
        let goal = self.transition_after(
            goal_id,
            GoalState::Blocked,
            Some(GoalWaitReason::ExternalEvent {
                key: "verification service error".into(),
            }),
            serde_json::json!({
                "action": "coding_verification_service_error",
                "attempt_id": attempt.id.0,
                "error": error,
            }),
        )?;
        Ok(AttemptCoordinationOutcome::Succeeded { attempt, goal })
    }

    fn persist_decision(
        &self,
        goal_id: GoalId,
        attempt_id: String,
        decision: &RetryDecision,
    ) -> Result<GoalSnapshot, AttemptCoordinatorError> {
        match decision {
            RetryDecision::RetrySame { after_ms, .. } => {
                let until_ms = self
                    .clock
                    .wall_now()
                    .0
                    .saturating_add((*after_ms).min(i64::MAX as u64) as i64);
                self.transition_after(
                    goal_id,
                    GoalState::Blocked,
                    Some(GoalWaitReason::Backoff { until_ms }),
                    serde_json::json!({
                        "action": "retry_scheduled",
                        "attempt_id": attempt_id,
                        "until_ms": until_ms,
                    }),
                )
            }
            RetryDecision::Escalate { runtime_id, .. } => self.transition_after(
                goal_id,
                GoalState::Blocked,
                Some(GoalWaitReason::ExternalEvent {
                    key: format!("runtime:{}", runtime_id.0),
                }),
                serde_json::json!({
                    "action": "runtime_escalated",
                    "attempt_id": attempt_id,
                    "runtime_id": runtime_id.0,
                }),
            ),
            RetryDecision::AwaitHuman { reason } => self.transition_after(
                goal_id,
                GoalState::AwaitingHuman,
                Some(GoalWaitReason::HumanInput {
                    prompt: reason.clone(),
                }),
                serde_json::json!({"action": "await_human", "attempt_id": attempt_id}),
            ),
            RetryDecision::Fail { reason } => self.transition_after(
                goal_id,
                GoalState::Failed,
                None,
                serde_json::json!({
                    "action": "attempt_failed_terminal",
                    "attempt_id": attempt_id,
                    "reason": reason,
                }),
            ),
            RetryDecision::Cancel => self.transition_after(
                goal_id,
                GoalState::Cancelled,
                None,
                serde_json::json!({"action": "attempt_cancelled", "attempt_id": attempt_id}),
            ),
        }
    }

    fn transition_after(
        &self,
        goal_id: GoalId,
        state: GoalState,
        wait_reason: Option<GoalWaitReason>,
        payload: serde_json::Value,
    ) -> Result<GoalSnapshot, AttemptCoordinatorError> {
        let store = self.store.lock().unwrap();
        let current = store
            .get_goal(goal_id)
            .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
            .ok_or(AttemptCoordinatorError::GoalNotFound(goal_id))?;
        store
            .transition_goal(
                goal_id,
                current.version,
                state,
                wait_reason.as_ref(),
                &payload,
            )
            .map_err(AttemptCoordinatorError::Transition)
    }
}

fn cancelled_failure() -> RuntimeFailure {
    RuntimeFailure {
        class: FailureClass::Cancelled,
        message: "attempt cancelled".into(),
        retryable: false,
        usage: AttemptUsage::default(),
        evidence: vec![],
    }
}

fn usage_for_budget(usage: &AttemptUsage) -> GoalBudgetUsage {
    GoalBudgetUsage {
        input_tokens: usage.input_tokens,
        output_tokens: usage.output_tokens,
        cost_usd: usage.cost_usd.unwrap_or_default(),
        attempts: 1,
    }
}

struct CodingEvidenceBundle {
    report: CodingJobReport,
    worktree_ref: PathBuf,
    diff: Vec<u8>,
}

fn parse_coding_evidence(
    evidence: &[fabric::AttemptEvidence],
) -> Result<CodingEvidenceBundle, String> {
    let content = |kind: &str| {
        evidence
            .iter()
            .find(|item| item.kind == kind)
            .map(|item| item.content.as_str())
            .ok_or_else(|| format!("Coding result is missing {kind} evidence"))
    };
    let report = serde_json::from_str::<CodingJobReport>(content("coding_job_report")?)
        .map_err(|error| format!("invalid coding job report: {error}"))?;
    let worktree_ref = PathBuf::from(content("coding_worktree_ref")?);
    validate_relative_worktree_ref(&worktree_ref)?;
    let diff = base64::engine::general_purpose::STANDARD
        .decode(content("coding_diff_base64")?)
        .map_err(|error| format!("invalid coding diff encoding: {error}"))?;
    Ok(CodingEvidenceBundle {
        report,
        worktree_ref,
        diff,
    })
}

fn capability_audit(
    evidence: &[fabric::AttemptEvidence],
) -> Result<CapabilityAuditSummary, String> {
    let item = evidence
        .iter()
        .find(|item| item.kind == "coding_capability_audit")
        .ok_or_else(|| "Coding result is missing capability audit evidence".to_string())?;
    serde_json::from_str(&item.content)
        .map(CapabilityAuditSummary::normalized)
        .map_err(|error| format!("invalid capability audit evidence: {error}"))
}

fn validate_relative_worktree_ref(path: &Path) -> Result<(), String> {
    if path.as_os_str().is_empty()
        || path.is_absolute()
        || path.components().any(|component| {
            matches!(
                component,
                Component::ParentDir | Component::RootDir | Component::Prefix(_)
            )
        })
    {
        return Err("invalid managed worktree reference".into());
    }
    Ok(())
}

fn resolve_worktree(base: &Path, relative: &Path) -> Result<PathBuf, String> {
    validate_relative_worktree_ref(relative)?;
    let path = base
        .join(relative)
        .canonicalize()
        .map_err(|error| format!("resolving managed worktree: {error}"))?;
    if !path.starts_with(base) || !path.is_dir() {
        return Err("managed worktree escapes configured base".into());
    }
    Ok(path)
}

fn create_apply_approval(
    approvals: &Arc<Mutex<ApprovalRepository>>,
    coding_request: &CodingAttemptRequest,
    persisted: &super::PersistedCodingJob,
    verification: &VerificationReport,
    now_ms: i64,
) -> Result<fabric::ApprovalSnapshot, AttemptCoordinatorError> {
    if !verification.passed
        || verification.job_id != persisted.report.job_id
        || verification.goal_id != persisted.report.goal_id
        || verification.attempt_id != persisted.report.attempt_id
    {
        return Err(AttemptCoordinatorError::Persistence(
            "unverified or mismatched coding result cannot request approval".into(),
        ));
    }
    let verification_json = serde_json::to_vec(verification)
        .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?;
    let verification_sha256 = format!("{:x}", Sha256::digest(verification_json));
    let subject = ApprovalSubject {
        category: ApprovalCategory::ApplyCode,
        goal_id: persisted.report.goal_id,
        attempt_id: Some(persisted.report.attempt_id),
        job_id: Some(persisted.report.job_id),
        attributes: BTreeMap::from([
            ("base_commit".into(), persisted.report.base_commit.clone()),
            (
                "repository_root".into(),
                coding_request
                    .job
                    .workspace
                    .repository_root()
                    .to_string_lossy()
                    .into_owned(),
            ),
            ("diff_sha256".into(), persisted.diff_sha256.clone()),
            ("verification_sha256".into(), verification_sha256),
            (
                "changed_file_count".into(),
                persisted.report.changed_files.len().to_string(),
            ),
            (
                "verification_summary".into(),
                "all required checks passed".into(),
            ),
        ]),
        allowed_scope: coding_request.job.workspace.allowed_paths().to_vec(),
        apply_target: Some(PathBuf::from(".")),
    };
    let approval = approvals
        .lock()
        .unwrap()
        .create(ApprovalCreate {
            subject,
            risk: ApprovalRisk::High,
            summary: format!(
                "Apply verified coding diff for Goal {} ({} changed files)",
                persisted.report.goal_id.0,
                persisted.report.changed_files.len()
            ),
            artifacts: vec![ApprovalArtifactRef {
                kind: "diff".into(),
                relative_path: persisted.diff_artifact_ref.clone(),
                sha256: persisted.diff_sha256.clone(),
            }],
            created_at_ms: now_ms,
            expires_at_ms: now_ms.saturating_add(24 * 60 * 60 * 1_000),
        })
        .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?;
    Ok(approval)
}
