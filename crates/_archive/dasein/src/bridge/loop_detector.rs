use crate::r#impl::security::{LoopDetector, LoopDetectorConfig, LoopVerdict};
use base::self_field::{RiskLevel, Verdict};
use base::tool::ToolResult;
use parking_lot::Mutex;
use serde_json::Value;

/// Bridges LoopDetector into SelfField's Verdict system.
///
/// Uses interior mutability (`parking_lot::Mutex`) because `LoopDetector` methods
/// take `&mut self` while the bridge API exposes `&self` for consistency with
/// other bridge modules.
pub struct LoopBridge {
    detector: Mutex<LoopDetector>,
}

impl LoopBridge {
    pub fn new() -> Self {
        Self {
            detector: Mutex::new(LoopDetector::new(LoopDetectorConfig::default())),
        }
    }

    /// Notify of a new turn.
    pub fn on_new_turn(&self, turn_id: &str) {
        self.detector.lock().on_new_turn(turn_id);
    }

    /// Pre-check a tool call for loops.
    /// Maps `LoopVerdict` to `Option<Verdict>`.
    pub fn pre_check(&self, tool_name: &str, args: &Value, turn_id: &str) -> Option<Verdict> {
        match self.detector.lock().pre_check(tool_name, args, turn_id) {
            LoopVerdict::Allow => None,
            LoopVerdict::Warn { .. } => None, // Warn but allow
            LoopVerdict::Block { reason, suggestion } => Some(Verdict::Deny {
                reason: format!("{}. Suggestion: {}", reason, suggestion),
            }),
            LoopVerdict::Escalate { reason } => Some(Verdict::RequireConfirmation {
                reason,
                risk_level: RiskLevel::Critical,
            }),
            LoopVerdict::InterruptTurn { reason, .. } => Some(Verdict::Deny {
                reason: format!("Turn interrupted: {}", reason),
            }),
        }
    }

    /// Post-check: record a completed tool call.
    pub fn post_check(&self, tool_name: &str, args: &Value, result: &ToolResult, turn_id: &str) {
        self.detector
            .lock()
            .post_check(tool_name, args, result, turn_id);
    }

    /// End a turn.
    pub fn end_turn(&self, turn_id: &str) {
        self.detector.lock().end_turn(turn_id);
    }
}

impl Default for LoopBridge {
    fn default() -> Self {
        Self::new()
    }
}
