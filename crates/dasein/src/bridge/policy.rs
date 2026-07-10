use crate::r#impl::security::{PolicyEngine, PolicyVerdict};
use fabric::self_field::{RiskLevel, Verdict};
use serde_json::Value;

/// Bridges PolicyEngine into SelfField's Verdict system
pub struct PolicyBridge {
    engine: PolicyEngine,
}

impl PolicyBridge {
    pub fn new() -> Self {
        Self {
            engine: PolicyEngine::with_defaults(),
        }
    }

    /// Check a tool call against the policy engine
    /// Maps PolicyVerdict → fabric::Verdict
    pub fn check(&self, tool_name: &str, input: &Value) -> Option<Verdict> {
        match self.engine.check(tool_name, input) {
            PolicyVerdict::Allow => None, // No verdict needed, continue to other checks
            PolicyVerdict::Deny { reason } => Some(Verdict::Deny { reason }),
            PolicyVerdict::RequireApproval { reason } => Some(Verdict::RequireConfirmation {
                reason,
                risk_level: RiskLevel::High,
            }),
        }
    }

    /// Access the underlying engine for configuration
    pub fn engine(&self) -> &PolicyEngine {
        &self.engine
    }
}

impl Default for PolicyBridge {
    fn default() -> Self {
        Self::new()
    }
}
