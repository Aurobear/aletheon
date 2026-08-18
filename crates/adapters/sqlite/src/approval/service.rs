//! SQLite implementation of the Application approval command/query port.

use crate::approval_repository::{
    ApprovalDecision as SqlDecision, ApprovalRepository, ApprovalRepositoryError,
    ApprovalResolutionContext as SqlContext,
};
use application::approval::{
    ApprovalDecision, ApprovalRepositoryPort, ApprovalResolutionContext, ApprovalServiceError,
};
use contracts::{ApprovalId, ApprovalSnapshot, PrincipalId};
use std::sync::{Arc, Mutex};

pub struct SqliteApprovalRepository {
    repository: Arc<Mutex<ApprovalRepository>>,
}

impl SqliteApprovalRepository {
    pub fn new(repository: Arc<Mutex<ApprovalRepository>>) -> Self {
        Self { repository }
    }
}

fn map(error: ApprovalRepositoryError) -> ApprovalServiceError {
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

impl ApprovalRepositoryPort for SqliteApprovalRepository {
    fn list_pending(
        &self,
        principal: &PrincipalId,
        now_ms: i64,
    ) -> Result<Vec<ApprovalSnapshot>, ApprovalServiceError> {
        self.repository
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .list_pending(principal, now_ms)
            .map_err(map)
    }

    fn get(&self, id: ApprovalId) -> Result<Option<ApprovalSnapshot>, ApprovalServiceError> {
        self.repository
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .get(id)
            .map_err(map)
    }

    fn resolve(
        &self,
        id: ApprovalId,
        version: u64,
        context: &ApprovalResolutionContext,
        decision: ApprovalDecision,
        now_ms: i64,
    ) -> Result<ApprovalSnapshot, ApprovalServiceError> {
        let context = SqlContext {
            principal_id: context.principal_id.clone(),
            channel: context.channel.clone(),
        };
        let decision = match decision {
            ApprovalDecision::Approve => SqlDecision::Approve,
            ApprovalDecision::Reject { reason } => SqlDecision::Reject { reason },
        };
        self.repository
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .resolve(id, version, &context, decision, now_ms)
            .map_err(map)
    }
}
