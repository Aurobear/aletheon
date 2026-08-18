//! One-shot durable Goal attempt coordination.
//!
//! A call performs exactly one runtime invocation. Any retry or escalation is
//! represented as durable Goal state for a later scheduler tick.

use super::{
    AttemptCoordinationOutcome, AttemptCoordinatorError, AttemptRequest, BeginAttemptCommand,
    CodingVerifier, CodingWorktreePort, GoalAttemptPersistencePort, GoalBudgetRequest,
    PersistedCodingJob,
};
use crate::approval::{ApprovalCreateCommand, GoalApprovalPort};
use crate::goal_attempt::{GoalAttempt, GoalAttemptPort};
use crate::goal_frame::GoalFrame;
use crate::goal_retry::{RetryDecision, RetryPolicy};
use crate::verification::{CapabilityAuditSummary, VerificationContext, VerificationSelection};
use ::contracts::CodingAttemptRequest;
use ::contracts::{
    ApprovalArtifactRef, ApprovalCategory, ApprovalRisk, ApprovalSubject, AttemptId, AttemptStatus,
    AttemptUsage, Clock, CodingJobReport, FailureClass, GoalBudgetUsage, GoalId, GoalSnapshot,
    GoalState, GoalWaitReason, RuntimeFailure, RuntimeResult, VerificationReport,
};
use base64::Engine;
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use std::path::{Component, Path, PathBuf};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Clone)]
struct CodingVerification {
    verifier: Arc<dyn CodingVerifier>,
    worktrees: Arc<dyn CodingWorktreePort>,
    approvals: Option<Arc<dyn GoalApprovalPort>>,
}

pub struct AttemptCoordinator {
    persistence: Arc<dyn GoalAttemptPersistencePort>,
    executor: Arc<dyn GoalAttemptPort>,
    clock: Arc<dyn Clock>,
    retry_policy: RetryPolicy,
    coding_verification: Option<CodingVerification>,
}

impl AttemptCoordinator {
    pub fn new(
        persistence: Arc<dyn GoalAttemptPersistencePort>,
        executor: Arc<dyn GoalAttemptPort>,
        clock: Arc<dyn Clock>,
        retry_policy: RetryPolicy,
    ) -> Self {
        Self {
            persistence,
            executor,
            clock,
            retry_policy,
            coding_verification: None,
        }
    }

    pub fn with_coding_verification(
        mut self,
        verifier: Arc<dyn CodingVerifier>,
        worktrees: Arc<dyn CodingWorktreePort>,
    ) -> Result<Self, AttemptCoordinatorError> {
        self.coding_verification = Some(CodingVerification {
            verifier,
            worktrees,
            approvals: None,
        });
        Ok(self)
    }

