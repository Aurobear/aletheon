//! Request-safe durable approval use cases.

use std::sync::{Arc, Mutex};

use ::contracts::{ApprovalCategory, ApprovalId, ApprovalSnapshot, Clock, PrincipalId, ProcessId};
use async_trait::async_trait;
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use adapters_sqlite::approval_repository::{
    ApprovalDecision, ApprovalRepository, ApprovalRepositoryError, ApprovalResolutionContext,
};
use crate::wiring::application::approval::ApplyCoordinator;

const MAX_PENDING_APPROVALS: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ApprovalContext {
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

#[derive(Debug, Error)]
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
    repository: Arc<Mutex<ApprovalRepository>>,
    apply: Option<Arc<ApplyCoordinator>>,
    dasein: Option<Arc<dyn DaseinMutationCoordinator>>,
    clock: Arc<dyn Clock>,
    owner_process: Arc<tokio::sync::Mutex<Option<ProcessId>>>,
}

impl ApprovalService {
    pub fn new(
        repository: Arc<Mutex<ApprovalRepository>>,
        apply: Option<Arc<ApplyCoordinator>>,
        clock: Arc<dyn Clock>,
        owner_process: Arc<tokio::sync::Mutex<Option<ProcessId>>>,
    ) -> Self {
        Self {
            repository,
            apply,
            dasein: None,
            clock,
            owner_process,
        }
    }

    pub fn with_dasein_coordinator(
        mut self,
        coordinator: Arc<dyn DaseinMutationCoordinator>,
    ) -> Self {
        self.dasein = Some(coordinator);
        self
    }

    fn map_repository_error(error: ApprovalRepositoryError) -> ApprovalServiceError {
        match error {
            ApprovalRepositoryError::NotFound(_) => ApprovalServiceError::NotFound,
            ApprovalRepositoryError::WrongOwner | ApprovalRepositoryError::ChannelDenied => {
                ApprovalServiceError::Forbidden(error.to_string())
            }
            ApprovalRepositoryError::AlreadyDecided
            | ApprovalRepositoryError::VersionConflict { .. }
            | ApprovalRepositoryError::ActiveSubjectConflict => {
                ApprovalServiceError::Conflict(error.to_string())
            }
            _ => ApprovalServiceError::Store(error.to_string()),
        }
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
            .lock()
            .unwrap()
            .list_pending(&context.principal_id, self.clock.wall_now().0)
            .map_err(Self::map_repository_error)?;
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
            .lock()
            .unwrap()
            .get(approval_id)
            .map_err(Self::map_repository_error)?
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
        // A Metacog apply may have failed after the human decision became
        // durable. Repeating the same approve request resumes idempotent apply
        // instead of asking the operator to create a second approval.
        if matches!(request.decision, ApprovalDecision::Approve) {
            let existing = {
                self.repository
                    .lock()
                    .unwrap()
                    .get(request.approval_id)
                    .map_err(Self::map_repository_error)?
            };
            if let Some(existing) = existing {
                if existing.category == ApprovalCategory::DaseinModification
                    && existing.status == ::contracts::ApprovalStatus::Approved
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
        let approval = self
            .repository
            .lock()
            .unwrap()
            .resolve(
                request.approval_id,
                request.version,
                &ApprovalResolutionContext {
                    principal_id: request.context.principal_id,
                    channel: request.context.channel,
                },
                request.decision,
                self.clock.wall_now().0,
            )
            .map_err(Self::map_repository_error)?;

        if approval.category == ApprovalCategory::ApplyCode {
            let coordinator = self.apply.as_ref().ok_or_else(|| {
                ApprovalServiceError::RuntimeUnavailable(
                    "approved apply runtime is unavailable".into(),
                )
            })?;
            let owner = self
                .owner_process
                .lock()
                .await
                .unwrap_or_else(ProcessId::new);
            coordinator
                .coordinate(approval.id, owner, CancellationToken::new())
                .await
                .map_err(|error| ApprovalServiceError::Store(error.to_string()))?;
        } else if approval.category == ApprovalCategory::DaseinModification
            && matches!(approval.status, ::contracts::ApprovalStatus::Approved)
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

#[async_trait]
pub trait DaseinMutationCoordinator: Send + Sync {
    async fn apply(&self, approval: ApprovalSnapshot) -> Result<(), ApprovalServiceError>;
}
