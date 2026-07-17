//! Admission controller trait — Phase 5A.
//!
//! The admission controller is the gatekeeper for all side-effecting capability
//! invocations. Every tool execution that touches resources must first pass
//! through `admit()`, produce an `ExecutionPermit`, and later settle via
//! `settle()`.

use crate::types::admission::{
    AdmissionError, AdmissionRequest, BudgetRequest, BudgetReservationId, BudgetReservationReceipt,
    BudgetScope, BudgetScopeId, BudgetScopeKind, ExecutionPermit, PermitId, RevokeReason,
    UsageReport,
};
use async_trait::async_trait;

/// Central admission control point for capability execution.
///
/// # Lifecycle
///
/// ```text
/// admit() → ExecutionPermit
/// → execute capability with permit
/// → settle(permit, usage) → final audit
/// ```
///
/// Permits are single-use: `settle()` MUST reject a permit that has already
/// been settled. `revoke()` cancels a permit that hasn't been settled yet.
#[async_trait]
pub trait AdmissionController: Send + Sync {
    /// Evaluate an admission request and, if approved, issue an execution permit.
    ///
    /// This reserves budget/quota/lease slots atomically — partial approval
    /// is not allowed. If any dimension fails, the entire request is denied.
    async fn admit(&self, request: AdmissionRequest) -> Result<ExecutionPermit, AdmissionError>;

    /// Settle a permit after execution completes.
    ///
    /// Consumes the budget/quota/lease reservations and writes an audit record.
    /// Returns `AdmissionError::AlreadySettled` if this permit was already
    /// settled or revoked.
    async fn settle(&self, permit_id: PermitId, usage: UsageReport) -> Result<(), AdmissionError>;

    /// Revoke an active (not-yet-settled) permit.
    ///
    /// Frees reserved budget/quota/lease without charging the operation.
    /// Idempotent: revoking an already-settled permit is a no-op.
    async fn revoke(&self, permit_id: PermitId, reason: RevokeReason)
        -> Result<(), AdmissionError>;
}

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
}
