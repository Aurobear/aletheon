//! Restart-safe, one-time coordination of approved coding patches.

use super::{
    ApprovalApplyClaim, ApprovalApplyReceipt, ApprovalRepository, ApprovalRepositoryError,
};
use crate::r#impl::goal::ObjectiveStore;
use async_trait::async_trait;
use corpus::tools::subagent::{
    ApplyAuthorization, ApplyAuthorizer, ApplyError, ApplySpec, ControlledApply,
};
use fabric::{
    ApprovalId, ApprovalStatus, Clock, GoalId, GoalState, GoalWaitReason, OperationId,
    OperationKind, OperationRequest, ProcessId,
};
use sha2::{Digest, Sha256};
use std::io::Write;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub struct ApplyCoordinatorConfig {
    pub repository_root: PathBuf,
    pub worktree_base: PathBuf,
    pub timeout: Duration,
}

#[async_trait]
pub trait ManagedWorktreeCleaner: Send + Sync {
    async fn cleanup(&self, job_id: fabric::CodingJobId, worktree: &Path) -> anyhow::Result<()>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApplyCoordinationOutcome {
    AwaitingDecision,
    Rejected {
        goal_id: GoalId,
        revision_requested: bool,
    },
    DuplicateInProgress {
        operation_id: OperationId,
    },
    Applied(ApprovalApplyReceipt),
    Failed(ApprovalApplyReceipt),
    Recovered(ApprovalApplyReceipt),
}

#[derive(Debug)]
pub enum ApplyCoordinationError {
    Approval(String),
    Goal(String),
    Evidence(String),
    Operation(String),
    Apply(String),
    Cleanup(String),
}

impl std::fmt::Display for ApplyCoordinationError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{self:?}")
    }
}
impl std::error::Error for ApplyCoordinationError {}

pub struct ApplyCoordinator {
    store: Arc<Mutex<ObjectiveStore>>,
    approvals: Arc<Mutex<ApprovalRepository>>,
    operations: Arc<aletheon_kernel::operation::OperationTable>,
    clock: Arc<dyn Clock>,
    config: ApplyCoordinatorConfig,
    cleaner: Arc<dyn ManagedWorktreeCleaner>,
}

impl ApplyCoordinator {
    pub fn new(
        store: Arc<Mutex<ObjectiveStore>>,
        approvals: Arc<Mutex<ApprovalRepository>>,
        operations: Arc<aletheon_kernel::operation::OperationTable>,
        clock: Arc<dyn Clock>,
        config: ApplyCoordinatorConfig,
        cleaner: Arc<dyn ManagedWorktreeCleaner>,
    ) -> Result<Self, ApplyCoordinationError> {
        if config.timeout.is_zero() {
            return Err(ApplyCoordinationError::Apply(
                "timeout must be positive".into(),
            ));
        }
        Ok(Self {
            store,
            approvals,
            operations,
            clock,
            config,
            cleaner,
        })
    }

