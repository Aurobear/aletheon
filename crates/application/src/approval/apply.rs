//! Restart-safe approved-apply orchestration.
//!
//! Application owns approval-state decisions and terminal settlement order.
//! Concrete repositories, process execution, worktree I/O and projections are
//! injected through consumer-owned ports.

use super::{ApprovalApplyClaim, ApprovalApplyReceipt};
use async_trait::async_trait;
use contracts::{
    ApprovalId, ApprovalSnapshot, ApprovalStatus, Clock, GoalId, OperationId, ProcessId,
};
use std::sync::Arc;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ApprovedApplyResult {
    pub head: String,
    pub diff_sha256: String,
    pub changed_paths: Vec<std::path::PathBuf>,
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

#[derive(Debug, thiserror::Error)]
pub enum ApplyCoordinationError {
    #[error("approval: {0}")]
    Approval(String),
    #[error("goal: {0}")]
    Goal(String),
    #[error("evidence: {0}")]
    Evidence(String),
    #[error("operation: {0}")]
    Operation(String),
    #[error("apply: {0}")]
    Apply(String),
    #[error("cleanup: {0}")]
    Cleanup(String),
}

pub trait ApprovalApplyRepositoryPort: Send + Sync {
    fn approval(&self, id: ApprovalId) -> Result<Option<ApprovalSnapshot>, String>;
    fn receipt(&self, id: ApprovalId) -> Result<Option<ApprovalApplyReceipt>, String>;
    fn claim(
        &self,
        id: ApprovalId,
        operation_id: OperationId,
        now_ms: i64,
    ) -> Result<ApprovalApplyClaim, String>;
    fn finish(&self, receipt: &ApprovalApplyReceipt) -> Result<(), String>;
}

pub trait ApprovedApplyGoalPort: Send + Sync {
    fn settle_rejected(
        &self,
        approval: &ApprovalSnapshot,
        revision_requested: bool,
    ) -> Result<(), String>;
    fn ensure_running(&self, goal_id: GoalId) -> Result<(), String>;
    fn settle_terminal(&self, receipt: &ApprovalApplyReceipt) -> Result<(), String>;
    fn reconcile_terminal(&self, receipt: &ApprovalApplyReceipt) -> Result<(), String>;
}

#[async_trait]
pub trait ApprovedApplyExecutorPort: Send + Sync {
    async fn execute(
        &self,
        approval: &ApprovalSnapshot,
        cancel: CancellationToken,
    ) -> Result<ApprovedApplyResult, String>;
    async fn cleanup(&self, approval: &ApprovalSnapshot) -> Result<(), String>;
}

#[async_trait]
pub trait ApprovalOperationPort: Send + Sync {
    async fn register_and_start(
        &self,
        operation_id: OperationId,
        owner: ProcessId,
    ) -> Result<bool, String>;
    async fn succeed(&self, operation_id: OperationId) -> Result<(), String>;
    async fn fail(&self, operation_id: OperationId, error: String) -> Result<(), String>;
}

#[async_trait]
pub trait ApprovalSummaryPort: Send + Sync {
    async fn record(&self, approval_id: ApprovalId) -> Result<(), String>;
}

pub struct ApplyCoordinator {
    repository: Arc<dyn ApprovalApplyRepositoryPort>,
    goals: Arc<dyn ApprovedApplyGoalPort>,
    executor: Arc<dyn ApprovedApplyExecutorPort>,
    operations: Arc<dyn ApprovalOperationPort>,
    summaries: Arc<dyn ApprovalSummaryPort>,
    clock: Arc<dyn Clock>,
}

impl ApplyCoordinator {
    pub fn new(
        repository: Arc<dyn ApprovalApplyRepositoryPort>,
        goals: Arc<dyn ApprovedApplyGoalPort>,
        executor: Arc<dyn ApprovedApplyExecutorPort>,
        operations: Arc<dyn ApprovalOperationPort>,
        summaries: Arc<dyn ApprovalSummaryPort>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            repository,
            goals,
            executor,
            operations,
            summaries,
            clock,
        }
    }

    pub async fn coordinate(
        &self,
        approval_id: ApprovalId,
        owner_process: ProcessId,
        cancel: CancellationToken,
    ) -> Result<ApplyCoordinationOutcome, ApplyCoordinationError> {
        let approval = self
            .repository
            .approval(approval_id)
            .map_err(ApplyCoordinationError::Approval)?
            .ok_or_else(|| ApplyCoordinationError::Approval("approval not found".into()))?;
        if let Some(receipt) = self
            .repository
            .receipt(approval_id)
            .map_err(ApplyCoordinationError::Approval)?
        {
            self.goals
                .reconcile_terminal(&receipt)
                .map_err(ApplyCoordinationError::Goal)?;
            if receipt.success {
                self.executor
                    .cleanup(&approval)
                    .await
                    .map_err(ApplyCoordinationError::Cleanup)?;
            }
            self.summaries
                .record(approval_id)
                .await
                .map_err(ApplyCoordinationError::Evidence)?;
            return Ok(ApplyCoordinationOutcome::Recovered(receipt));
        }
        match approval.status {
            ApprovalStatus::Pending => return Ok(ApplyCoordinationOutcome::AwaitingDecision),
            ApprovalStatus::Rejected | ApprovalStatus::Expired => {
                let revision_requested = approval
                    .resolution
                    .as_ref()
                    .and_then(|value| value.reason.as_deref())
                    == Some("owner requested revision");
                self.goals
                    .settle_rejected(&approval, revision_requested)
                    .map_err(ApplyCoordinationError::Goal)?;
                self.summaries
                    .record(approval_id)
                    .await
                    .map_err(ApplyCoordinationError::Evidence)?;
                return Ok(ApplyCoordinationOutcome::Rejected {
                    goal_id: approval.subject.goal_id,
                    revision_requested,
                });
            }
            ApprovalStatus::Consumed => {
                return Err(ApplyCoordinationError::Approval(
                    "consumed approval has no apply receipt".into(),
                ))
            }
            ApprovalStatus::Approved => {}
        }

        let proposed = OperationId::new();
        let claim = self
            .repository
            .claim(approval_id, proposed, self.clock.wall_now().0)
            .map_err(ApplyCoordinationError::Approval)?;
        let operation_id = match claim {
            ApprovalApplyClaim::Claimed(value) | ApprovalApplyClaim::Existing(value) => {
                value.operation_id
            }
        };
        if !self
            .operations
            .register_and_start(operation_id, owner_process)
            .await
            .map_err(ApplyCoordinationError::Operation)?
        {
            return Ok(ApplyCoordinationOutcome::DuplicateInProgress { operation_id });
        }
        self.goals
            .ensure_running(approval.subject.goal_id)
            .map_err(ApplyCoordinationError::Goal)?;
        let applied = self.executor.execute(&approval, cancel).await;
        let receipt = match applied {
            Ok(value) => ApprovalApplyReceipt {
                approval_id,
                operation_id,
                goal_id: approval.subject.goal_id,
                success: true,
                applied_head: Some(value.head),
                diff_sha256: value.diff_sha256,
                changed_paths: value.changed_paths,
                error: None,
                finished_at_ms: self.clock.wall_now().0,
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
                error: Some(bound(&error, 2048)),
                finished_at_ms: self.clock.wall_now().0,
            },
        };
        self.repository
            .finish(&receipt)
            .map_err(ApplyCoordinationError::Approval)?;
        if receipt.success {
            self.operations
                .succeed(operation_id)
                .await
                .map_err(ApplyCoordinationError::Operation)?;
        } else {
            self.operations
                .fail(operation_id, receipt.error.clone().unwrap_or_default())
                .await
                .map_err(ApplyCoordinationError::Operation)?;
        }
        self.goals
            .settle_terminal(&receipt)
            .map_err(ApplyCoordinationError::Goal)?;
        self.summaries
            .record(approval_id)
            .await
            .map_err(ApplyCoordinationError::Evidence)?;
        if receipt.success {
            self.executor
                .cleanup(&approval)
                .await
                .map_err(ApplyCoordinationError::Cleanup)?;
            Ok(ApplyCoordinationOutcome::Applied(receipt))
        } else {
            Ok(ApplyCoordinationOutcome::Failed(receipt))
        }
    }
}

fn bound(value: &str, max: usize) -> String {
    value.chars().take(max).collect()
}
