//! Admission controller implementations.
//!
//! The production admission controller enforces permit checks, budget/quota,
//! sandbox requirements, and audit. The `AllowAllAdmissionController` is
//! **testing-only** and must never be wired into production paths.

pub mod budget;
pub mod lease;
pub mod production;

pub use budget::InMemoryBudgetController;
pub use lease::InMemoryResourceLeaseManager;
pub use production::ProductionAdmissionController;

use async_trait::async_trait;
use fabric::{
    AdmissionController, AdmissionError, AdmissionRequest, ExecutionPermit, MonoDeadline, PermitId,
    RevokeReason, SandboxDecision, SandboxRequirement, UsageReport,
};
use std::collections::HashSet;
use std::sync::Arc;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Allow-All (testing only)
// ---------------------------------------------------------------------------

/// **TESTING ONLY** admission controller that approves everything.
///
/// This bypasses all security checks — budget, quota, sandbox, scope, and
/// approval. It exists solely so tests can exercise the CapabilityInvoker
/// → admission → execution pipeline without setting up real policies.
///
/// # Production Safety
///
/// The type name includes `Testing` and the module doc explicitly states
/// it must never be wired into production. The `admit()` method logs at
/// `warn` level on every invocation as a runtime signal.
pub struct AllowAllAdmissionController {
    clock: Arc<dyn fabric::Clock>,
    settled: Mutex<HashSet<PermitId>>,
}

impl std::fmt::Debug for AllowAllAdmissionController {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("AllowAllAdmissionController")
            .finish_non_exhaustive()
    }
}

impl AllowAllAdmissionController {
    pub fn new(clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            clock,
            settled: Mutex::new(HashSet::new()),
        }
    }
}

#[async_trait]
impl AdmissionController for AllowAllAdmissionController {
    async fn admit(&self, request: AdmissionRequest) -> Result<ExecutionPermit, AdmissionError> {
        tracing::warn!(
            operation_id = ?request.operation_id,
            capability = %request.capability.0,
            "AllowAllAdmissionController: approving without checks (TESTING ONLY)",
        );

        let now = self.clock.mono_now();
        let permit = ExecutionPermit {
            id: PermitId::new(),
            operation_id: request.operation_id,
            process_id: request.process_id,
            capability: request.capability,
            granted_scope: request.requested_scope,
            expires_at: MonoDeadline::after(now, 30_000), // 30s default
            sandbox: match request.sandbox {
                SandboxRequirement::NotRequired => SandboxDecision::NotApplicable,
                // Testing: assume sandbox passes.
                SandboxRequirement::Required | SandboxRequirement::RequiredThenPromote => {
                    SandboxDecision::Passed
                }
            },
            budget_reservation: None,
            lease: None,
        };
        Ok(permit)
    }

    async fn settle(&self, permit_id: PermitId, _usage: UsageReport) -> Result<(), AdmissionError> {
        let mut settled = self.settled.lock().await;
        if settled.contains(&permit_id) {
            return Err(AdmissionError::AlreadySettled);
        }
        settled.insert(permit_id);
        Ok(())
    }

    async fn revoke(
        &self,
        _permit_id: PermitId,
        _reason: RevokeReason,
    ) -> Result<(), AdmissionError> {
        // Testing: always succeeds.
        Ok(())
    }
}
