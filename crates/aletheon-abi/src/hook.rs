// crates/aletheon-abi/src/hook.rs

//! Hook types — lifecycle callback definitions for the Aletheon runtime.
//!
//! Hooks are synchronous intervention points in the ReAct loop where
//! external scripts or builtin logic can inspect and modify behavior.

use std::collections::HashMap;
use serde::{Deserialize, Serialize};

/// Points in the execution lifecycle where hooks can intervene.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HookPoint {
    /// Fired once when a new session starts.
    OnSessionStart,
    /// Fired when a session ends.
    OnSessionEnd,
    /// Fired before processing a user message.
    PreTurn,
    /// Fired after LLM response is generated.
    PostTurn,
    /// Fired before a tool executes.
    PreTool,
    /// Fired after a tool executes.
    PostTool,
    /// Fired when a memory entry is stored.
    OnMemoryStore,
    /// Fired when a memory entry is recalled.
    OnMemoryRecall,
}

/// Context passed to hook execution.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookContext {
    /// Which hook point triggered this execution.
    pub point: HookPoint,
    /// Current session identifier.
    pub session_id: String,
    /// Number of turns completed in this session.
    pub turn_count: usize,
    /// Tool name (for PreTool/PostTool hooks).
    pub tool_name: Option<String>,
    /// Tool input (for PreTool hooks).
    pub tool_input: Option<serde_json::Value>,
    /// Tool result (for PostTool hooks).
    pub tool_result: Option<HookToolResult>,
    /// User message (for PreTurn hooks).
    pub message: Option<String>,
    /// Arbitrary key-value metadata.
    pub metadata: HashMap<String, String>,
}

/// Simplified tool result for hook context serialization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HookToolResult {
    pub content: String,
    pub is_error: bool,
    pub execution_time_ms: u64,
}

/// Result of hook execution.
#[derive(Debug, Clone)]
pub enum HookResult {
    /// Continue normal execution without modification.
    Continue,
    /// Modify the tool input (only valid for PreTool).
    ModifyInput(serde_json::Value),
    /// Block execution with a reason.
    Block { reason: String },
    /// Inject additional content into the user message.
    Inject(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_point_serde_roundtrip() {
        let points = vec![
            HookPoint::OnSessionStart,
            HookPoint::PreTool,
            HookPoint::PostTurn,
        ];
        for point in points {
            let json = serde_json::to_string(&point).unwrap();
            let back: HookPoint = serde_json::from_str(&json).unwrap();
            assert_eq!(point, back);
        }
    }

    #[test]
    fn hook_context_serde_roundtrip() {
        let ctx = HookContext {
            point: HookPoint::PreTool,
            session_id: "test-session".into(),
            turn_count: 5,
            tool_name: Some("bash_exec".into()),
            tool_input: Some(serde_json::json!({"command": "ls"})),
            tool_result: None,
            message: None,
            metadata: HashMap::new(),
        };
        let json = serde_json::to_string(&ctx).unwrap();
        let back: HookContext = serde_json::from_str(&json).unwrap();
        assert_eq!(back.point, HookPoint::PreTool);
        assert_eq!(back.tool_name, Some("bash_exec".into()));
    }

    #[test]
    fn hook_result_continue_is_default() {
        let result = HookResult::Continue;
        assert!(matches!(result, HookResult::Continue));
    }

    #[test]
    fn hook_tool_result_serde() {
        let result = HookToolResult {
            content: "output".into(),
            is_error: false,
            execution_time_ms: 100,
        };
        let json = serde_json::to_string(&result).unwrap();
        let back: HookToolResult = serde_json::from_str(&json).unwrap();
        assert_eq!(back.content, "output");
        assert!(!back.is_error);
    }
}
