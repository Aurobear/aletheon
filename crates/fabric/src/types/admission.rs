//! Admission control types — Phase 5A.
//!
//! Every capability invocation that produces side effects, consumes budget,
//! or touches high-risk resources must pass through the admission controller.
//! No `ExecutionPermit` = no execution.

use crate::types::time::MonoDeadline;
use serde::{Deserialize, Serialize};
use uuid::Uuid;

// ---------------------------------------------------------------------------
// Core identifiers
// ---------------------------------------------------------------------------

/// Unique permit identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PermitId(pub Uuid);

impl PermitId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for PermitId {
    fn default() -> Self {
        // The default usage report represents work that never received an
        // execution permit. Real permits must be created explicitly with
        // `PermitId::new()`.
        Self(Uuid::nil())
    }
}

/// Principal requesting the capability.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PrincipalId(pub String);

impl PrincipalId {
    pub fn local_uid(uid: u32) -> Self {
        Self(format!("local-uid:{uid}"))
    }
}

/// Human-readable capability name (e.g. "shell.execute", "memory.write").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct CapabilityId(pub String);

/// Scope limits for a capability invocation.
#[derive(Debug, Clone, PartialEq, Eq, Default, Serialize, Deserialize)]
pub struct CapabilityScope {
    /// Allowed paths (for filesystem operations). Empty = any.
    pub allowed_paths: Vec<String>,
    /// Allowed network targets. Empty = any.
    pub allowed_targets: Vec<String>,
    /// Maximum runtime in monotonic milliseconds.
    pub max_runtime_ms: Option<u64>,
    /// Maximum output bytes.
    pub max_output_bytes: Option<u64>,
}

/// Risk classification for a capability invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub enum RiskLevel {
    /// Read-only, no side effects.
    ReadOnly = 0,
    /// Writes within sandbox only.
    Sandboxed = 1,
    /// Modifies system state.
    SystemModify = 2,
    /// Destructive or irreversible.
    Destructive = 3,
}

// ---------------------------------------------------------------------------
// Sandbox requirement
// ---------------------------------------------------------------------------

/// Whether a sandbox is required for this invocation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxRequirement {
    /// No sandbox needed.
    NotRequired,
    /// Sandbox is mandatory; fail if unavailable.
    Required,
    /// Sandbox must pass before real execution is allowed.
    RequiredThenPromote,
}

/// Sandbox decision in the permit.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum SandboxDecision {
    NotApplicable,
    Required,
    Passed,
    Failed { reason: String },
    Unavailable,
}

// ---------------------------------------------------------------------------
// Budget and quota
// ---------------------------------------------------------------------------

/// Reserved budget identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BudgetReservationId(pub Uuid);

impl BudgetReservationId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Schema version for persisted hierarchical budget scope identifiers.
pub const BUDGET_SCOPE_SCHEMA_VERSION: u16 = 1;

/// Stable identifier for one node in a rollout budget hierarchy.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct BudgetScopeId(pub Uuid);

impl BudgetScopeId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for BudgetScopeId {
    fn default() -> Self {
        Self::new()
    }
}

/// The only valid levels in the monetary budget ownership tree.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BudgetScopeKind {
    Rollout,
    Process,
    Operation,
    Capability,
}

impl BudgetScopeKind {
    pub fn accepts_child(self, child: Self) -> bool {
        matches!(
            (self, child),
            (Self::Rollout, Self::Process)
                | (Self::Process, Self::Operation)
                | (Self::Operation, Self::Capability)
        )
    }
}

/// Public immutable view of a budget scope.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetScope {
    pub schema_version: u16,
    pub id: BudgetScopeId,
    pub parent: Option<BudgetScopeId>,
    pub kind: BudgetScopeKind,
    /// Typed owner rendered at its adapter boundary (rollout/process/operation/permit).
    pub owner: String,
    pub limit: BudgetRequest,
}

/// Parent-bound proof returned when a child allocation is reserved.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetReservationReceipt {
    pub reservation_id: BudgetReservationId,
    pub scope_id: BudgetScopeId,
    pub parent_scope_id: BudgetScopeId,
    pub request: BudgetRequest,
}

impl Default for BudgetReservationId {
    fn default() -> Self {
        Self::new()
    }
}

/// Budget request during admission.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BudgetRequest {
    /// Maximum token budget.
    pub max_tokens: Option<u64>,
    /// Maximum cost in micro-dollars.
    pub max_cost_micro: Option<u64>,
}

/// Resource lease identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ResourceLeaseId(pub Uuid);

impl ResourceLeaseId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for ResourceLeaseId {
    fn default() -> Self {
        Self::new()
    }
}

/// Lease request during admission.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LeaseRequest {
    /// Resource name (e.g. "gpu", "sandbox-instance").
    pub resource: String,
    /// Lease duration in monotonic milliseconds.
    pub duration_ms: u64,
}

// ---------------------------------------------------------------------------
// Admission request and permit
// ---------------------------------------------------------------------------

/// Request to the admission controller.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdmissionRequest {
    pub operation_id: crate::types::operation::OperationId,
    pub process_id: crate::types::operation::ProcessId,
    pub principal: PrincipalId,
    pub capability: CapabilityId,
    pub action: String,
    pub input_summary: String,
    pub risk: RiskLevel,
    pub requested_scope: CapabilityScope,
    pub budget: Option<BudgetRequest>,
    pub lease: Option<LeaseRequest>,
    pub sandbox: SandboxRequirement,
}

