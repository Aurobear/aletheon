//! Request-safe durable approval use cases.

use std::sync::{Arc, Mutex};

use async_trait::async_trait;
use fabric::{ApprovalCategory, ApprovalId, ApprovalSnapshot, Clock, PrincipalId, ProcessId};
use thiserror::Error;
use tokio_util::sync::CancellationToken;

use crate::r#impl::approval::{
    ApplyCoordinator, ApprovalDecision, ApprovalRepository, ApprovalRepositoryError,
    ApprovalResolutionContext,
};

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
            clock,
            owner_process,
        }
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
        }
        Ok(approval)
    }
}
