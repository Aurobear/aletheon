//! Security layer — policy engine, loop detection, and rollback.

pub mod rate_limiting;
pub mod rollback;
pub mod runner;
pub mod sandbox;
pub mod self_protection;

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

// Backward-compatible module re-exports for code using
// `use crate::impl::security::audit;` etc.
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

pub use runner::ToolRunnerWithGuard;