    pub fn with_approval_port(
        mut self,
        approvals: Arc<dyn GoalApprovalPort>,
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
        let is_coding = request_value
            .as_ref()
            .is_some_and(|value| value.get("job").is_some() && value.get("task_input").is_some());
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
        let context = self.persistence.context(
            request.goal_id,
            request.expected_version,
            request.sequence,
        )?;
        if let Some(attempt) = context.existing_attempt {
            return self.recover_settlement(&request, attempt, cancel).await;
        }

        // Resolve before budget reservation or attempt creation.
        if !self.executor.is_available(&request.runtime_id) {
            return Err(AttemptCoordinatorError::RuntimeUnavailable(
                request.runtime_id,
            ));
        }

        let frame_task = coding_request
            .as_ref()
            .map(|coding| coding.task_input.as_str())
            .unwrap_or(request.task.as_str());
        let frame = GoalFrame::build(&context.goal, &context.previous_attempts, frame_task);
        let rendered_task = if let Some(coding) = coding_request.as_mut() {
            coding.job.goal_id = request.goal_id;
            coding.job.attempt_id = AttemptId::new();
            coding.task_input = frame.render();
            serde_json::to_string(coding)
                .map_err(|error| AttemptCoordinatorError::Persistence(error.to_string()))?
        } else {
            frame.render()
        };
        let attempt_id = coding_request.as_ref().map(|coding| coding.job.attempt_id);
        let input = serde_json::json!({
            "task": request.task,
            "goal_version": request.expected_version,
            "goal_frame": frame,
            "runtime_request": coding_request,
        });
        let begun = self.persistence.begin(BeginAttemptCommand {
            attempt_id,
            goal_id: request.goal_id,
            expected_version: request.expected_version,
            sequence: request.sequence,
            runtime_id: request.runtime_id.clone(),
            role: request.role,
            input,
            budget: GoalBudgetRequest {
                input_tokens: request.estimated_usage.input_tokens,
                output_tokens: request.estimated_usage.output_tokens,
                cost_usd: request.estimated_usage.cost_usd.unwrap_or_default(),
                attempts: 1,
            },
            now_ms: self.clock.wall_now().0,
        })?;
        let reservation_id = begun.reservation_id;
        let running_attempt = begun.attempt;

        let runtime_outcome = tokio::select! {
            outcome = self.executor.run_once(&request.runtime_id, &rendered_task, cancel.clone()) => outcome,
            _ = cancel.cancelled() => Err(cancelled_failure()),
        };

        let terminal_attempt = self.persistence.finish_and_settle(
            running_attempt.id,
            &reservation_id,
            runtime_outcome.clone(),
        )?;

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
        self.persistence
            .settle_budget(reservation_id, usage_for_budget(&attempt.usage))?;

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

        if let Some(persisted_request) = attempt
            .input
            .get("runtime_request")
            .filter(|value| !value.is_null())
            .cloned()
        {
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
        let Some(persisted) = self
            .persistence
            .load_coding_job(coding_request.job.job_id)?
        else {
            return Ok(None);
        };
        if persisted.report.goal_id != request.goal_id {
            return Err(AttemptCoordinatorError::Persistence(
                "persisted coding job belongs to another goal".into(),
            ));
        }
        let verification = self
            .persistence
            .load_verification(coding_request.job.job_id)?;
        let attempt = self
            .persistence
            .attempt(persisted.report.attempt_id)?
            .ok_or_else(|| {
                AttemptCoordinatorError::Persistence("persisted coding attempt is missing".into())
            })?;
        let goal = self
            .persistence
            .goal(request.goal_id)?
            .ok_or(AttemptCoordinatorError::GoalNotFound(request.goal_id))?;

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
        let persisted = match self.persistence.load_coding_job(bundle.report.job_id)? {
            Some(existing) => existing,
            None => self.persistence.persist_coding_job(
                &bundle.report,
                &bundle.worktree_ref,
                &bundle.diff,
                self.clock.wall_now().0,
            )?,
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
        persisted: &PersistedCodingJob,
        evidence: &[::contracts::AttemptEvidence],
        cancel: CancellationToken,
    ) -> Result<VerificationReport, AttemptCoordinatorError> {
        let coding = self.coding_verification.as_ref().ok_or_else(|| {
            AttemptCoordinatorError::Persistence("coding verification is not configured".into())
        })?;
        let worktree = coding
            .worktrees
            .resolve(&persisted.worktree_ref)
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
        self.persistence
            .persist_verification(&report, self.clock.wall_now().0)?;
        Ok(report)
    }

    fn outcome_for_verification(
        &self,
        request: &AttemptRequest,
        coding_request: &CodingAttemptRequest,
        persisted: &PersistedCodingJob,
        attempt: GoalAttempt,
        report: VerificationReport,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError> {
        let verification_evidence: Vec<_> = report
            .checks
            .iter()
            .filter(|check| !check.passed || !check.evidence.is_empty())
            .map(|check| ::contracts::AttemptEvidence {
                kind: format!("verification_{}", check.name),
                summary: check.summary.clone(),
                content: check.evidence.join("\n"),
            })
            .collect();
        let attempt = if verification_evidence.is_empty() {
            attempt
        } else {
            self.persistence
                .append_evidence(attempt.id, &verification_evidence)?
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
        let attempt_count = self
            .persistence
            .attempt_count(request.goal_id, request.role)?;
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
        self.persistence
            .transition_latest(goal_id, state, wait_reason.as_ref(), &payload)
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
    evidence: &[::contracts::AttemptEvidence],
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
    evidence: &[::contracts::AttemptEvidence],
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

fn create_apply_approval(
    approvals: &Arc<dyn GoalApprovalPort>,
    coding_request: &CodingAttemptRequest,
    persisted: &PersistedCodingJob,
    verification: &VerificationReport,
    now_ms: i64,
) -> Result<::contracts::ApprovalSnapshot, AttemptCoordinatorError> {
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
        .create(ApprovalCreateCommand {
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

#[async_trait::async_trait]
impl super::GoalAttemptUseCase for AttemptCoordinator {
    async fn execute_one(
        &self,
        request: AttemptRequest,
        cancel: CancellationToken,
    ) -> Result<AttemptCoordinationOutcome, AttemptCoordinatorError> {
        AttemptCoordinator::execute_one(self, request, cancel).await
    }
}
