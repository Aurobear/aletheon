use crate::r#impl::hook::dispatcher::HookDispatcher;
use crate::r#impl::hook::types::{HandlerResult, HookContext, HookEventName};
use base::self_field::Verdict;

/// Bridges HookDispatcher into SelfField.
pub struct HookBridge {
    dispatcher: Option<HookDispatcher>,
}

impl HookBridge {
    pub fn new() -> Self {
        Self {
            dispatcher: HookDispatcher::try_load(),
        }
    }

    /// Fire PreToolUse hooks. Returns Some(Verdict) if a hook blocks.
    pub async fn fire_pre_tool(&self, tool_name: &str, args: &str) -> Option<Verdict> {
        let dispatcher = self.dispatcher.as_ref()?;
        let ctx = HookContext {
            tool: Some(tool_name.to_string()),
            args: Some(args.to_string()),
            risk: None,
            message: None,
        };
        match dispatcher.fire(HookEventName::PreToolUse, &ctx).await {
            HandlerResult::Continue => None,
            HandlerResult::Block(reason) => Some(Verdict::Deny { reason }),
            HandlerResult::ModifyArgs(new_args) => Some(Verdict::AllowWithModification {
                modification: new_args,
            }),
            HandlerResult::InjectContext(text) => Some(Verdict::AllowWithModification {
                modification: serde_json::json!({"inject_context": text}),
            }),
            HandlerResult::Failed(reason) => Some(Verdict::Deny {
                reason: format!("Hook failed: {}", reason),
            }),
            HandlerResult::TimedOut => Some(Verdict::Deny {
                reason: "Hook timed out".to_string(),
            }),
        }
    }

    /// Fire PreLLMCall hooks.
    pub async fn fire_pre_llm(&self) -> Option<Verdict> {
        let dispatcher = self.dispatcher.as_ref()?;
        let ctx = HookContext {
            tool: None,
            args: None,
            risk: None,
            message: None,
        };
        match dispatcher.fire(HookEventName::PreLLMCall, &ctx).await {
            HandlerResult::Continue => None,
            HandlerResult::Block(reason) => Some(Verdict::Deny { reason }),
            _ => None,
        }
    }

    /// Check if hooks are loaded.
    pub fn is_loaded(&self) -> bool {
        self.dispatcher.is_some()
    }
}

impl Default for HookBridge {
    fn default() -> Self {
        Self::new()
    }
}
