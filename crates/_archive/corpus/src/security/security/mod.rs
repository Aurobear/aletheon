//! Security layer — guardrails, rate limiting, loop detection, and audit logging.
//!
//! Security layer — guardrails, rate limiting, loop detection, and audit logging.

pub mod approval;
pub mod audit;
pub mod circuit_breaker;
pub mod exec_policy;
pub mod loop_detector;
pub mod output_guardrail;
pub mod permission_rules;
pub mod policy;
pub mod risk_classifier;
pub mod runner;
pub mod socket_approval;

// Re-export key types
pub use approval::{
    ApprovalDecision, ApprovalGate, ApprovalRequest, AutoApproveGate, AutoDenyGate,
    TerminalApprovalGate,
};
pub use audit::AuditLogger;
pub use circuit_breaker::LoopCircuitBreaker;
pub use exec_policy::{
    ExecPolicyEngine, PolicyAction as ExecPolicyAction, PolicyDecision as ExecPolicyDecision,
};
pub use loop_detector::{LoopDetector, LoopDetectorConfig, LoopVerdict};
pub use output_guardrail::OutputGuardrail;
pub use permission_rules::{load_permission_context, load_permission_context_from_str};
pub use policy::{PolicyEngine, PolicyVerdict};
pub use risk_classifier::{RiskCategory, RiskClassifier};
pub use runner::ToolRunnerWithGuard;
pub use socket_approval::{PendingApproval, SocketApprovalGate};
