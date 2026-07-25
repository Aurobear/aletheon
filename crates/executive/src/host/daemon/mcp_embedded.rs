// Embedded MCP host adapter.

//! Embedded MCP server — exposes body tools via MCP protocol.
//!
//! The MCP server listens on a Unix socket and responds to
//! `initialize`, `tools/list`, `tools/call`, and `ping` methods.
//! Tools are dynamically sourced from the ToolRegistry.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::{json, Value};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixListener;
use tracing::{error, info, warn};

use crate::application::CapabilityService;

/// Embedded MCP server that exposes body tools via MCP protocol.
pub struct McpEmbedded {
    corpus: Arc<dyn corpus::CorpusService>,
    grant: corpus::ExtensionGrant,
    capability: Arc<dyn CapabilityService>,
    socket_path: PathBuf,
}

impl McpEmbedded {
    pub fn new(
        corpus: Arc<dyn corpus::CorpusService>,
        grant: corpus::ExtensionGrant,
        capability: Arc<dyn CapabilityService>,
        socket_path: PathBuf,
    ) -> Self {
        Self {
            corpus,
            grant,
            capability,
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
                    let corpus = self.corpus.clone();
                    let grant = self.grant.clone();
                    let capability = self.capability.clone();
                    tokio::spawn(async move {
                        if let Err(e) =
                            Self::handle_connection(stream, corpus, grant, capability).await
                        {
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
        corpus: Arc<dyn corpus::CorpusService>,
        grant: corpus::ExtensionGrant,
        capability: Arc<dyn CapabilityService>,
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

            let response = Self::handle_request(&request, &corpus, &grant, &capability).await;
            let response_str = serde_json::to_string(&response)?;
            writer.write_all(response_str.as_bytes()).await?;
            writer.write_all(b"\n").await?;
            writer.flush().await?;
        }

        Ok(())
    }

    async fn handle_request(
        request: &Value,
        corpus: &Arc<dyn corpus::CorpusService>,
        grant: &corpus::ExtensionGrant,
        capability: &Arc<dyn CapabilityService>,
    ) -> Value {
        let method = request.get("method").and_then(|v| v.as_str()).unwrap_or("");
        let id = request.get("id").cloned().unwrap_or(Value::Null);

        match method {
            "initialize" => Self::handle_initialize(id),
            "tools/list" => Self::handle_tools_list(id, corpus, grant).await,
            "tools/call" => Self::handle_tools_call(id, request, corpus, grant, capability).await,
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

    async fn handle_tools_list(
        id: Value,
        corpus: &Arc<dyn corpus::CorpusService>,
        grant: &corpus::ExtensionGrant,
    ) -> Value {
        let snapshot = match corpus.catalog(grant).await {
            Ok(snapshot) => snapshot,
            Err(error) => {
                return json!({"jsonrpc":"2.0","id":id,"error":{"code":-32000,"message":error.to_string()}})
            }
        };
        let tools: Vec<Value> = snapshot
            .entries
            .into_iter()
            .filter_map(|entry| entry.tool_definition)
            .map(|definition| {
                json!({
                    "name": definition.name,
                    "description": definition.description,
                    "inputSchema": definition.input_schema,
                })
            })
            .collect();

        json!({
            "jsonrpc": "2.0",
            "id": id,
            "result": {"tools": tools}
        })
    }

    async fn handle_tools_call(
        id: Value,
        request: &Value,
        corpus: &Arc<dyn corpus::CorpusService>,
        grant: &corpus::ExtensionGrant,
        capability: &Arc<dyn CapabilityService>,
    ) -> Value {
        let params = request.get("params").cloned().unwrap_or(json!({}));
        let tool_name = params.get("name").and_then(|v| v.as_str()).unwrap_or("");
        let arguments = params.get("arguments").cloned().unwrap_or(json!({}));

        let known = corpus
            .catalog(grant)
            .await
            .map(|snapshot| {
                snapshot.entries.iter().any(|entry| {
                    entry
                        .capabilities
                        .iter()
                        .any(|capability| capability.0 == tool_name)
                })
            })
            .unwrap_or(false);
        if !known {
            return json!({
                "jsonrpc": "2.0",
                "id": id,
                "error": {"code": -32602, "message": format!("Unknown tool: {}", tool_name)}
            });
        }

        let result = capability
            .invoke(
                None,
                fabric::CapabilityCall {
                    operation_id: fabric::OperationId::default(),
                    process_id: fabric::ProcessId::default(),
                    name: tool_name.to_string(),
                    input: arguments,
                    call_id: id.to_string(),
                    deadline: None,
                },
                tokio_util::sync::CancellationToken::new(),
            )
            .await;
        let content_text = if result.is_error {
            format!("Error: {}", result.output)
        } else {
            result.output
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
    use kernel::capability::ToolExecutor;

    struct RejectCapability;

    #[async_trait::async_trait]
    impl CapabilityService for RejectCapability {
        async fn invoke(
            &self,
            _context: Option<crate::application::CapabilityExecutionContext>,
            call: fabric::CapabilityCall,
            _cancel: tokio_util::sync::CancellationToken,
        ) -> fabric::CapabilityResult {
            fabric::CapabilityResult {
                call_id: call.call_id,
                output: "unused".into(),
                is_error: true,
                usage: fabric::UsageReport::default(),
                audit_id: None,
                patch_delta: None,
            }
        }
    }

    fn capability() -> Arc<dyn CapabilityService> {
        Arc::new(RejectCapability)
    }

    struct RejectExecutor;

    #[async_trait::async_trait]
    impl ToolExecutor for RejectExecutor {
        async fn execute_with_permit(
            &self,
            request: &fabric::CapabilityRequest,
            _permit: &fabric::ExecutionPermit,
        ) -> fabric::CapabilityResult {
            fabric::CapabilityResult {
                call_id: request.call.call_id.clone(),
                output: "unused".into(),
                is_error: true,
                usage: Default::default(),
                audit_id: None,
                patch_delta: None,
            }
        }
    }

    fn corpus(with_tool: bool) -> (Arc<dyn corpus::CorpusService>, corpus::ExtensionGrant) {
        let mut capabilities = Vec::new();
        let catalog = if with_tool {
            capabilities.push(fabric::CapabilityId("bash_exec".into()));
            corpus::ExtensionCatalog::new([corpus::ExtensionDescriptor::new(
                corpus::ExtensionKind::Tool,
                "bash_exec",
                "1",
                "execute bash",
                fabric::CapabilityId("bash_exec".into()),
                fabric::types::admission::RiskLevel::SystemModify,
            )
            .unwrap()
            .with_tool_definition(fabric::ToolDefinition {
                name: "bash_exec".into(),
                description: "execute bash".into(),
                input_schema: json!({"type":"object"}),
            })
            .unwrap()])
            .unwrap()
        } else {
            corpus::ExtensionCatalog::default()
        };
        (
            Arc::new(corpus::DefaultCorpusService::new(
                catalog,
                Arc::new(RejectExecutor),
            )),
            corpus::ExtensionGrant {
                grant_id: "test".into(),
                principal: fabric::PrincipalId("test".into()),
                session_id: "test".into(),
                agent_id: None,
                capabilities,
                resources: Default::default(),
            },
        )
    }

    #[tokio::test]
    async fn handle_initialize_returns_server_info() {
        let (corpus, grant) = corpus(false);
        let request = json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {}});
        let response = McpEmbedded::handle_request(&request, &corpus, &grant, &capability()).await;

        assert_eq!(response["result"]["protocolVersion"], "2024-11-05");
        assert_eq!(
            response["result"]["serverInfo"]["name"],
            "aletheon-embedded-mcp"
        );
    }

    #[tokio::test]
    async fn handle_tools_list_returns_registry_tools() {
        let (corpus, grant) = corpus(true);

        let request = json!({"jsonrpc": "2.0", "id": 2, "method": "tools/list", "params": {}});
        let response = McpEmbedded::handle_request(&request, &corpus, &grant, &capability()).await;

        let tools = response["result"]["tools"].as_array().unwrap();
        assert!(tools.iter().any(|t| t["name"] == "bash_exec"));
    }

    #[tokio::test]
    async fn handle_ping() {
        let (corpus, grant) = corpus(false);
        let request = json!({"jsonrpc": "2.0", "id": 3, "method": "ping"});
        let response = McpEmbedded::handle_request(&request, &corpus, &grant, &capability()).await;
        assert!(response["result"].is_object());
    }

    #[tokio::test]
    async fn handle_unknown_method() {
        let (corpus, grant) = corpus(false);
        let request = json!({"jsonrpc": "2.0", "id": 4, "method": "unknown"});
        let response = McpEmbedded::handle_request(&request, &corpus, &grant, &capability()).await;
        assert_eq!(response["error"]["code"], -32601);
    }

    #[tokio::test]
    async fn handle_tools_call_unknown_tool() {
        let (corpus, grant) = corpus(false);
        let request = json!({
            "jsonrpc": "2.0", "id": 5, "method": "tools/call",
            "params": {"name": "nonexistent", "arguments": {}}
        });
        let response = McpEmbedded::handle_request(&request, &corpus, &grant, &capability()).await;
        assert_eq!(response["error"]["code"], -32602);
    }
}