    pub async fn coordinate(
        &self,
        approval_id: ApprovalId,
        owner_process: ProcessId,
        cancel: CancellationToken,
    ) -> Result<ApplyCoordinationOutcome, ApplyCoordinationError> {
        let approval = self
            .approvals
            .lock()
            .unwrap()
            .get(approval_id)
            .map_err(approval_error)?
            .ok_or_else(|| ApplyCoordinationError::Approval("approval not found".into()))?;
        let existing_receipt = {
            self.approvals
                .lock()
                .unwrap()
                .apply_receipt(approval_id)
                .map_err(approval_error)?
        };
        if let Some(receipt) = existing_receipt {
            self.reconcile_receipt(&approval, &receipt).await?;
            return Ok(ApplyCoordinationOutcome::Recovered(receipt));
        }
        match approval.status {
            ApprovalStatus::Pending => return Ok(ApplyCoordinationOutcome::AwaitingDecision),
            ApprovalStatus::Rejected | ApprovalStatus::Expired => {
                return self.transition_rejected(&approval)
            }
            ApprovalStatus::Consumed => {
                return Err(ApplyCoordinationError::Approval(
                    "consumed approval has no apply receipt".into(),
                ))
            }
            ApprovalStatus::Approved => {}
        }

        let proposed_operation = OperationId::new();
        let claim = self
            .approvals
            .lock()
            .unwrap()
            .claim_apply(approval_id, proposed_operation, self.clock.wall_now().0)
            .map_err(approval_error)?;
        let operation_id = match claim {
            ApprovalApplyClaim::Claimed(value) | ApprovalApplyClaim::Existing(value) => {
                value.operation_id
            }
        };
        let request = OperationRequest {
            owner: owner_process,
            parent: None,
            kind: OperationKind::Other("approved_apply".into()),
            deadline: None,
        };
        if self
            .operations
            .submit_with_id(operation_id, request)
            .await
            .is_err()
        {
            return Ok(ApplyCoordinationOutcome::DuplicateInProgress { operation_id });
        }
        // A recovered durable claim is intentionally re-registered with the
        // fresh in-memory table; a concurrently active claim cannot register twice.
        self.operations
            .start(operation_id)
            .await
            .map_err(|error| ApplyCoordinationError::Operation(error.to_string()))?;
        self.ensure_goal_running(approval.subject.goal_id)?;

        let apply_result = self.execute_approved(&approval, cancel).await;
        let finished_at = self.clock.wall_now().0;
        let receipt = match apply_result {
            Ok(outcome) => ApprovalApplyReceipt {
                approval_id,
                operation_id,
                goal_id: approval.subject.goal_id,
                success: true,
                applied_head: Some(outcome.head),
                diff_sha256: outcome.diff_sha256,
                changed_paths: outcome.changed_paths,
                error: None,
                finished_at_ms: finished_at,
            },
            Err(error) => ApprovalApplyReceipt {
                approval_id,
                operation_id,
                goal_id: approval.subject.goal_id,
                success: false,
                applied_head: None,
                diff_sha256: approval
                    .subject
                    .attributes
                    .get("diff_sha256")
                    .cloned()
                    .unwrap_or_default(),
                changed_paths: vec![],
                error: Some(bound(&error.to_string(), 2048)),
                finished_at_ms: finished_at,
            },
        };
        self.approvals
            .lock()
            .unwrap()
            .finish_apply(&receipt)
            .map_err(approval_error)?;

        if receipt.success {
            self.operations
                .succeed(operation_id)
                .await
                .map_err(|error| ApplyCoordinationError::Operation(error.to_string()))?;
            self.transition_terminal(receipt.goal_id, GoalState::Completed, &receipt)?;
            let (job_id, worktree) = self.worktree_for(&approval)?;
            self.cleaner
                .cleanup(job_id, &worktree)
                .await
                .map_err(|error| ApplyCoordinationError::Cleanup(error.to_string()))?;
            Ok(ApplyCoordinationOutcome::Applied(receipt))
        } else {
            self.operations
                .fail(operation_id, receipt.error.clone().unwrap_or_default())
                .await
                .map_err(|error| ApplyCoordinationError::Operation(error.to_string()))?;
            self.transition_terminal(receipt.goal_id, GoalState::Blocked, &receipt)?;
            Ok(ApplyCoordinationOutcome::Failed(receipt))
        }
    }

    fn transition_rejected(
        &self,
        approval: &fabric::ApprovalSnapshot,
    ) -> Result<ApplyCoordinationOutcome, ApplyCoordinationError> {
        let revision_requested = approval
            .resolution
            .as_ref()
            .and_then(|value| value.reason.as_deref())
            == Some("owner requested revision");
        let store = self.store.lock().unwrap();
        let goal = store
            .get_goal(approval.subject.goal_id)
            .map_err(|error| ApplyCoordinationError::Goal(error.to_string()))?
            .ok_or_else(|| ApplyCoordinationError::Goal("goal not found".into()))?;
        let target = if revision_requested {
            GoalState::Ready
        } else {
            GoalState::Cancelled
        };
        if !goal.state.is_terminal() && goal.state != target {
            store
                .transition_goal(
                    goal.id,
                    goal.version,
                    target,
                    None,
                    &serde_json::json!({"approval_id":approval.id.0,"revision":revision_requested}),
                )
                .map_err(|error| ApplyCoordinationError::Goal(error.to_string()))?;
        }
        Ok(ApplyCoordinationOutcome::Rejected {
            goal_id: approval.subject.goal_id,
            revision_requested,
        })
    }

    fn ensure_goal_running(&self, goal_id: GoalId) -> Result<(), ApplyCoordinationError> {
        let store = self.store.lock().unwrap();
        let mut goal = store
            .get_goal(goal_id)
            .map_err(|error| ApplyCoordinationError::Goal(error.to_string()))?
            .ok_or_else(|| ApplyCoordinationError::Goal("goal not found".into()))?;
        if matches!(
            goal.state,
            GoalState::AwaitingHuman | GoalState::Blocked | GoalState::Suspended
        ) {
            goal = store
                .transition_goal(
                    goal.id,
                    goal.version,
                    GoalState::Ready,
                    None,
                    &serde_json::json!({"action":"approved_apply_ready"}),
                )
                .map_err(|error| ApplyCoordinationError::Goal(error.to_string()))?;
        }
        if goal.state == GoalState::Ready {
            goal = store
                .transition_goal(
                    goal.id,
                    goal.version,
                    GoalState::Running,
                    None,
                    &serde_json::json!({"action":"approved_apply_running"}),
                )
                .map_err(|error| ApplyCoordinationError::Goal(error.to_string()))?;
        }
        if goal.state != GoalState::Running {
            return Err(ApplyCoordinationError::Goal(format!(
                "goal is not runnable: {}",
                goal.state
            )));
        }
        Ok(())
    }

