// crates/aletheon-runtime/src/impl/daemon/mcp_embedded.rs

//! Embedded MCP server — exposes body tools via MCP protocol.
//!
//! The MCP server listens on a Unix socket and responds to
//! `initialize`, `tools/list`, `tools/call`, and `ping` methods.
//! Tools are dynamically sourced from the ToolRegistry.

use std::path::PathBuf;
use std::sync::Arc;

use aletheon_abi::Registry;
use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tracing::{error, info, warn};

use aletheon_body::r#impl::tools::ToolRegistry;

/// Embedded MCP server that exposes body tools via MCP protocol.
pub struct McpEmbedded {
    tool_registry: Arc<ToolRegistry>,
    socket_path: PathBuf,
}

impl McpEmbedded {
    pub fn new(tool_registry: Arc<ToolRegistry>, socket_path: PathBuf) -> Self {
        Self {
            tool_registry,
            socket_path,
        }
    }

    /// Start the MCP server, listening on a Unix socket.
    pub async fn serve(&self) -> anyhow::Result<()> {
        if self.socket_path.exists() {
            std::fs::remove_file(&self.socket_path)?;
        }

        let listener = UnixListener::bind(&self.socket_path)?;
        info!(path = %self.socket_path.display(), "MCP server listening");

        loop {
            match listener.accept().await {
                Ok((stream, _)) => {
                    let registry = self.tool_registry.clone();
                    tokio::spawn(async move {
                        if let Err(e) = Self::handle_connection(stream, registry).await {
                            warn!(error = %e, "MCP connection error");
                        }
                    });
                }
                Err(e) => {
                    error!(error = %e, "MCP accept error");
                }
            }
        }
    }

    async fn handle_connection(
        stream: tokio::net::UnixStream,
        registry: Arc<ToolRegistry>,
    ) -> anyhow::Result<()> {
        let (reader, mut writer) = stream.into_split();
        let mut reader = BufReader::new(reader);
        let mut line = String::new();

        loop {
            line.clear();
            if reader.read_line(&mut line).await? == 0 {
                break;
            }

            let request: Value = match serde_json::from_str(line.trim()) {
                Ok(v) => v,
                Err(e) => {
                    warn!(error = %e, "Invalid JSON-RPC request");
                    continue;
                }
            };

            let response = Self::handle_request(&request, &registry).await;
            let response_str = serde_json::to_string(&response)?;
            writer.write_all(response_str.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }

        Ok(())
    }

    async fn handle_request(request: &Value, registry: &Arc<ToolRegistry>) -> Value {
        let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = request.get("id").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => Self::handle_initialize(id),
            "tools/list" => Self::handle_tools_list(id, registry),
            "tools/call" => Self::handle_tools_call(id, request, registry).await,
            "ping" => json!({"jsonrpc": "2.0", "id": id, "result": {}}),
            _ => json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32601, "message": format!("Method not found: {}", method)}
            }),
        }
    }

    fn handle_initialize(id: Value) -> Value {
        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "protocolVersion": "2024-11-05",
                "capabilities": {"tools": {}},
                "serverInfo": {
                    "name": "aletheon-embedded-mcp",
                    "version": env!("CARGO_PKG_VERSION")
                }
            }
        })
    }

    fn handle_tools_list(id: Value, registry: &Arc<ToolRegistry>) -> Value {
        let tools: Vec<Value> = registry
            .definitions()
            .into_iter()
            .map(|def| {
                json!({
                    "name": def.name,
                    "description": def.description,
                    "inputSchema": def.input_schema,
                })
            })
            .collect();

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"tools": tools}
        })
    }

    async fn handle_tools_call(id: Value, request: &Value, registry: &Arc<ToolRegistry>) -> Value {
        let params = request.get("params").cloned().unwrap_or(json!({}));
        let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        let tool = match registry.get(tool_name) {
            Some(t) => t,
            None => {
                return json!({
                    "jsonrpc": "2.0",
                    "id": id,
                    "error": {"code": -32602, "message": format!("Unknown tool: {}", tool_name)}
                });
            }
        };

        let ctx = aletheon_abi::tool::ToolContext {
            working_dir: std::env::current_dir().unwrap_or_default(),
            session_id: "mcp-session".into(),
        };

        let result = tool.execute(arguments, &ctx).await;
        let content_text = if result.is_error {
            format!("Error: {}", result.content)
        } else {
            result.content
        };

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {
                "content": [{"type": "text", "text": content_text}],
                "isError": result.is_error
            }
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn handle_initialize_returns_server_info() {
        let registry = Arc::new(ToolRegistry::new());
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
        let rt = tokio::runtime::Runtime::new().unwrap();
        let response = rt.block_on(McpEmbedded::handle_request(&request, &registry));

        assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(response["result"]["serverInfo"]["name"], "aletheon-embedded-mcp");
    }

    #[test]
    fn handle_tools_list_returns_registry_tools() {
        let mut reg = ToolRegistry::new();
        reg.register(Arc::new(aletheon_body::r#impl::tools::bash_exec::BashExecTool));
        let registry = Arc::new(reg);

        let request = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}});
        let rt = tokio::runtime::Runtime::new().unwrap();
        let response = rt.block_on(McpEmbedded::handle_request(&request, &registry));

        let tools = response["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "bash_exec"));
    }

    #[test]
    fn handle_ping() {
        let registry = Arc::new(ToolRegistry::new());
        let request = json!({"jsonrpc": "2.0", "id": 3, "method": "ping"});
        let rt = tokio::runtime::Runtime::new().unwrap();
        let response = rt.block_on(McpEmbedded::handle_request(&request, &registry));
        assert!(response["result"].is_object());
    }

    #[test]
    fn handle_unknown_method() {
        let registry = Arc::new(ToolRegistry::new());
        let request = json!({"jsonrpc": "2.0", "id": 4, "method": "unknown"});
        let rt = tokio::runtime::Runtime::new().unwrap();
        let response = rt.block_on(McpEmbedded::handle_request(&request, &registry));
        assert_eq!(response["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn handle_tools_call_unknown_tool() {
        let registry = Arc::new(ToolRegistry::new());
        let request = json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": {"name": "nonexistent", "arguments": {}}
        });
        let response = McpEmbedded::handle_request(&request, &registry).await;
        assert_eq!(response["error"]["code"], -32602);
    }
}
