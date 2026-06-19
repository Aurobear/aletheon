//! Security layer — policy engine, loop detection, and rollback.

pub mod audit;
pub mod circuit_breaker;
pub mod loop_detector;
pub mod output_guardrail;
pub mod policy;
pub mod rate_limiting;
pub mod risk_classifier;
pub mod rollback;
pub mod runner;
pub mod sandbox;
pub mod self_protection;

// Re-export key types
pub use audit::AuditLogger;
pub use circuit_breaker::LoopCircuitBreaker;
pub use loop_detector::{LoopDetector, LoopDetectorConfig, LoopVerdict};
pub use output_guardrail::OutputGuardrail;
pub use policy::{PolicyEngine, PolicyVerdict};
pub use risk_classifier::{RiskCategory, RiskClassifier};
pub use runner::ToolRunnerWithGuard;
