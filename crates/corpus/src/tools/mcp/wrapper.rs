use std::collections::HashMap;
use std::sync::Arc;

use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use super::client::{McpClient, McpTool};
use super::config::McpTrustLevel;
use crate::tools::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

/// Wraps an MCP-discovered tool as a local `Tool` implementation.
pub struct McpToolWrapper {
    pub normalized_name: String,
    pub mcp_tool: McpTool,
    pub client: Arc<Mutex<McpClient>>,
    pub trust_level: McpTrustLevel,
    /// The server this tool belongs to, used for permission override lookup.
    pub server_name: String,
    /// Per-server permission level overrides (server_name → PermissionLevel).
    /// If the tool's server has an entry, it overrides the trust→permission mapping.
    pub overrides: HashMap<String, PermissionLevel>,
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
        // Check for override first
        if let Some(override_level) = self.overrides.get(&self.server_name) {
            return *override_level;
        }
        // Fall back to trust-based mapping
        match self.trust_level {
            McpTrustLevel::LocalTrusted => PermissionLevel::L1,
            McpTrustLevel::RemoteTrusted => PermissionLevel::L1,
            McpTrustLevel::Untrusted => PermissionLevel::L0,
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