/// Execution permit granted by the admission controller.
///
/// This is the ONLY way to execute a side-effecting capability.
/// Tool runners MUST check for a valid, non-expired permit.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionPermit {
    pub id: PermitId,
    pub operation_id: crate::types::operation::OperationId,
    pub process_id: crate::types::operation::ProcessId,
    pub capability: CapabilityId,
    pub granted_scope: CapabilityScope,
    pub expires_at: MonoDeadline,
    pub sandbox: SandboxDecision,
    pub budget_reservation: Option<BudgetReservationId>,
    pub lease: Option<ResourceLeaseId>,
}

impl ExecutionPermit {
    /// Check whether this permit has expired at the given monotonic time.
    pub fn is_expired_at(&self, now: crate::types::time::MonoTime) -> bool {
        self.expires_at.is_expired_at(now)
    }

    /// Check whether this permit is valid for execution at the given time.
    pub fn is_valid_at(&self, now: crate::types::time::MonoTime) -> bool {
        if self.is_expired_at(now) {
            return false;
        }
        !matches!(
            self.sandbox,
            SandboxDecision::Failed { .. }
                | SandboxDecision::Unavailable
                | SandboxDecision::Required
        )
    }
}

// ---------------------------------------------------------------------------
// Usage report (post-execution settlement)
// ---------------------------------------------------------------------------

/// Usage report submitted after capability execution.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct UsageReport {
    pub permit_id: PermitId,
    pub tokens_used: u64,
    pub cost_micro: u64,
    pub wall_time_ms: u64,
    pub output_bytes: u64,
    pub exit_code: Option<i32>,
}

/// Audit event identifier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AuditEventId(pub Uuid);

impl AuditEventId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

impl Default for AuditEventId {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Admission errors
// ---------------------------------------------------------------------------

/// Reasons admission may be denied.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum AdmissionError {
    Denied { reason: String },
    ApprovalRequired { prompt: String },
    SandboxRequiredUnavailable,
    BudgetExceeded,
    QuotaExceeded,
    LeaseUnavailable,
    InvalidScope { reason: String },
    PermitExpired,
    AlreadySettled,
}

impl std::fmt::Display for AdmissionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            AdmissionError::Denied { reason } => write!(f, "admission denied: {reason}"),
            AdmissionError::ApprovalRequired { prompt } => {
                write!(f, "approval required: {prompt}")
            }
            AdmissionError::SandboxRequiredUnavailable => {
                write!(f, "sandbox required but unavailable")
            }
            AdmissionError::BudgetExceeded => write!(f, "budget exceeded"),
            AdmissionError::QuotaExceeded => write!(f, "quota exceeded"),
            AdmissionError::LeaseUnavailable => write!(f, "lease unavailable"),
            AdmissionError::InvalidScope { reason } => write!(f, "invalid scope: {reason}"),
            AdmissionError::PermitExpired => write!(f, "permit expired"),
            AdmissionError::AlreadySettled => write!(f, "permit already settled"),
        }
    }
}

impl std::error::Error for AdmissionError {}

// ---------------------------------------------------------------------------
// Revoke reason
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RevokeReason {
    OperationCancelled,
    LeaseExpired,
    BudgetDepleted,
    PolicyChange,
    AdminAction,
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::time::MonoTime;

    fn future_deadline() -> MonoDeadline {
        MonoDeadline::after(MonoTime(0), 10_000)
    }

    fn past_deadline() -> MonoDeadline {
        MonoDeadline::after(MonoTime(0), 0)
    }

    #[test]
    fn permit_valid_when_not_expired_and_sandbox_passed() {
        let permit = ExecutionPermit {
            id: PermitId::new(),
            operation_id: Default::default(),
            process_id: Default::default(),
            capability: CapabilityId("test".into()),
            granted_scope: CapabilityScope::default(),
            expires_at: future_deadline(),
            sandbox: SandboxDecision::Passed,
            budget_reservation: None,
            lease: None,
        };
        assert!(permit.is_valid_at(MonoTime(0)));
    }

    #[test]
    fn permit_invalid_when_expired() {
        let permit = ExecutionPermit {
            id: PermitId::new(),
            operation_id: Default::default(),
            process_id: Default::default(),
            capability: CapabilityId("test".into()),
            granted_scope: CapabilityScope::default(),
            expires_at: past_deadline(),
            sandbox: SandboxDecision::Passed,
            budget_reservation: None,
            lease: None,
        };
        assert!(permit.is_expired_at(MonoTime(100)));
        assert!(!permit.is_valid_at(MonoTime(100)));
    }

    #[test]
    fn permit_invalid_when_sandbox_unavailable() {
        let permit = ExecutionPermit {
            id: PermitId::new(),
            operation_id: Default::default(),
            process_id: Default::default(),
            capability: CapabilityId("test".into()),
            granted_scope: CapabilityScope::default(),
            expires_at: future_deadline(),
            sandbox: SandboxDecision::Unavailable,
            budget_reservation: None,
            lease: None,
        };
        assert!(!permit.is_valid_at(MonoTime(0)));
    }

    #[test]
    fn permit_invalid_when_sandbox_failed() {
        let permit = ExecutionPermit {
            id: PermitId::new(),
            operation_id: Default::default(),
            process_id: Default::default(),
            capability: CapabilityId("test".into()),
            granted_scope: CapabilityScope::default(),
            expires_at: future_deadline(),
            sandbox: SandboxDecision::Failed {
                reason: "test failure".into(),
            },
            budget_reservation: None,
            lease: None,
        };
        assert!(!permit.is_valid_at(MonoTime(0)));
    }

    #[test]
    fn admission_error_display() {
        let e = AdmissionError::Denied {
            reason: "test".into(),
        };
        assert!(e.to_string().contains("test"));

        let e = AdmissionError::SandboxRequiredUnavailable;
        assert!(e.to_string().contains("sandbox"));
    }
}
