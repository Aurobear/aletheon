// crates/aletheon-runtime/src/impl/hooks/mod.rs

pub mod builtin;
pub mod lifecycle;
pub mod loader;
pub mod registry;

use base::hook::{HookContext, HookPoint, HookResult};
use registry::HookRegistry;

/// High-level hook events that map to ABI `HookPoint` values.
///
/// This enum provides a typed, ergonomic API on top of the lower-level
/// `HookContext`/`HookPoint` system used by the registry.
#[derive(Debug, Clone)]
pub enum HookEvent {
    SessionStart {
        session_id: String,
        mode: String,
        model: String,
    },
    PreTool {
        tool_name: String,
        input: serde_json::Value,
    },
    PostTool {
        tool_name: String,
        output: String,
        success: bool,
        duration_ms: u64,
    },
    PreResponse {
        response: String,
        tokens_used: u32,
    },
    SessionEnd {
        session_id: String,
        duration_secs: u64,
        total_tokens: u32,
    },
    Custom {
        event_type: String,
        data: serde_json::Value,
    },
}

/// Result returned by `HookRegistry::fire()`.
#[derive(Debug, Clone)]
pub enum FireResult {
    /// No hook blocked or modified the event.
    Continue,
    /// A hook blocked the event.
    Block { reason: String },
    /// A hook requested input modification.
    ModifyInput(serde_json::Value),
    /// A hook injected additional content.
    Inject(String),
}

impl From<HookResult> for FireResult {
    fn from(r: HookResult) -> Self {
        match r {
            HookResult::Continue => FireResult::Continue,
            HookResult::Block { reason } => FireResult::Block { reason },
            HookResult::ModifyInput(v) => FireResult::ModifyInput(v),
            HookResult::Inject(s) => FireResult::Inject(s),
        }
    }
}

impl HookEvent {
    /// Convert this high-level event into a `HookContext` suitable for the
    /// existing `HookRegistry::execute()` method.
    pub fn to_context(&self, turn_count: usize) -> HookContext {
        match self {
            HookEvent::SessionStart {
                session_id,
                mode: _,
                model: _,
            } => HookContext {
                point: HookPoint::OnSessionStart,
                session_id: session_id.clone(),
                turn_count,
                tool_name: None,
                tool_input: None,
                tool_result: None,
                message: None,
                metadata: std::collections::HashMap::new(),
            },
            HookEvent::PreTool { tool_name, input } => HookContext {
                point: HookPoint::PreTool,
                session_id: String::new(),
                turn_count,
                tool_name: Some(tool_name.clone()),
                tool_input: Some(input.clone()),
                tool_result: None,
                message: None,
                metadata: std::collections::HashMap::new(),
            },
            HookEvent::PostTool {
                tool_name,
                output,
                success,
                duration_ms,
            } => {
                let mut metadata = std::collections::HashMap::new();
                metadata.insert("success".to_string(), success.to_string());
                metadata.insert("duration_ms".to_string(), duration_ms.to_string());
                HookContext {
                    point: HookPoint::PostTool,
                    session_id: String::new(),
                    turn_count,
                    tool_name: Some(tool_name.clone()),
                    tool_input: None,
                    tool_result: Some(base::hook::HookToolResult {
                        content: output.clone(),
                        is_error: !success,
                        execution_time_ms: *duration_ms,
                    }),
                    message: None,
                    metadata,
                }
            }
            HookEvent::PreResponse {
                response: _,
                tokens_used,
            } => {
                let mut metadata = std::collections::HashMap::new();
                metadata.insert("tokens_used".to_string(), tokens_used.to_string());
                HookContext {
                    point: HookPoint::PostTurn,
                    session_id: String::new(),
                    turn_count,
                    tool_name: None,
                    tool_input: None,
                    tool_result: None,
                    message: None,
                    metadata,
                }
            }
            HookEvent::SessionEnd {
                session_id,
                duration_secs,
                total_tokens,
            } => {
                let mut metadata = std::collections::HashMap::new();
                metadata.insert("duration_secs".to_string(), duration_secs.to_string());
                metadata.insert("total_tokens".to_string(), total_tokens.to_string());
                HookContext {
                    point: HookPoint::OnSessionEnd,
                    session_id: session_id.clone(),
                    turn_count,
                    tool_name: None,
                    tool_input: None,
                    tool_result: None,
                    message: None,
                    metadata,
                }
            }
            HookEvent::Custom {
                event_type,
                data: _,
            } => {
                let mut metadata = std::collections::HashMap::new();
                metadata.insert("event_type".to_string(), event_type.clone());
                HookContext {
                    point: HookPoint::PostTurn,
                    session_id: String::new(),
                    turn_count,
                    tool_name: None,
                    tool_input: None,
                    tool_result: None,
                    message: None,
                    metadata,
                }
            }
        }
    }
}

/// Convenience extension for `HookRegistry` to fire typed hook events.
#[allow(async_fn_in_trait)]
pub trait HookEventExt {
    /// Fire a hook event, converting it to the ABI context and executing
    /// through the registry.
    async fn fire(&self, event: &HookEvent, turn_count: usize) -> FireResult;
}

impl HookEventExt for HookRegistry {
    async fn fire(&self, event: &HookEvent, turn_count: usize) -> FireResult {
        let ctx = event.to_context(turn_count);
        let result = self.execute(&ctx).await;
        FireResult::from(result)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hook_event_to_context_session_start() {
        let event = HookEvent::SessionStart {
            session_id: "s1".into(),
            mode: "auto".into(),
            model: "gpt-4".into(),
        };
        let ctx = event.to_context(0);
        assert_eq!(ctx.point, HookPoint::OnSessionStart);
        assert_eq!(ctx.session_id, "s1");
    }

    #[test]
    fn hook_event_to_context_pre_tool() {
        let event = HookEvent::PreTool {
            tool_name: "bash_exec".into(),
            input: serde_json::json!({"command": "ls"}),
        };
        let ctx = event.to_context(5);
        assert_eq!(ctx.point, HookPoint::PreTool);
        assert_eq!(ctx.tool_name, Some("bash_exec".into()));
        assert_eq!(ctx.turn_count, 5);
    }

    #[test]
    fn hook_event_to_context_post_tool() {
        let event = HookEvent::PostTool {
            tool_name: "file_read".into(),
            output: "contents".into(),
            success: true,
            duration_ms: 42,
        };
        let ctx = event.to_context(1);
        assert_eq!(ctx.point, HookPoint::PostTool);
        let result = ctx.tool_result.unwrap();
        assert_eq!(result.content, "contents");
        assert!(!result.is_error);
    }

    #[test]
    fn hook_event_to_context_session_end() {
        let event = HookEvent::SessionEnd {
            session_id: "s2".into(),
            duration_secs: 3600,
            total_tokens: 5000,
        };
        let ctx = event.to_context(10);
        assert_eq!(ctx.point, HookPoint::OnSessionEnd);
        assert_eq!(ctx.session_id, "s2");
        assert_eq!(ctx.metadata.get("duration_secs").unwrap(), "3600");
    }

    #[test]
    fn fire_result_from_hook_result() {
        assert!(matches!(
            FireResult::from(HookResult::Continue),
            FireResult::Continue
        ));
        assert!(matches!(
            FireResult::from(HookResult::Block {
                reason: "no".into()
            }),
            FireResult::Block { .. }
        ));
    }
}
