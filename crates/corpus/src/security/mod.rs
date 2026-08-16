//! Security pipeline and sandbox execution.

pub mod approval;
pub mod audit;
pub mod circuit_breaker;
pub(crate) mod command_effect;
pub mod credential_vault;
pub mod escape_detector;
pub mod exec_policy;
pub mod execd_backend;
pub mod execpolicy;
pub mod loop_detector;
pub mod network_policy;
pub mod output_guardrail;
pub mod permission_rules;
pub mod policy;
pub mod risk_classifier;
pub mod runner;
pub mod sandbox;
mod sandbox_glob;
pub mod socket_approval;
pub mod storm_breaker;
pub mod strategy;
pub mod structured_sandbox;

pub use audit::AuditLogger;
pub use risk_classifier::{RiskCategory, RiskClassifier};

pub use circuit_breaker::LoopCircuitBreaker;
pub use loop_detector::{LoopDetector, LoopDetectorConfig, LoopVerdict};
pub use output_guardrail::OutputGuardrail;

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
pub use structured_sandbox::StructuredToolSandbox;
