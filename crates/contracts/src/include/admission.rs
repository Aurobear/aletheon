//! Shared budget reservation port retained for Runtime/Kernel dependency neutrality.

use crate::types::admission::{
    AdmissionError, BudgetRequest, BudgetReservationId, BudgetReservationReceipt, BudgetScope,
    BudgetScopeId, BudgetScopeKind, BudgetTransferReceipt, UsageReport,
};
use async_trait::async_trait;

/// Hierarchical monetary budget controller.
///
/// This is the boundary contract the application layer depends on so it never
/// binds to a concrete `InMemoryBudgetController` (coupling-optimization plan:
/// no concrete kernel type crosses the domain boundary). The kernel provides
/// the implementation; `KernelRuntime::budget_controller()` returns
/// `Arc<dyn BudgetController>`.
#[async_trait]
pub trait BudgetController: Send + Sync {
    /// Create a root budget scope (one tree per rollout) and return its id.
    async fn create_root(&self, owner: String, limit: BudgetRequest) -> BudgetScopeId;

    /// Atomically allocate a child scope from its direct parent, holding
    /// capacity in the parent until the child is settled or revoked.
    async fn reserve_child(
        &self,
        parent: BudgetScopeId,
        kind: BudgetScopeKind,
        owner: String,
        request: BudgetRequest,
    ) -> Result<BudgetReservationReceipt, AdmissionError>;

    /// Close a leaf reservation and return its unused capacity to the parent,
    /// charging the reported usage.
    async fn settle_reservation(
        &self,
        reservation: BudgetReservationId,
        usage: &UsageReport,
    ) -> Result<(), AdmissionError>;

    /// Atomically charge child usage, close the child reservation, and move
    /// its unused capacity into a live parent reservation. Replays return the
    /// same immutable receipt.
    async fn transfer_remaining_reservation(
        &self,
        child: BudgetReservationId,
        parent: BudgetReservationId,
        usage: &UsageReport,
    ) -> Result<BudgetTransferReceipt, AdmissionError>;

    /// Close a reservation, returning unused capacity to its parent. Repeating
    /// the call on an already-closed reservation is a successful no-op.
    async fn revoke_reservation(
        &self,
        reservation: BudgetReservationId,
    ) -> Result<(), AdmissionError>;

    /// Read a budget scope's current view, if it exists.
    async fn scope(&self, id: BudgetScopeId) -> Option<BudgetScope>;

    /// Count the reservations currently held open (for lifecycle inspection).
    async fn active_reservation_count(&self) -> usize;

    /// Durable lookup used by crash recovery; includes closed reservations so
    /// a transfer receipt can be replayed after restart.
    async fn reservation_for_owner(&self, owner: &str) -> Option<BudgetReservationId>;

    /// Return the immutable transfer receipt when the child reservation was
    /// already transferred before a crash.
    async fn transfer_for_child(&self, child: BudgetReservationId)
        -> Option<BudgetTransferReceipt>;
}
