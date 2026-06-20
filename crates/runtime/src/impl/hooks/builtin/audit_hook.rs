// crates/aletheon-runtime/src/impl/hooks/builtin/audit_hook.rs

//! Audit hook — logs all tool calls to the audit log.
//!
//! This hook registers for PostTool and logs tool name, input summary,
//! success/failure, and execution time.

use tracing::info;

use crate::r#impl::hooks::registry::{HookRegistry, RegisteredHook};
use base::hook::HookPoint;

/// Register the audit hook in the hook registry.
pub fn register_audit_hook(registry: &mut HookRegistry) {
    registry.register(RegisteredHook {
        name: "builtin:audit".into(),
        source: "builtin".into(),
        script_path: None,
        point: HookPoint::PostTool,
        priority: 1000, // Run last
    });
}

/// Log a tool call result. Called from handler.rs after PostTool hooks.
pub fn log_tool_call(tool_name: &str, is_error: bool, execution_time_ms: u64, content_len: usize) {
    info!(
        tool = %tool_name,
        error = is_error,
        ms = execution_time_ms,
        bytes = content_len,
        "Tool call completed"
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_audit_hook_adds_entry() {
        let mut reg = HookRegistry::new();
        register_audit_hook(&mut reg);
        assert_eq!(reg.count(&HookPoint::PostTool), 1);
    }
}