    async fn execute_approved(
        &self,
        approval: &fabric::ApprovalSnapshot,
        cancel: CancellationToken,
    ) -> Result<corpus::tools::subagent::ApplyOutcome, ApplyError> {
        let job_id = approval
            .subject
            .job_id
            .ok_or_else(|| ApplyError::Unauthorized("approval has no coding job".into()))?;
        if approval.category != fabric::ApprovalCategory::ApplyCode
            || approval.subject.apply_target.as_deref() != Some(Path::new("."))
        {
            return Err(ApplyError::Unauthorized(
                "approval is not a repository-root code apply".into(),
            ));
        }
        let (coding, verification, artifact_dir) = {
            let store = self.store.lock().unwrap();
            let coding = store
                .load_coding_job(job_id)
                .map_err(|error| ApplyError::Artifact(error.to_string()))?
                .ok_or_else(|| ApplyError::Artifact("coding job not found".into()))?;
            let verification = store
                .load_verification_report(job_id)
                .map_err(|error| ApplyError::Artifact(error.to_string()))?
                .ok_or_else(|| ApplyError::Artifact("verification report not found".into()))?;
            (coding, verification, store.artifact_dir.clone())
        };
        if !verification.report.passed
            || coding.report.goal_id != approval.subject.goal_id
            || approval.subject.attempt_id != Some(coding.report.attempt_id)
            || verification.report.job_id != coding.report.job_id
            || verification.report.goal_id != coding.report.goal_id
            || verification.report.attempt_id != coding.report.attempt_id
        {
            return Err(ApplyError::Unauthorized(
                "coding evidence identity mismatch".into(),
            ));
        }
        let verification_bytes = serde_json::to_vec(&verification.report)
            .map_err(|error| ApplyError::Artifact(error.to_string()))?;
        let verification_hash = format!("{:x}", Sha256::digest(&verification_bytes));
        if approval.subject.attributes.get("verification_sha256") != Some(&verification_hash) {
            return Err(ApplyError::Unauthorized(
                "verification hash mismatch".into(),
            ));
        }
        let approved_diff = approval
            .artifacts
            .iter()
            .find(|artifact| artifact.kind == "diff")
            .ok_or_else(|| ApplyError::Unauthorized("approval has no diff artifact".into()))?;
        if approved_diff.relative_path != coding.diff_artifact_ref
            || approved_diff.sha256 != coding.diff_sha256
            || approval.subject.attributes.get("diff_sha256") != Some(&coding.diff_sha256)
            || approval.subject.attributes.get("base_commit") != Some(&coding.report.base_commit)
        {
            return Err(ApplyError::Unauthorized(
                "approved coding artifact metadata mismatch".into(),
            ));
        }
        let verification_file = TemporaryArtifact::write(&verification_bytes)?;
        let authorizer: Arc<dyn ApplyAuthorizer> = Arc::new(RepositoryAuthorizer {
            repository: self.approvals.clone(),
        });
        let applier = ControlledApply::new(authorizer)?;
        applier
            .apply(
                ApplySpec {
                    repository_root: self.config.repository_root.clone(),
                    expected_head: coding.report.base_commit.clone(),
                    diff_artifact: artifact_dir.join(&coding.diff_artifact_ref),
                    diff_sha256: coding.diff_sha256,
                    verification_artifact: verification_file.path.clone(),
                    verification_sha256: verification_hash,
                    allowed_paths: approval.subject.allowed_scope.clone(),
                    approval_id: approval.id,
                    subject_hash: approval.subject_hash.clone(),
                    timeout: self.config.timeout,
                    dry_run: false,
                },
                cancel,
            )
            .await
    }

    fn transition_terminal(
        &self,
        goal_id: GoalId,
        target: GoalState,
        receipt: &ApprovalApplyReceipt,
    ) -> Result<(), ApplyCoordinationError> {
        let store = self.store.lock().unwrap();
        let goal = store
            .get_goal(goal_id)
            .map_err(|error| ApplyCoordinationError::Goal(error.to_string()))?
            .ok_or_else(|| ApplyCoordinationError::Goal("goal not found".into()))?;
        if goal.state == target {
            return Ok(());
        }
        let wait = (target == GoalState::Blocked).then(|| GoalWaitReason::HumanInput {
            prompt: "Approved patch failed to apply; fresh verification and approval required"
                .into(),
        });
        store
            .transition_goal(
                goal.id,
                goal.version,
                target,
                wait.as_ref(),
                &serde_json::json!({"apply_receipt":receipt}),
            )
            .map_err(|error| ApplyCoordinationError::Goal(error.to_string()))?;
        Ok(())
    }

