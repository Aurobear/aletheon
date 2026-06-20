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
        match self.trust_level {
            McpTrustLevel::LocalTrusted => PermissionLevel::L0,
            McpTrustLevel::RemoteTrusted => PermissionLevel::L1,
            McpTrustLevel::Untrusted => PermissionLevel::L2,
        }
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(McpToolWrapper {
            normalized_name: self.normalized_name.clone(),
            mcp_tool: self.mcp_tool.clone(),
            client: self.client.clone(),
            trust_level: self.trust_level,
        })
    }

    async fn execute(&self, input: Value, _ctx: &ToolContext) -> ToolResult {
        let mut client = self.client.lock().await;
        let start = std::time::Instant::now();

        match client.call_tool(&self.mcp_tool.name, input).await {
            Ok(response) => {
                let content = serde_json::to_string_pretty(&response)
                    .unwrap_or_else(|_| format!("{:?}", response));
                ToolResult {
                    content,
                    is_error: false,
                    metadata: ToolResultMeta {
                        execution_time_ms: start.elapsed().as_millis() as u64,
                        truncated: false,
                    },
                }
            }
            Err(e) => ToolResult {
                content: format!("MCP tool error: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                },
            },
        }
    }
}
