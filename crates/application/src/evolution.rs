//! Persistence port for governed evolution proposal disposition.

use contracts::ApprovalId;

pub trait EvolutionProposalStore: Send + Sync {
    fn pending_approval(&self, mutation_id: uuid::Uuid) -> anyhow::Result<Option<ApprovalId>>;
    fn park_insufficient_evidence(
        &self,
        mutation_id: uuid::Uuid,
        now_ms: i64,
    ) -> anyhow::Result<()>;
    fn record_pending_approval(
        &self,
        mutation_id: uuid::Uuid,
        approval_id: ApprovalId,
        evidence_digest: &str,
        now_ms: i64,
    ) -> anyhow::Result<()>;
}
