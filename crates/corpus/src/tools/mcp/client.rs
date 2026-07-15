use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use serde_json::Value;
use tokio::sync::Mutex;

use super::auth::BearerTokenAuth;
use super::config::{McpConfig, McpTransportConfig, McpTrustLevel};
use super::transport::McpTransport;
use super::wrapper::McpToolWrapper;

/// Tool discovered from an MCP server.
#[derive(Debug, Clone)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// MCP client for a single server.
pub struct McpClient {
    pub server_name: String,
    transport: McpTransport,
    next_id: u64,
    pub trust_level: McpTrustLevel,
    pub tools: Vec<McpTool>,
}

impl McpClient {
    /// Connect to an MCP server via stdio and discover its tools.
    pub async fn connect_stdio(
        server_name: String,
        command: &str,
        args: &[String],
        trust_level: McpTrustLevel,
    ) -> Result<Self> {
        let mut transport = McpTransport::stdio(command, args).await?;

        // Initialize handshake
        let _init_result = transport
            .request(
                1,
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "aletheon", "version": "0.1.0" }
                }),
            )
            .await?;

        tracing::info!(server = %server_name, "MCP server initialized");

        // Send initialized notification
        // (notifications have no id; we reuse request with a throwaway id for simplicity)

        // Discover tools
        let tools_result = transport
            .request(2, "tools/list", serde_json::json!({}))
            .await?;
        let tools = Self::parse_tools(&tools_result);

        tracing::info!(server = %server_name, count = tools.len(), "MCP tools discovered");

        Ok(Self {
            server_name,
            transport,
            next_id: 3,
            trust_level,
            tools,
        })
    }

    /// Connect to an MCP server via Streamable HTTP and discover its tools.
    pub async fn connect_http(
        server_name: String,
        url: &str,
        auth: Option<BearerTokenAuth>,
        trust_level: McpTrustLevel,
    ) -> Result<Self> {
        let mut transport = McpTransport::streamable_http(url, auth);

        // Initialize handshake
        let _init_result = transport
            .request(
                1,
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "aletheon", "version": "0.1.0" }
                }),
            )
            .await?;

        tracing::info!(server = %server_name, "MCP HTTP server initialized");

        // Discover tools
        let tools_result = transport
            .request(2, "tools/list", serde_json::json!({}))
            .await?;
        let tools = Self::parse_tools(&tools_result);

        tracing::info!(server = %server_name, count = tools.len(), "MCP HTTP tools discovered");

        Ok(Self {
            server_name,
            transport,
            next_id: 3,
            trust_level,
            tools,
        })
    }

    /// Connect to an MCP server via SSE and discover its tools.
    pub async fn connect_sse(
        server_name: String,
        url: &str,
        auth: Option<BearerTokenAuth>,
        trust_level: McpTrustLevel,
    ) -> Result<Self> {
        let mut transport = McpTransport::sse(url, auth).await?;

        // Initialize handshake
        let _init_result = transport
            .request(
                1,
                "initialize",
                serde_json::json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {},
                    "clientInfo": { "name": "aletheon", "version": "0.1.0" }
                }),
            )
            .await?;

        tracing::info!(server = %server_name, "MCP SSE server initialized");

        // Discover tools
        let tools_result = transport
            .request(2, "tools/list", serde_json::json!({}))
            .await?;
        let tools = Self::parse_tools(&tools_result);

        tracing::info!(server = %server_name, count = tools.len(), "MCP SSE tools discovered");

        Ok(Self {
            server_name,
            transport,
            next_id: 3,
            trust_level,
            tools,
        })
    }

    fn parse_tools(result: &Value) -> Vec<McpTool> {
        let mut tools = Vec::new();
        if let Some(tools_array) = result.get("tools").and_then(|v| v.as_array()) {
            for tool_val in tools_array {
                let name = tool_val
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let description = tool_val
                    .get("description")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let input_schema = tool_val
                    .get("inputSchema")
                    .cloned()
                    .unwrap_or(serde_json::json!({}));
                if !name.is_empty() {
                    tools.push(McpTool {
                        name,
                        description,
                        input_schema,
                    });
                }
            }
        }
        tools
    }

    /// Call a tool on this server.
    pub async fn call_tool(&mut self, tool_name: &str, args: Value) -> Result<Value> {
        let id = self.next_id;
        self.next_id += 1;
        let result = self
            .transport
            .request(
                id,
                "tools/call",
                serde_json::json!({
                    "name": tool_name,
                    "arguments": args,
                }),
            )
            .await?;
        if result
            .get("isError")
            .and_then(Value::as_bool)
            .unwrap_or(false)
        {
            // Do not copy server-provided text into the error: it may contain
            // credentials or other sensitive request context.
            anyhow::bail!("MCP tool '{}' reported an application error", tool_name);
        }
        Ok(result)
    }
}

