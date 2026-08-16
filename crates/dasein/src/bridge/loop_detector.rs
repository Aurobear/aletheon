use std::sync::Arc;

use crate::core::contracts::{AwarenessRiskLevel, Verdict};
use ::contracts::tool::ToolResult;
use serde_json::Value;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum LoopDecision {
    Allow,
    Warn {
        reason: String,
    },
    Block {
        reason: String,
        suggestion: String,
    },
    Escalate {
        reason: String,
    },
    InterruptTurn {
        reason: String,
        consecutive_blocks: usize,
    },
}

/// Host-supplied stateful loop detector port.
pub trait LoopDecisionPort: Send + Sync {
    fn on_new_turn(&self, turn_id: &str);
    fn pre_check(&self, tool_name: &str, args: &Value, turn_id: &str) -> LoopDecision;
    fn post_check(&self, tool_name: &str, args: &Value, result: &ToolResult, turn_id: &str);
    fn end_turn(&self, turn_id: &str);
}

/// Maps host loop decisions into SelfField's Verdict system.
pub struct LoopBridge {
    detector: Arc<dyn LoopDecisionPort>,
}

impl LoopBridge {
    pub fn new(detector: Arc<dyn LoopDecisionPort>) -> Self {
        Self { detector }
    }

    pub fn on_new_turn(&self, turn_id: &str) {
        self.detector.on_new_turn(turn_id);
    }

    pub fn pre_check(&self, tool_name: &str, args: &Value, turn_id: &str) -> Option<Verdict> {
        match self.detector.pre_check(tool_name, args, turn_id) {
            LoopDecision::Allow | LoopDecision::Warn { .. } => None,
            LoopDecision::Block { reason, suggestion } => Some(Verdict::Deny {
                reason: format!("{reason}. Suggestion: {suggestion}"),
            }),
            LoopDecision::Escalate { reason } => Some(Verdict::RequireConfirmation {
                reason,
                risk_level: AwarenessRiskLevel::Critical,
            }),
            LoopDecision::InterruptTurn { reason, .. } => Some(Verdict::Deny {
                reason: format!("Turn interrupted: {reason}"),
            }),
        }
    }

    pub fn post_check(&self, tool_name: &str, args: &Value, result: &ToolResult, turn_id: &str) {
        self.detector.post_check(tool_name, args, result, turn_id);
    }

    pub fn end_turn(&self, turn_id: &str) {
        self.detector.end_turn(turn_id);
    }
}
