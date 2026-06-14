use aletheon_abi::body::{Action, ActionResult, SideEffect, SideEffectKind};
use aletheon_abi::capability::{Capability, PermissionLevel as AbiPermissionLevel};
use aletheon_abi::context::Context;
use aletheon_abi::tool::{ToolResult, ToolResultMeta, PermissionLevel as ArgosPermissionLevel, ToolContext};
use std::time::Duration;

/// Convert aletheon Action → argos Tool name + JSON input
pub fn action_to_tool_input(action: &Action) -> (String, serde_json::Value) {
    (action.name.clone(), action.parameters.clone())
}

/// Convert argos ToolResult → aletheon ActionResult
pub fn tool_result_toActionResult(result: &ToolResult) -> ActionResult {
    ActionResult {
        success: !result.is_error,
        output: result.content.clone(),
        error: if result.is_error { Some(result.content.clone()) } else { None },
        elapsed_ms: result.metadata.execution_time_ms,
        truncated: result.metadata.truncated,
        side_effects: Vec::new(), // ToolResult doesn't track side effects
    }
}

/// Convert aletheon Context → argos ToolContext
pub fn context_to_tool_context(ctx: &Context) -> ToolContext {
    ToolContext {
        working_dir: ctx.working_dir.clone(),
        session_id: ctx.session_id.clone(),
    }
}

/// Convert argos PermissionLevel → aletheon PermissionLevel
pub fn argos_to_abi_permission(level: ArgosPermissionLevel) -> AbiPermissionLevel {
    match level {
        ArgosPermissionLevel::L0 => AbiPermissionLevel::ReadOnly,
        ArgosPermissionLevel::L1 => AbiPermissionLevel::SandboxWrite,
        ArgosPermissionLevel::L2 => AbiPermissionLevel::SystemChange,
        ArgosPermissionLevel::L3 => AbiPermissionLevel::Destructive,
    }
}

/// Convert argos Tool metadata → aletheon Capability
pub fn tool_to_capability(name: &str, level: ArgosPermissionLevel, description: &str) -> Capability {
    Capability {
        name: name.to_string(),
        level: argos_to_abi_permission(level),
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
        let ar = tool_result_toActionResult(&result);
        assert!(ar.success);
        assert_eq!(ar.output, "hello");
        assert_eq!(ar.elapsed_ms, 100);
    }

    #[test]
    fn test_permission_mapping() {
        assert_eq!(argos_to_abi_permission(ArgosPermissionLevel::L0), AbiPermissionLevel::ReadOnly);
        assert_eq!(argos_to_abi_permission(ArgosPermissionLevel::L3), AbiPermissionLevel::Destructive);
    }
}
