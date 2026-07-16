use fabric::body::{Action, ActionResult};
use fabric::capability::{Capability, CapabilityLevel as AbiPermissionLevel};
use fabric::context::Context;
use fabric::tool::{PermissionLevel as ToolPermissionLevel, ToolContext, ToolResult};
use std::sync::Arc;
use std::time::Duration;

/// Convert Action to tool name + JSON input
pub fn action_to_tool_input(action: &Action) -> (String, serde_json::Value) {
    (action.name.clone(), action.parameters.clone())
}

/// Convert ToolResult to ActionResult
pub fn tool_result_to_action_result(result: &ToolResult) -> ActionResult {
    ActionResult {
        success: !result.is_error,
        output: result.content.clone(),
        error: if result.is_error {
            Some(result.content.clone())
        } else {
            None
        },
        elapsed_ms: result.metadata.execution_time_ms,
        truncated: result.metadata.truncated,
        side_effects: Vec::new(), // ToolResult doesn't track side effects
    }
}

/// Convert Context to ToolContext
pub fn context_to_tool_context(ctx: &Context, clock: Arc<dyn fabric::Clock>) -> ToolContext {
    ToolContext {
        agent: None,
        working_dir: ctx.working_dir.clone(),
        session_id: ctx.session_id.clone(),
        clock,
    }
}

/// Convert tool PermissionLevel to ABI PermissionLevel
pub fn tool_to_abi_permission(level: ToolPermissionLevel) -> AbiPermissionLevel {
    match level {
        ToolPermissionLevel::L0 => AbiPermissionLevel::ReadOnly,
        ToolPermissionLevel::L1 => AbiPermissionLevel::SandboxWrite,
        ToolPermissionLevel::L2 => AbiPermissionLevel::SystemChange,
        ToolPermissionLevel::L3 => AbiPermissionLevel::Destructive,
    }
}

/// Convert tool metadata to Capability
pub fn tool_to_capability(name: &str, level: ToolPermissionLevel, description: &str) -> Capability {
    Capability {
        name: name.to_string(),
        level: tool_to_abi_permission(level),
        description: description.to_string(),
    }
}

/// Convert ActionResult elapsed_ms to Duration
pub fn elapsed_to_duration(ms: u64) -> Duration {
    Duration::from_millis(ms)
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::tool::ToolResultMeta;

    #[test]
    fn test_action_roundtrip() {
        let action = Action {
            name: "bash_exec".to_string(),
            parameters: serde_json::json!({"command": "ls -la"}),
            requires_sandbox: false,
            timeout: None,
        };
        let (name, params) = action_to_tool_input(&action);
        assert_eq!(name, "bash_exec");
        assert_eq!(params["command"], "ls -la");
    }

    #[test]
    fn test_tool_result_conversion() {
        let result = ToolResult {
            content: "hello".to_string(),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: 100,
                truncated: false,
            },
        };
        let ar = tool_result_to_action_result(&result);
        assert!(ar.success);
        assert_eq!(ar.output, "hello");
        assert_eq!(ar.elapsed_ms, 100);
    }

    #[test]
    fn test_permission_mapping() {
        assert_eq!(
            tool_to_abi_permission(ToolPermissionLevel::L0),
            AbiPermissionLevel::ReadOnly
        );
        assert_eq!(
            tool_to_abi_permission(ToolPermissionLevel::L3),
            AbiPermissionLevel::Destructive
        );
    }
}
