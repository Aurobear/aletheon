//! Port abstractions that decouple the channel layer from concrete
//! executive-side stores.
//!
//! [`ChannelApprovalPort`] lets `dispatcher.rs` and `handlers/approval.rs`
//! depend only on fabric-native vocabulary instead of the concrete
//! the private approval repository (and its
//! `ApprovalDecision`/`ApprovalResolutionContext` types), so the channel
//! layer can eventually move to a separate crate.

use fabric::{ApprovalId, ApprovalSnapshot, PrincipalId};

/// Channel-facing approval decision. Mirrors
/// the private approval decision type without naming it, so the
/// port stays free of the concrete approval-repository dependency.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ChannelApprovalDecision {
    Approve,
    Reject { reason: Option<String> },
}

/// Durable approval operations needed by the channel dispatcher and the
/// approval-callback handler. Implemented in executive by adapting the
/// concrete `ApprovalRepository`.
pub trait ChannelApprovalPort: Send + Sync {
    fn get(&self, id: ApprovalId) -> anyhow::Result<Option<ApprovalSnapshot>>;

    #[allow(clippy::too_many_arguments)]
    fn resolve(
        &self,
        id: ApprovalId,
        expected_version: u64,
        principal: PrincipalId,
        channel: String,
        decision: ChannelApprovalDecision,
        now_ms: i64,
    ) -> anyhow::Result<ApprovalSnapshot>;

    fn record_delivery_pending(
        &self,
        approval_id: ApprovalId,
        channel: &str,
        conversation_id: &str,
        correlation_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<()>;

    fn record_delivery_sent(
        &self,
        correlation_id: &str,
        provider_message_id: &str,
        now_ms: i64,
    ) -> anyhow::Result<()>;

    fn record_delivery_failed(
        &self,
        correlation_id: &str,
        error: &str,
        now_ms: i64,
    ) -> anyhow::Result<()>;

    fn list_pending(
        &self,
        principal: &PrincipalId,
        now_ms: i64,
    ) -> anyhow::Result<Vec<ApprovalSnapshot>>;
}
