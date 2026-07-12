//! Admission controller trait — Phase 5A.
//!
//! The admission controller is the gatekeeper for all side-effecting capability
//! invocations. Every tool execution that touches resources must first pass
//! through `admit()`, produce an `ExecutionPermit`, and later settle via
//! `settle()`.

use crate::types::admission::{
    AdmissionError, AdmissionRequest, ExecutionPermit, PermitId, RevokeReason, UsageReport,
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
