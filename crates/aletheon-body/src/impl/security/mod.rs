//! Security layer — guardrails, rate limiting, loop detection, and audit logging.
//!
//! Security layer — guardrails, rate limiting, loop detection, and audit logging.

pub mod loop_detector;
pub mod risk_classifier;
pub mod circuit_breaker;
pub mod output_guardrail;
pub mod audit;
pub mod policy;
pub mod exec_policy;
pub mod runner;

// Re-export key types
pub use loop_detector::{LoopDetector, LoopVerdict, LoopDetectorConfig};
pub use risk_classifier::{RiskClassifier, RiskCategory};
pub use circuit_breaker::LoopCircuitBreaker;
pub use output_guardrail::OutputGuardrail;
pub use audit::AuditLogger;
pub use policy::{PolicyEngine, PolicyVerdict};
pub use exec_policy::{ExecPolicyEngine, PolicyDecision as ExecPolicyDecision, PolicyAction as ExecPolicyAction};
pub use runner::ToolRunnerWithGuard;
