//! Restart-safe, one-time coordination of approved coding patches.

use crate::adapters::memory_projection::{MemoryProjection, ProjectionStatus};
use ::contracts::{
    ApprovalId, Clock, GoalId, GoalState, GoalWaitReason, OperationId, OperationKind,
    OperationRequest, ProcessId,
};
use adapters_sqlite::approval_repository::{
    ApprovalApplyClaim, ApprovalApplyReceipt, ApprovalRepository, ApprovalRepositoryError,
};
use adapters_sqlite::goal::ObjectiveStore;
use application::approval::{ManagedWorktreeCleaner, TemporaryArtifactStore};
use corpus::tools::subagent::{
    ApplyAuthorization, ApplyAuthorizer, ApplyError, ApplySpec, ControlledApply,
};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;
use tokio_util::sync::CancellationToken;
use tracing::warn;

#[derive(Debug, Clone)]
pub struct ApplyCoordinatorConfig {
    pub worktree_base: PathBuf,
    pub timeout: Duration,
}

pub use application::approval::{ApplyCoordinationError, ApplyCoordinationOutcome};

#[derive(Clone)]
pub struct ApplyCoordinator {
    store: Arc<Mutex<ObjectiveStore>>,
    approvals: Arc<Mutex<ApprovalRepository>>,
    kernel: Arc<kernel::KernelRuntime>,
    clock: Arc<dyn Clock>,
    config: ApplyCoordinatorConfig,
    cleaner: Arc<dyn ManagedWorktreeCleaner>,
    temporary_artifacts: Arc<dyn TemporaryArtifactStore>,
    memory_projection: Option<MemoryProjection>,
}

pub struct ApprovedCodeSettlement {
    coordinator: Arc<ApplyCoordinator>,
    owner_process: Arc<tokio::sync::Mutex<Option<ProcessId>>>,
}

impl ApprovedCodeSettlement {
    pub fn new(
        coordinator: Arc<ApplyCoordinator>,
        owner_process: Arc<tokio::sync::Mutex<Option<ProcessId>>>,
    ) -> Self {
        Self {
            coordinator,
            owner_process,
        }
    }
}

#[async_trait::async_trait]
impl application::approval::ApprovedCodeSettlementPort for ApprovedCodeSettlement {
    async fn apply(
        &self,
        approval: ::contracts::ApprovalSnapshot,
    ) -> Result<(), application::approval::ApprovalServiceError> {
        let owner = self
            .owner_process
            .lock()
            .await
            .unwrap_or_else(ProcessId::new);
        self.coordinator
            .coordinate(approval.id, owner, CancellationToken::new())
            .await
            .map(|_| ())
            .map_err(|error| application::approval::ApprovalServiceError::Store(error.to_string()))
    }
}

impl ApplyCoordinator {
    pub fn new(
        store: Arc<Mutex<ObjectiveStore>>,
        approvals: Arc<Mutex<ApprovalRepository>>,
        kernel: Arc<kernel::KernelRuntime>,
        clock: Arc<dyn Clock>,
        config: ApplyCoordinatorConfig,
        cleaner: Arc<dyn ManagedWorktreeCleaner>,
        temporary_artifacts: Arc<dyn TemporaryArtifactStore>,
    ) -> Result<Self, ApplyCoordinationError> {
        if config.timeout.is_zero() {
            return Err(ApplyCoordinationError::Apply(
                "timeout must be positive".into(),
            ));
        }
        Ok(Self {
            store,
            approvals,
            kernel,
            clock,
            config,
            cleaner,
            temporary_artifacts,
            memory_projection: None,
        })
    }

    pub fn with_memory_projection(mut self, projection: MemoryProjection) -> Self {
        self.memory_projection = Some(projection);
        self
    }

    pub async fn coordinate(
        &self,
        approval_id: ApprovalId,
        owner_process: ProcessId,
        cancel: CancellationToken,
    ) -> Result<ApplyCoordinationOutcome, ApplyCoordinationError> {
        let adapter = Arc::new(self.clone());
        application::approval::ApplyCoordinator::new(
            adapter.clone(),
            adapter.clone(),
            adapter.clone(),
            adapter.clone(),
            adapter,
            self.clock.clone(),
        )
        .coordinate(approval_id, owner_process, cancel)
        .await
    }