    async fn reconcile_receipt(
        &self,
        approval: &fabric::ApprovalSnapshot,
        receipt: &ApprovalApplyReceipt,
    ) -> Result<(), ApplyCoordinationError> {
        let target = if receipt.success {
            GoalState::Completed
        } else {
            GoalState::Blocked
        };
        let current = self
            .store
            .lock()
            .unwrap()
            .get_goal(receipt.goal_id)
            .map_err(|error| ApplyCoordinationError::Goal(error.to_string()))?
            .ok_or_else(|| ApplyCoordinationError::Goal("goal not found".into()))?;
        if current.state != target {
            self.ensure_goal_running(receipt.goal_id)?;
            self.transition_terminal(receipt.goal_id, target, receipt)?;
        }
        if receipt.success {
            let (job_id, worktree) = self.worktree_for(approval)?;
            self.cleaner
                .cleanup(job_id, &worktree)
                .await
                .map_err(|error| ApplyCoordinationError::Cleanup(error.to_string()))?;
        }
        Ok(())
    }

    fn worktree_for(
        &self,
        approval: &fabric::ApprovalSnapshot,
    ) -> Result<(fabric::CodingJobId, PathBuf), ApplyCoordinationError> {
        let job_id = approval
            .subject
            .job_id
            .ok_or_else(|| ApplyCoordinationError::Evidence("approval has no coding job".into()))?;
        let store = self.store.lock().unwrap();
        let coding = store
            .load_coding_job(job_id)
            .map_err(|error| ApplyCoordinationError::Evidence(error.to_string()))?
            .ok_or_else(|| ApplyCoordinationError::Evidence("coding job not found".into()))?;
        let base = self
            .config
            .worktree_base
            .canonicalize()
            .map_err(|error| ApplyCoordinationError::Cleanup(error.to_string()))?;
        let candidate = base.join(coding.worktree_ref);
        let path = if candidate.exists() {
            candidate
                .canonicalize()
                .map_err(|error| ApplyCoordinationError::Cleanup(error.to_string()))?
        } else {
            candidate
        };
        if !path.starts_with(&base) {
            return Err(ApplyCoordinationError::Cleanup(
                "worktree escaped managed base".into(),
            ));
        }
        Ok((job_id, path))
    }
}

struct RepositoryAuthorizer {
    repository: Arc<Mutex<ApprovalRepository>>,
}

impl ApplyAuthorizer for RepositoryAuthorizer {
    fn authorization(&self, approval_id: ApprovalId) -> Result<Option<ApplyAuthorization>, String> {
        let approval = self
            .repository
            .lock()
            .map_err(|_| "approval repository lock poisoned".to_string())?
            .get(approval_id)
            .map_err(|error| error.to_string())?;
        approval
            .map(|value| {
                let expected_head = attribute(&value, "base_commit")?;
                let diff_sha256 = attribute(&value, "diff_sha256")?;
                let verification_sha256 = attribute(&value, "verification_sha256")?;
                Ok(ApplyAuthorization {
                    approval_id: value.id,
                    status: value.status,
                    subject_hash: value.subject_hash,
                    expected_head,
                    diff_sha256,
                    verification_sha256,
                    allowed_paths: value.subject.allowed_scope,
                })
            })
            .transpose()
    }
}

fn attribute(approval: &fabric::ApprovalSnapshot, name: &str) -> Result<String, String> {
    approval
        .subject
        .attributes
        .get(name)
        .cloned()
        .ok_or_else(|| format!("approval missing {name}"))
}

fn approval_error(error: ApprovalRepositoryError) -> ApplyCoordinationError {
    ApplyCoordinationError::Approval(error.to_string())
}

fn bound(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}

struct TemporaryArtifact {
    path: PathBuf,
}

impl TemporaryArtifact {
    fn write(bytes: &[u8]) -> Result<Self, ApplyError> {
        let path = std::env::temp_dir().join(format!(
            "aletheon-verification-{}.json",
            uuid::Uuid::new_v4()
        ));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
            .map_err(|error| ApplyError::Artifact(error.to_string()))?;
        file.write_all(bytes)
            .and_then(|_| file.sync_all())
            .map_err(|error| ApplyError::Artifact(error.to_string()))?;
        Ok(Self { path })
    }
}

impl Drop for TemporaryArtifact {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.path);
    }
}
