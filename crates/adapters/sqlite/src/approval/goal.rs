//! SQLite adapter for Goal approval requests.

use crate::approval_repository::{ApprovalCreate, ApprovalRepository};
use application::approval::{ApprovalCreateCommand, GoalApprovalPort};
use contracts::ApprovalSnapshot;
use std::sync::{Arc, Mutex};

pub struct SqliteGoalApprovalPort {
    repository: Arc<Mutex<ApprovalRepository>>,
}

impl SqliteGoalApprovalPort {
    pub fn new(repository: Arc<Mutex<ApprovalRepository>>) -> Self {
        Self { repository }
    }
}

impl GoalApprovalPort for SqliteGoalApprovalPort {
    fn create(&self, command: ApprovalCreateCommand) -> Result<ApprovalSnapshot, String> {
        self.repository
            .lock()
            .unwrap_or_else(|error| error.into_inner())
            .create(ApprovalCreate {
                subject: command.subject,
                risk: command.risk,
                summary: command.summary,
                artifacts: command.artifacts,
                created_at_ms: command.created_at_ms,
                expires_at_ms: command.expires_at_ms,
            })
            .map_err(|error| error.to_string())
    }
}
