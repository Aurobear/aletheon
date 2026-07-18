use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use super::client::{McpClient, McpResource, McpTool};
use super::config::McpTrustLevel;
use crate::tools::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use fabric::tool::ConcurrencyClass;

fn permission_override(
    overrides: &HashMap<String, PermissionLevel>,
    tool_name: &str,
    server_name: &str,
) -> Option<PermissionLevel> {
    overrides
        .get(tool_name)
        .or_else(|| overrides.get(server_name))
        .copied()
}

/// Wraps an MCP-discovered tool as a local `Tool` implementation.
pub struct McpToolWrapper {
    pub normalized_name: String,
    pub mcp_tool: McpTool,
    pub client: Arc<Mutex<McpClient>>,
    pub trust_level: McpTrustLevel,
    /// The server this tool belongs to, used for permission override lookup.
    pub server_name: String,
    /// Per-tool overrides keyed by normalized tool name. Server-name keys are
    /// retained only as a compatibility fallback.
    pub overrides: HashMap<String, PermissionLevel>,
    /// Whether the MCP server supports parallel tool calls.
    pub supports_parallel: bool,
}

#[async_trait]
impl Tool for McpToolWrapper {
    fn name(&self) -> &str {
        &self.normalized_name
    }

    fn description(&self) -> &str {
        &self.mcp_tool.description
    }

    fn input_schema(&self) -> Value {
        self.mcp_tool.input_schema.clone()
    }

    fn permission_level(&self) -> PermissionLevel {
        if let Some(override_level) =
            self.overrides
                .get(&self.mcp_tool.name)
                .copied()
                .or_else(|| {
                    permission_override(&self.overrides, &self.normalized_name, &self.server_name)
                })
        {
            return override_level;
        }
        // Fall back to trust-based mapping
        match self.trust_level {
            McpTrustLevel::LocalTrusted => PermissionLevel::L1,
            McpTrustLevel::RemoteTrusted => PermissionLevel::L1,
            McpTrustLevel::Untrusted => PermissionLevel::L0,
        }
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        if self.supports_parallel {
            ConcurrencyClass::ReadOnly
        } else {
            ConcurrencyClass::SideEffect
        }
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(McpToolWrapper {
            normalized_name: self.normalized_name.clone(),
            mcp_tool: self.mcp_tool.clone(),
            client: self.client.clone(),
            trust_level: self.trust_level,
            server_name: self.server_name.clone(),
            overrides: self.overrides.clone(),
            supports_parallel: self.supports_parallel,
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let mut client = self.client.lock().await;
        let start = ctx.clock.mono_now();

        match client.call_tool(&self.mcp_tool.name, input).await {
            Ok(response) => {
                let content = serde_json::to_string_pretty(&response)
                    .unwrap_or_else(|_| format!("{:?}", response));
                ToolResult {
                    content,
                    is_error: false,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                }
            }
            Err(e) => ToolResult {
                content: format!("MCP tool error: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }
}

/// Wraps an MCP resource as a Tool so it can be called by the harness.
///
/// Resources are read-only content accessed via `resources/read`.
pub struct McpResourceProvider {
    pub uri: String,
    pub normalized_name: String,
    pub mcp_resource: McpResource,
    pub client: Arc<Mutex<McpClient>>,
    /// The server this resource belongs to, used for permission override lookup.
    pub server_name: String,
    /// Per-tool overrides keyed by normalized resource-tool name.
    pub overrides: HashMap<String, PermissionLevel>,
}

/// Generic read-only MCP resource tool. Unlike the wrappers created for
/// statically advertised resources, this also supports resource-template URIs.
pub struct McpResourceReadTool {
    pub normalized_name: String,
    pub client: Arc<Mutex<McpClient>>,
    pub server_name: String,
    pub overrides: HashMap<String, PermissionLevel>,
}

#[async_trait]
impl Tool for McpResourceReadTool {
    fn name(&self) -> &str {
        &self.normalized_name
    }

    fn description(&self) -> &str {
        "Read an MCP resource by URI from this server"
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({
            "type": "object",
            "properties": {"uri": {"type": "string"}},
            "required": ["uri"],
            "additionalProperties": false
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        self.overrides
            .get("mcp_resource_read")
            .copied()
            .or_else(|| {
                permission_override(&self.overrides, &self.normalized_name, &self.server_name)
            })
            .unwrap_or(PermissionLevel::L0)
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(Self {
            normalized_name: self.normalized_name.clone(),
            client: self.client.clone(),
            server_name: self.server_name.clone(),
            overrides: self.overrides.clone(),
        })
    }

    async fn execute(&self, input: Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let Some(uri) = input.get("uri").and_then(Value::as_str) else {
            return ToolResult {
                content: "mcp_resource_read requires a string uri".into(),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            };
        };
        let mut client = self.client.lock().await;
        match client.read_resource(uri).await {
            Ok(content) => ToolResult {
                content: content.text,
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
            Err(error) => ToolResult {
                content: format!("Error reading MCP resource: {error}"),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }
}

#[async_trait]
impl Tool for McpResourceProvider {
    fn name(&self) -> &str {
        &self.normalized_name
    }

    fn description(&self) -> &str {
        self.mcp_resource
            .description
            .as_deref()
            .unwrap_or("MCP resource")
    }

    fn input_schema(&self) -> Value {
        serde_json::json!({"type": "object", "properties": {}})
    }

    fn permission_level(&self) -> PermissionLevel {
        if let Some(override_level) = self
            .overrides
            .get(&self.mcp_resource.name)
            .copied()
            .or_else(|| {
                permission_override(&self.overrides, &self.normalized_name, &self.server_name)
            })
        {
            return override_level;
        }
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(McpResourceProvider {
            uri: self.uri.clone(),
            normalized_name: self.normalized_name.clone(),
            mcp_resource: self.mcp_resource.clone(),
            client: self.client.clone(),
            server_name: self.server_name.clone(),
            overrides: self.overrides.clone(),
        })
    }

    async fn execute(&self, _input: Value, ctx: &ToolContext) -> ToolResult {
        let mut client = self.client.lock().await;
        let start = ctx.clock.mono_now();

        match client.read_resource(&self.uri).await {
            Ok(content) => ToolResult {
                content: content.text,
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
            Err(e) => ToolResult {
                content: format!("Error reading resource: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }
}

#[cfg(test)]
mod permission_override_tests {
    use super::*;

    #[test]
    fn exact_tool_override_takes_precedence_over_legacy_server_override() {
        let overrides = HashMap::from([
            ("github".to_string(), PermissionLevel::L1),
            ("github__delete_repo".to_string(), PermissionLevel::L3),
        ]);
        assert_eq!(
            permission_override(&overrides, "github__delete_repo", "github"),
            Some(PermissionLevel::L3)
        );
    }

    #[test]
    fn unrelated_tool_does_not_inherit_another_tools_override() {
        let overrides = HashMap::from([("github__delete_repo".to_string(), PermissionLevel::L3)]);
        assert_eq!(
            permission_override(&overrides, "github__list_repos", "github"),
            None
        );
    }

    #[test]
    fn legacy_server_override_remains_a_fallback() {
        let overrides = HashMap::from([("github".to_string(), PermissionLevel::L2)]);
        assert_eq!(
            permission_override(&overrides, "github__list_repos", "github"),
            Some(PermissionLevel::L2)
        );
    }
}
