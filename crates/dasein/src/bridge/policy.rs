use std::sync::Arc;

use crate::core::contracts::{AwarenessRiskLevel, Verdict};
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { reason: String },
}

/// Host-supplied policy decision port. Dasein owns only the mapping into its
/// constitutional Verdict vocabulary, never a concrete tool policy engine.
pub trait PolicyDecisionPort: Send + Sync {
    fn check(&self, tool_name: &str, input: &Value) -> PolicyDecision;
}

/// Maps a host policy decision into SelfField's Verdict system.
pub struct PolicyBridge {
    policy: Arc<dyn PolicyDecisionPort>,
}

impl PolicyBridge {
    pub fn new(policy: Arc<dyn PolicyDecisionPort>) -> Self {
        Self { policy }
    }

    pub fn check(&self, tool_name: &str, input: &Value) -> Option<Verdict> {
        match self.policy.check(tool_name, input) {
            PolicyDecision::Allow => None,
            PolicyDecision::Deny { reason } => Some(Verdict::Deny { reason }),
            PolicyDecision::RequireApproval { reason } => Some(Verdict::RequireConfirmation {
                reason,
                risk_level: AwarenessRiskLevel::High,
            }),
        }
    }
}
