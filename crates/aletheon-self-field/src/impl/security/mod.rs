//! Security layer for Argos (migrated from argos-security).

pub mod loop_detector;
pub mod risk_classifier;
pub mod circuit_breaker;
pub mod output_guardrail;
pub mod audit;
pub mod policy;
pub mod runner;
pub mod rollback;
pub mod sandbox;
pub mod self_protection;
pub mod rate_limiting;

// Re-export key types
pub use loop_detector::{LoopDetector, LoopVerdict, LoopDetectorConfig};
pub use risk_classifier::{RiskClassifier, RiskCategory};
pub use circuit_breaker::LoopCircuitBreaker;
pub use output_guardrail::OutputGuardrail;
pub use audit::AuditLogger;
pub use policy::{PolicyEngine, PolicyVerdict};
pub use runner::ToolRunnerWithGuard;
