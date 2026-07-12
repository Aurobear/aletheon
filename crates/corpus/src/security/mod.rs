//! Security pipeline and sandbox execution.

pub mod approval;
pub mod exec_policy;
pub mod permission_rules;
pub mod runner;
pub mod sandbox;
pub mod socket_approval;
pub mod storm_breaker;

// Shared security types — re-exported from fabric (single source of truth).
// Both corpus and dasein share these types via fabric::security.
pub use fabric::security::{
    audit::AuditLogger,
    circuit_breaker::LoopCircuitBreaker,
    loop_detector::{LoopDetector, LoopDetectorConfig, LoopVerdict},
    output_guardrail::OutputGuardrail,
    policy::{PolicyEngine, PolicyVerdict},
    risk_classifier::{RiskCategory, RiskClassifier},
};

// Backward-compatible module re-exports for code using `use crate::security::audit;` etc.
pub mod audit {
    pub use fabric::security::audit::*;
}
pub mod circuit_breaker {
    pub use fabric::security::circuit_breaker::*;
}
pub mod loop_detector {
    pub use fabric::security::loop_detector::*;
}
pub mod output_guardrail {
    pub use fabric::security::output_guardrail::*;
}
pub mod policy {
    pub use fabric::security::policy::*;
}
pub mod risk_classifier {
    pub use fabric::security::risk_classifier::*;
}

// Re-export key types
pub use approval::{
    ApprovalDecision, ApprovalGate, ApprovalRequest, AutoApproveGate, AutoDenyGate,
    TerminalApprovalGate,
};
pub use exec_policy::{
    ExecPolicyEngine, PolicyAction as ExecPolicyAction, PolicyDecision as ExecPolicyDecision,
};
pub use permission_rules::{load_permission_context, load_permission_context_from_str};
pub use runner::ToolRunnerWithGuard;
pub use socket_approval::{PendingApproval, SocketApprovalGate};
