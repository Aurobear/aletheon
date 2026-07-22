//! Idempotent terminal Agent resource reclamation.

use std::sync::Arc;

use fabric::AgentControlError;

use super::AgentResourceLease;
use super::{AgentResourceLeaseKind, AgentRunRepository};

pub trait AgentWorktreeReclaimer: Send + Sync {
    fn reclaim(&self, lease: &AgentResourceLease) -> anyhow::Result<()>;
}

pub const MAX_CLEANUP_BATCH: usize = 256;

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AgentCleanupReport {
    pub examined: usize,
    pub reclaimed: usize,
    pub retained_unsafe: usize,
    pub failures: usize,
    pub compacted_rows: usize,
}

pub struct AgentCleanupCoordinator {
    repository: Arc<dyn AgentRunRepository>,
    worktrees: Arc<dyn AgentWorktreeReclaimer>,
}

impl AgentCleanupCoordinator {
    pub fn new(
        repository: Arc<dyn AgentRunRepository>,
        worktrees: Arc<dyn AgentWorktreeReclaimer>,
    ) -> Self {
        Self {
            repository,
            worktrees,
        }
    }

    pub async fn reclaim_expired(
        &self,
        now_ms: i64,
    ) -> Result<AgentCleanupReport, AgentControlError> {
        let leases = self
            .repository
            .list_expired_resource_leases(now_ms, MAX_CLEANUP_BATCH)
            .await?;
        let mut report = AgentCleanupReport {
            examined: leases.len(),
            ..Default::default()
        };
        for lease in leases {
            let Some(run) = self.repository.get(lease.agent_id).await? else {
                report.retained_unsafe += 1;
                continue;
            };
            if !run.status().is_terminal() {
                report.retained_unsafe += 1;
                continue;
            }
            let reclaimed = match lease.kind {
                AgentResourceLeaseKind::Admission
                | AgentResourceLeaseKind::Mailbox
                | AgentResourceLeaseKind::Execution => true,
                AgentResourceLeaseKind::Worktree => match self.worktrees.reclaim(&lease) {
                    Ok(()) => true,
                    Err(_) => {
                        report.failures += 1;
                        false
                    }
                },
            };
            if reclaimed
                && self
                    .repository
                    .delete_resource_lease(&lease.lease_key, &lease.owner)
                    .await?
            {
                report.reclaimed += 1;
            }
        }
        report.compacted_rows = self
            .repository
            .compact_terminal(now_ms, MAX_CLEANUP_BATCH)
            .await?
            .len();
        Ok(report)
    }
}