    fn transition_rejected(
        &self,
        approval: &::contracts::ApprovalSnapshot,
    ) -> Result<ApplyCoordinationOutcome, ApplyCoordinationError> {
        let revision_requested = approval
            .resolution
            .as_ref()
            .and_then(|value| value.reason.as_deref())
            == Some("owner requested revision");
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
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
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
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
        approval: &::contracts::ApprovalSnapshot,
        cancel: CancellationToken,
    ) -> Result<corpus::tools::subagent::ApplyOutcome, ApplyError> {
        let job_id = approval
            .subject
            .job_id
            .ok_or_else(|| ApplyError::Unauthorized("approval has no coding job".into()))?;
        if approval.category != ::contracts::ApprovalCategory::ApplyCode
            || approval.subject.apply_target.as_deref() != Some(Path::new("."))
        {
            return Err(ApplyError::Unauthorized(
                "approval is not a repository-root code apply".into(),
            ));
        }
        let (coding, verification, artifact_dir) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let coding = store
                .load_coding_job(job_id)
                .map_err(|error| ApplyError::Artifact(error.to_string()))?
                .ok_or_else(|| ApplyError::Artifact("coding job not found".into()))?;
            let verification = store
                .load_verification_report(job_id)
                .map_err(|error| ApplyError::Artifact(error.to_string()))?
                .ok_or_else(|| ApplyError::Artifact("verification report not found".into()))?;
            (coding, verification, store.artifact_root().to_owned())
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
        let verification_file = self
            .temporary_artifacts
            .write("aletheon-verification", &verification_bytes)
            .map_err(ApplyError::Artifact)?;
        let authorizer: Arc<dyn ApplyAuthorizer> = Arc::new(RepositoryAuthorizer {
            repository: self.approvals.clone(),
        });
        let applier = ControlledApply::new(authorizer)?;
        applier
            .apply(
                ApplySpec {
                    repository_root: approved_repository_root(approval)?,
                    expected_head: coding.report.base_commit.clone(),
                    diff_artifact: artifact_dir.join(&coding.diff_artifact_ref),
                    diff_sha256: coding.diff_sha256,
                    verification_artifact: verification_file.path().to_path_buf(),
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
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
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

    fn worktree_for(
        &self,
        approval: &::contracts::ApprovalSnapshot,
    ) -> Result<(::contracts::CodingJobId, PathBuf, PathBuf), ApplyCoordinationError> {
        let job_id = approval
            .subject
            .job_id
            .ok_or_else(|| ApplyCoordinationError::Evidence("approval has no coding job".into()))?;
        let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
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
        let repository_root = approved_repository_root(approval)
            .map_err(|error| ApplyCoordinationError::Cleanup(error.to_string()))?;
        Ok((job_id, repository_root, path))
    }

    async fn record_summary(&self, approval_id: ApprovalId) -> Result<(), ApplyCoordinationError> {
        let (approval, receipt) = {
            let approvals = self.approvals.lock().unwrap_or_else(|e| e.into_inner());
            let approval = approvals
                .get(approval_id)
                .map_err(approval_error)?
                .ok_or_else(|| ApplyCoordinationError::Approval("approval not found".into()))?;
            let receipt = approvals
                .apply_receipt(approval_id)
                .map_err(approval_error)?;
            (approval, receipt)
        };
        let (persisted, evidence) = {
            let store = self.store.lock().unwrap_or_else(|e| e.into_inner());
            let summary = adapters_sqlite::goal::GoalCompletionSummary::build(
                &store,
                &approval,
                receipt.as_ref(),
                self.clock.wall_now().0,
            )
            .map_err(|error| ApplyCoordinationError::Evidence(error.to_string()))?;
            let persisted = store
                .persist_goal_completion_summary(&summary)
                .map_err(|error| ApplyCoordinationError::Evidence(error.to_string()))?;
            let evidence = store
                .goal_projection_evidence(persisted.goal_id)
                .map_err(|error| ApplyCoordinationError::Evidence(error.to_string()))?;
            (persisted, evidence)
        };
        if let Some(projection) = &self.memory_projection {
            if projection
                .project_goal_summary(
                    &persisted,
                    &evidence,
                    mnemosyne::MemorySensitivity::Internal,
                )
                .await
                == ProjectionStatus::Degraded
            {
                warn!(
                    goal_id = persisted.goal_id.0,
                    "best-effort goal summary memory projection is degraded"
                );
            }
        }
        Ok(())
    }
}

impl application::approval::ApprovalApplyRepositoryPort for ApplyCoordinator {
    fn approval(&self, id: ApprovalId) -> Result<Option<::contracts::ApprovalSnapshot>, String> {
        self.approvals
            .lock()
            .map_err(|_| "approval repository lock poisoned".to_string())?
            .get(id)
            .map_err(|error| error.to_string())
    }

    fn receipt(&self, id: ApprovalId) -> Result<Option<ApprovalApplyReceipt>, String> {
        self.approvals
            .lock()
            .map_err(|_| "approval repository lock poisoned".to_string())?
            .apply_receipt(id)
            .map_err(|error| error.to_string())
    }

    fn claim(
        &self,
        id: ApprovalId,
        operation_id: OperationId,
        now_ms: i64,
    ) -> Result<ApprovalApplyClaim, String> {
        self.approvals
            .lock()
            .map_err(|_| "approval repository lock poisoned".to_string())?
            .claim_apply(id, operation_id, now_ms)
            .map_err(|error| error.to_string())
    }

    fn finish(&self, receipt: &ApprovalApplyReceipt) -> Result<(), String> {
        self.approvals
            .lock()
            .map_err(|_| "approval repository lock poisoned".to_string())?
            .finish_apply(receipt)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }
}

impl application::approval::ApprovedApplyGoalPort for ApplyCoordinator {
    fn settle_rejected(
        &self,
        approval: &::contracts::ApprovalSnapshot,
        _revision_requested: bool,
    ) -> Result<(), String> {
        self.transition_rejected(approval)
            .map(|_| ())
            .map_err(|error| error.to_string())
    }

    fn ensure_running(&self, goal_id: GoalId) -> Result<(), String> {
        self.ensure_goal_running(goal_id)
            .map_err(|error| error.to_string())
    }

    fn settle_terminal(&self, receipt: &ApprovalApplyReceipt) -> Result<(), String> {
        let target = if receipt.success {
            GoalState::Completed
        } else {
            GoalState::Blocked
        };
        self.transition_terminal(receipt.goal_id, target, receipt)
            .map_err(|error| error.to_string())
    }

    fn reconcile_terminal(&self, receipt: &ApprovalApplyReceipt) -> Result<(), String> {
        let target = if receipt.success {
            GoalState::Completed
        } else {
            GoalState::Blocked
        };
        let current = self
            .store
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get_goal(receipt.goal_id)
            .map_err(|error| error.to_string())?
            .ok_or_else(|| "goal not found".to_string())?;
        if current.state != target {
            self.ensure_goal_running(receipt.goal_id)
                .map_err(|error| error.to_string())?;
            self.transition_terminal(receipt.goal_id, target, receipt)
                .map_err(|error| error.to_string())?;
        }
        Ok(())
    }
}

#[async_trait::async_trait]
impl application::approval::ApprovedApplyExecutorPort for ApplyCoordinator {
    async fn execute(
        &self,
        approval: &::contracts::ApprovalSnapshot,
        cancel: CancellationToken,
    ) -> Result<application::approval::ApprovedApplyResult, String> {
        self.execute_approved(approval, cancel)
            .await
            .map(|outcome| application::approval::ApprovedApplyResult {
                head: outcome.head,
                diff_sha256: outcome.diff_sha256,
                changed_paths: outcome.changed_paths,
            })
            .map_err(|error| error.to_string())
    }

    async fn cleanup(&self, approval: &::contracts::ApprovalSnapshot) -> Result<(), String> {
        let (job_id, repository_root, worktree) = self
            .worktree_for(approval)
            .map_err(|error| error.to_string())?;
        self.cleaner
            .cleanup(job_id, &repository_root, &worktree)
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait::async_trait]
impl application::approval::ApprovalOperationPort for ApplyCoordinator {
    async fn register_and_start(
        &self,
        operation_id: OperationId,
        owner: ProcessId,
    ) -> Result<bool, String> {
        let request = OperationRequest {
            owner,
            parent: None,
            kind: OperationKind::ApprovedApply,
            deadline: None,
        };
        if self
            .kernel
            .submit_operation_with_id(operation_id, request)
            .await
            .is_err()
        {
            return Ok(false);
        }
        self.kernel
            .start_operation(operation_id)
            .await
            .map_err(|error| error.to_string())?;
        Ok(true)
    }

    async fn succeed(&self, operation_id: OperationId) -> Result<(), String> {
        self.kernel
            .succeed_operation(operation_id)
            .await
            .map_err(|error| error.to_string())
    }

    async fn fail(&self, operation_id: OperationId, error: String) -> Result<(), String> {
        self.kernel
            .fail_operation(operation_id, error)
            .await
            .map_err(|error| error.to_string())
    }
}

#[async_trait::async_trait]
impl application::approval::ApprovalSummaryPort for ApplyCoordinator {
    async fn record(&self, approval_id: ApprovalId) -> Result<(), String> {
        self.record_summary(approval_id)
            .await
            .map_err(|error| error.to_string())
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

fn attribute(approval: &::contracts::ApprovalSnapshot, name: &str) -> Result<String, String> {
    approval
        .subject
        .attributes
        .get(name)
        .cloned()
        .ok_or_else(|| format!("approval missing {name}"))
}

fn approved_repository_root(
    approval: &::contracts::ApprovalSnapshot,
) -> Result<PathBuf, ApplyError> {
    let raw = approval
        .subject
        .attributes
        .get("repository_root")
        .ok_or_else(|| ApplyError::Unauthorized("approval missing repository_root".into()))?;
    let path = PathBuf::from(raw)
        .canonicalize()
        .map_err(|error| ApplyError::Unauthorized(format!("repository_root: {error}")))?;
    if !path.is_absolute() || !path.join(".git").exists() {
        return Err(ApplyError::Unauthorized(
            "approved repository_root is not a git worktree".into(),
        ));
    }
    Ok(path)
}

fn approval_error(error: ApprovalRepositoryError) -> ApplyCoordinationError {
    ApplyCoordinationError::Approval(error.to_string())
}
