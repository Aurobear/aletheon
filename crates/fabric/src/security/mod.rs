//! Shared security primitives — policy engine, loop detection, audit, and risk classification.
//!
//! These types are shared between `corpus` and `dasein` via `fabric`.
//! Both crates re-export from here to avoid duplication.

pub mod audit;
pub mod circuit_breaker;
pub mod loop_detector;
pub mod output_guardrail;
pub mod policy;
pub mod risk_classifier;

// Re-export key types
pub use audit::AuditLogger;
pub use circuit_breaker::LoopCircuitBreaker;
pub use loop_detector::{LoopDetector, LoopDetectorConfig, LoopVerdict};
pub use output_guardrail::OutputGuardrail;
pub use policy::{PolicyEngine, PolicyVerdict};
pub use risk_classifier::{RiskCategory, RiskClassifier};