/// Manages connections to multiple MCP servers.
pub struct McpConnectionManager {
    clients: HashMap<String, Arc<Mutex<McpClient>>>,
    config: McpConfig,
}

impl McpConnectionManager {
    pub fn new(config: McpConfig) -> Self {
        Self {
            clients: HashMap::new(),
            config,
        }
    }

    /// Connect to all enabled servers in the config.
    pub async fn connect_all(&mut self) -> Result<()> {
        for server_config in &self.config.servers {
            if !server_config.enabled {
                continue;
            }

            // Resolve bearer token from environment
            let auth: Option<BearerTokenAuth> = server_config
                .bearer_token_env
                .as_ref()
                .map(|env_var| BearerTokenAuth::new(env_var.clone()));

            match &server_config.transport {
                McpTransportConfig::Stdio { command, args } => {
                    match McpClient::connect_stdio(
                        server_config.name.clone(),
                        command,
                        args,
                        server_config.trust,
                    )
                    .await
                    {
                        Ok(client) => {
                            self.clients
                                .insert(server_config.name.clone(), Arc::new(Mutex::new(client)));
                        }
                        Err(e) => {
                            tracing::warn!(
                                server = %server_config.name,
                                error = %e,
                                "Failed to connect MCP server"
                            );
                        }
                    }
                }
                McpTransportConfig::StreamableHttp { url } => {
                    match McpClient::connect_http(
                        server_config.name.clone(),
                        url,
                        auth.clone(),
                        server_config.trust,
                    )
                    .await
                    {
                        Ok(client) => {
                            self.clients
                                .insert(server_config.name.clone(), Arc::new(Mutex::new(client)));
                        }
                        Err(e) => {
                            tracing::warn!(
                                server = %server_config.name,
                                error = %e,
                                "Failed to connect MCP server via HTTP"
                            );
                        }
                    }
                }
                McpTransportConfig::Sse { url } => {
                    match McpClient::connect_sse(
                        server_config.name.clone(),
                        url,
                        auth.clone(),
                        server_config.trust,
                    )
                    .await
                    {
                        Ok(client) => {
                            self.clients
                                .insert(server_config.name.clone(), Arc::new(Mutex::new(client)));
                        }
                        Err(e) => {
                            tracing::warn!(
                                server = %server_config.name,
                                error = %e,
                                "Failed to connect MCP server via SSE"
                            );
                        }
                    }
                }
            }
        }
        Ok(())
    }

    /// Get all tool wrappers from all connected servers.
    pub fn get_all_tools(&self) -> Vec<McpToolWrapper> {
        let mut wrappers = Vec::new();
        for (server_name, client_arc) in &self.clients {
            // We need to block briefly to read the tools list.
            // Since connect_all has already completed, the mutex is uncontested.
            let Ok(client) = client_arc.try_lock() else {
                tracing::warn!(server = %server_name, "MCP client busy during tool discovery");
                continue;
            };
            let prefix = if self.config.tool_name_prefix {
                format!("{}__", server_name)
            } else {
                String::new()
            };
            for tool in &client.tools {
                let normalized_name = if prefix.is_empty() {
                    tool.name.clone()
                } else {
                    let full = format!("{}{}", prefix, tool.name);
                    if full.len() > self.config.max_tool_name_length {
                        full[..self.config.max_tool_name_length].to_string()
                    } else {
                        full
                    }
                };
                wrappers.push(McpToolWrapper {
                    normalized_name,
                    mcp_tool: tool.clone(),
                    client: client_arc.clone(),
                    trust_level: client.trust_level,
                });
            }
        }
        wrappers
    }

    /// Get a reference to a specific client by server name.
    pub fn get_client(&self, server_name: &str) -> Option<&Arc<Mutex<McpClient>>> {
        self.clients.get(server_name)
    }

    /// Call a tool on a named server and return the raw result.
    ///
    /// Returns an error if the server is not connected or the tool call fails.
    /// Token values never appear in error messages.
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        args: Value,
    ) -> Result<Value> {
        let client_arc = self
            .clients
            .get(server_name)
            .with_context(|| format!("MCP server '{}' is not connected", server_name))?;
        let mut client = client_arc.lock().await;
        client.call_tool(tool_name, args).await
    }

    /// Number of connected servers.
    pub fn connected_count(&self) -> usize {
        self.clients.len()
    }

    /// Whether a connected server advertised every required tool.
    pub fn server_has_tools(&self, server_name: &str, required: &[&str]) -> bool {
        self.clients.get(server_name).is_some_and(|client| {
            let Ok(client) = client.try_lock() else {
                return false;
            };
            required
                .iter()
                .all(|name| client.tools.iter().any(|tool| tool.name == *name))
        })
    }

    /// Snapshot advertised tool descriptors without exposing the live client.
    pub fn server_tools(&self, server_name: &str) -> Option<Vec<McpTool>> {
        let client = self.clients.get(server_name)?.try_lock().ok()?;
        Some(client.tools.clone())
    }
}
