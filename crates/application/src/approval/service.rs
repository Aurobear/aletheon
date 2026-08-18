//! Request-safe durable approval command/query use cases.

use async_trait::async_trait;
use contracts::{
    ApprovalCategory, ApprovalId, ApprovalSnapshot, ApprovalStatus, Clock, PrincipalId,
};
use std::sync::Arc;

const MAX_PENDING_APPROVALS: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalContext {
    pub principal_id: PrincipalId,
    pub channel: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ApprovalDecision {
    Approve,
    Reject { reason: Option<String> },
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalResolutionContext {
    pub principal_id: PrincipalId,
    pub channel: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ResolveApprovalRequest {
    pub context: ApprovalContext,
    pub approval_id: ApprovalId,
    pub version: u64,
    pub decision: ApprovalDecision,
}

#[derive(Debug, thiserror::Error)]
pub enum ApprovalServiceError {
    #[error("approval not found")]
    NotFound,
    #[error("approval access forbidden: {0}")]
    Forbidden(String),
    #[error("approval conflict: {0}")]
    Conflict(String),
    #[error("approval runtime unavailable: {0}")]
    RuntimeUnavailable(String),
    #[error("approval operation failed: {0}")]
    Store(String),
}

pub trait ApprovalRepositoryPort: Send + Sync {
    fn list_pending(
        &self,
        principal: &PrincipalId,
        now_ms: i64,
    ) -> Result<Vec<ApprovalSnapshot>, ApprovalServiceError>;
    fn get(&self, id: ApprovalId) -> Result<Option<ApprovalSnapshot>, ApprovalServiceError>;
    fn resolve(
        &self,
        id: ApprovalId,
        version: u64,
        context: &ApprovalResolutionContext,
        decision: ApprovalDecision,
        now_ms: i64,
    ) -> Result<ApprovalSnapshot, ApprovalServiceError>;
}

#[async_trait]
pub trait ApprovedCodeSettlementPort: Send + Sync {
    async fn apply(&self, approval: ApprovalSnapshot) -> Result<(), ApprovalServiceError>;
}

#[async_trait]
pub trait DaseinMutationCoordinator: Send + Sync {
    async fn apply(&self, approval: ApprovalSnapshot) -> Result<(), ApprovalServiceError>;
}

#[async_trait]
pub trait ApprovalUseCases: Send + Sync {
    async fn list(
        &self,
        context: ApprovalContext,
    ) -> Result<Vec<ApprovalSnapshot>, ApprovalServiceError>;
    async fn show(
        &self,
        context: ApprovalContext,
        approval_id: ApprovalId,
    ) -> Result<ApprovalSnapshot, ApprovalServiceError>;
    async fn resolve(
        &self,
        request: ResolveApprovalRequest,
    ) -> Result<ApprovalSnapshot, ApprovalServiceError>;
}

pub struct ApprovalService {
    repository: Arc<dyn ApprovalRepositoryPort>,
    code: Option<Arc<dyn ApprovedCodeSettlementPort>>,
    dasein: Option<Arc<dyn DaseinMutationCoordinator>>,
    clock: Arc<dyn Clock>,
}

impl ApprovalService {
    pub fn new(
        repository: Arc<dyn ApprovalRepositoryPort>,
        code: Option<Arc<dyn ApprovedCodeSettlementPort>>,
        clock: Arc<dyn Clock>,
    ) -> Self {
        Self {
            repository,
            code,
            dasein: None,
            clock,
        }
    }

    pub fn with_dasein_coordinator(
        mut self,
        coordinator: Arc<dyn DaseinMutationCoordinator>,
    ) -> Self {
        self.dasein = Some(coordinator);
        self
    }
}

#[async_trait]
impl ApprovalUseCases for ApprovalService {
    async fn list(
        &self,
        context: ApprovalContext,
    ) -> Result<Vec<ApprovalSnapshot>, ApprovalServiceError> {
        let mut approvals = self
            .repository
            .list_pending(&context.principal_id, self.clock.wall_now().0)?;
        approvals.truncate(MAX_PENDING_APPROVALS);
        Ok(approvals)
    }

    async fn show(
        &self,
        context: ApprovalContext,
        approval_id: ApprovalId,
    ) -> Result<ApprovalSnapshot, ApprovalServiceError> {
        let approval = self
            .repository
            .get(approval_id)?
            .ok_or(ApprovalServiceError::NotFound)?;
        if approval.owner_id != context.principal_id {
            return Err(ApprovalServiceError::Forbidden("wrong owner".into()));
        }
        Ok(approval)
    }

    async fn resolve(
        &self,
        request: ResolveApprovalRequest,
    ) -> Result<ApprovalSnapshot, ApprovalServiceError> {
        if matches!(request.decision, ApprovalDecision::Approve) {
            if let Some(existing) = self.repository.get(request.approval_id)? {
                if existing.category == ApprovalCategory::DaseinModification
                    && existing.status == ApprovalStatus::Approved
                    && existing.owner_id == request.context.principal_id
                {
                    self.dasein
                        .as_ref()
                        .ok_or_else(|| {
                            ApprovalServiceError::RuntimeUnavailable(
                                "approved metacognition runtime is unavailable".into(),
                            )
                        })?
                        .apply(existing.clone())
                        .await?;
                    return Ok(existing);
                }
            }
        }
        let approval = self.repository.resolve(
            request.approval_id,
            request.version,
            &ApprovalResolutionContext {
                principal_id: request.context.principal_id,
                channel: request.context.channel,
            },
            request.decision,
            self.clock.wall_now().0,
        )?;
        if approval.category == ApprovalCategory::ApplyCode {
            self.code
                .as_ref()
                .ok_or_else(|| {
                    ApprovalServiceError::RuntimeUnavailable(
                        "approved apply runtime is unavailable".into(),
                    )
                })?
                .apply(approval.clone())
                .await?;
        } else if approval.category == ApprovalCategory::DaseinModification
            && approval.status == ApprovalStatus::Approved
        {
            self.dasein
                .as_ref()
                .ok_or_else(|| {
                    ApprovalServiceError::RuntimeUnavailable(
                        "approved metacognition runtime is unavailable".into(),
                    )
                })?
                .apply(approval.clone())
                .await?;
        }
        Ok(approval)
    }
}
