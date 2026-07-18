use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{Mutex, mpsc};

use super::auth::BearerTokenAuth;
use super::config::{McpConfig, McpTransportConfig, McpTrustLevel};
use super::transport::{McpNotification, McpTransport};
use super::wrapper::{McpResourceProvider, McpToolWrapper};

// ---------------------------------------------------------------------------
// Elicitation handler trait
// ---------------------------------------------------------------------------

/// Handler for MCP elicitation requests received from servers.
///
/// When an MCP server sends an `elicitation/create` request (JSON-RPC
/// server-to-client), the client invokes this handler to obtain the
/// user's approval decision. Implementations may forward to external
/// approval systems (e.g. `SocketApprovalGate`) or use inline policies.
///
/// # Fail-safe
///
/// If the handler is absent, the client auto-denies the elicitation.
#[async_trait]
pub trait ElicitationHandler: Send + Sync {
    /// Handle an elicitation request and return the user's decision.
    ///
    /// `message` — the user-facing prompt from the server.
    /// `mode` — the elicitation mode (e.g. `"once"`, `"always"`).
    ///
    /// Returns `true` if the user approved, `false` if denied.
    async fn handle_elicitation(&self, message: &str, mode: &str) -> Result<bool, String>;
}

/// Tool discovered from an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpTool {
    pub name: String,
    pub description: String,
    pub input_schema: Value,
}

/// Resource content returned by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceContent {
    pub uri: String,
    #[serde(default)]
    pub mime_type: Option<String>,
    pub text: String,
}

/// Resource discovered from an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResource {
    pub uri: String,
    pub name: String,
    #[serde(default)]
    pub description: Option<String>,
    #[serde(default)]
    pub mime_type: Option<String>,
}

/// MCP client for a single server.
pub struct McpClient {
    pub server_name: String,
    transport: McpTransport,
    next_id: u64,
    pub trust_level: McpTrustLevel,
    pub tools: Vec<McpTool>,
    pub resources: Vec<McpResource>,
    /// Channel sender for notifications received from the server.
    pub notification_tx: mpsc::Sender<McpNotification>,
    /// Whether the server supports parallel tool calls.
    pub supports_parallel_tool_calls: bool,
    /// Handler for elicitation requests received from the server.
    /// If `None` (default), elicitation requests are auto-denied.
    pub elicitation_handler: Option<Arc<dyn ElicitationHandler>>,
}

impl McpClient {
    /// Connect to an MCP server via stdio and discover its tools.
    pub async fn connect_stdio(
        server_name: String,
        command: &str,
        args: &[String],
        trust_level: McpTrustLevel,
    ) -> Result<Self> {
        let (notification_tx, _notification_rx) = mpsc::channel(64);
        let mut transport = McpTransport::stdio(command, args).await?;

        // Initialize handshake
        let init_result = transport
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

        let supports_parallel_tool_calls = init_result
            .get("capabilities")
            .and_then(|c| c.get("tools"))
            .and_then(|t| t.get("supports_parallel_tool_calls"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        tracing::info!(server = %server_name, "MCP server initialized");

        // Send initialized notification
        // (notifications have no id; we reuse request with a throwaway id for simplicity)

        // Discover tools
        let tools_result = transport
            .request(2, "tools/list", serde_json::json!({}))
            .await?;
        let tools = Self::parse_tools(&tools_result);

        tracing::info!(server = %server_name, count = tools.len(), "MCP tools discovered");

        // Discover resources
        let resources = Self::discover_resources(&mut transport, &server_name, 3).await
            .unwrap_or_default();

        Ok(Self {
            server_name,
            transport,
            next_id: 4,
            trust_level,
            tools,
            resources,
            notification_tx,
            supports_parallel_tool_calls,
            elicitation_handler: None,
        })
    }

    /// Connect to an MCP server via Streamable HTTP and discover its tools.
    pub async fn connect_http(
        server_name: String,
        url: &str,
        auth: Option<BearerTokenAuth>,
        trust_level: McpTrustLevel,
    ) -> Result<Self> {
        let (notification_tx, _notification_rx) = mpsc::channel(64);
        let mut transport = McpTransport::streamable_http(url, auth);

        // Initialize handshake
        let init_result = transport
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

        let supports_parallel_tool_calls = init_result
            .get("capabilities")
            .and_then(|c| c.get("tools"))
            .and_then(|t| t.get("supports_parallel_tool_calls"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        tracing::info!(server = %server_name, "MCP HTTP server initialized");

        // Discover tools
        let tools_result = transport
            .request(2, "tools/list", serde_json::json!({}))
            .await?;
        let tools = Self::parse_tools(&tools_result);

        tracing::info!(server = %server_name, count = tools.len(), "MCP HTTP tools discovered");

        // Discover resources
        let resources = Self::discover_resources(&mut transport, &server_name, 3).await
            .unwrap_or_default();

        Ok(Self {
            server_name,
            transport,
            next_id: 4,
            trust_level,
            tools,
            resources,
            notification_tx,
            supports_parallel_tool_calls,
            elicitation_handler: None,
        })
    }

    /// Connect to an MCP server via SSE and discover its tools.
    pub async fn connect_sse(
        server_name: String,
        url: &str,
        auth: Option<BearerTokenAuth>,
        trust_level: McpTrustLevel,
    ) -> Result<Self> {
        let (notification_tx, _notification_rx) = mpsc::channel(64);
        let mut transport = McpTransport::sse(url, auth).await?;

        // Initialize handshake
        let init_result = transport
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

        let supports_parallel_tool_calls = init_result
            .get("capabilities")
            .and_then(|c| c.get("tools"))
            .and_then(|t| t.get("supports_parallel_tool_calls"))
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        tracing::info!(server = %server_name, "MCP SSE server initialized");

        // Discover tools
        let tools_result = transport
            .request(2, "tools/list", serde_json::json!({}))
            .await?;
        let tools = Self::parse_tools(&tools_result);

        tracing::info!(server = %server_name, count = tools.len(), "MCP SSE tools discovered");

        // Discover resources
        let resources = Self::discover_resources(&mut transport, &server_name, 3).await
            .unwrap_or_default();

        Ok(Self {
            server_name,
            transport,
            next_id: 4,
            trust_level,
            tools,
            resources,
            notification_tx,
            supports_parallel_tool_calls,
            elicitation_handler: None,
        })
    }

    /// Getter for supports_parallel_tool_calls.
    pub fn supports_parallel_tool_calls(&self) -> bool {
        self.supports_parallel_tool_calls
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

    /// Discover resources from the server via `resources/list`.
    async fn discover_resources(
        transport: &mut McpTransport,
        server_name: &str,
        starting_id: u64,
    ) -> Result<Vec<McpResource>> {
        let result = transport
            .request(starting_id, "resources/list", serde_json::json!({}))
            .await;
        match result {
            Ok(response) => {
                let resources = Self::parse_resources(&response);
                tracing::info!(
                    server = %server_name,
                    count = resources.len(),
                    "MCP resources discovered"
                );
                Ok(resources)
            }
            Err(e) => {
                tracing::debug!(
                    server = %server_name,
                    error = %e,
                    "MCP resources/list not supported by server"
                );
                Ok(Vec::new())
            }
        }
    }

    fn parse_resources(result: &Value) -> Vec<McpResource> {
        let mut resources = Vec::new();
        if let Some(resources_array) = result.get("resources").and_then(|v| v.as_array()) {
            for res_val in resources_array {
                let uri = res_val
                    .get("uri")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let name = res_val
                    .get("name")
                    .and_then(|v| v.as_str())
                    .unwrap_or("")
                    .to_string();
                let description = res_val
                    .get("description")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                let mime_type = res_val
                    .get("mimeType")
                    .and_then(|v| v.as_str())
                    .map(|s| s.to_string());
                if !uri.is_empty() && !name.is_empty() {
                    resources.push(McpResource {
                        uri,
                        name,
                        description,
                        mime_type,
                    });
                }
            }
        }
        resources
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

    /// List available resources from this server.
    pub async fn list_resources(&mut self) -> Result<Vec<McpResource>> {
        let id = self.next_id;
        self.next_id += 1;
        let result = self
            .transport
            .request(id, "resources/list", serde_json::json!({}))
            .await?;
        Ok(Self::parse_resources(&result))
    }

    /// Read a specific resource by URI.
    pub async fn read_resource(&mut self, uri: &str) -> Result<super::client::ResourceContent> {
        let id = self.next_id;
        self.next_id += 1;
        let result = self
            .transport
            .request(
                id,
                "resources/read",
                serde_json::json!({"uri": uri}),
            )
            .await?;

        // MCP resources/read returns the content array
        let text = if let Some(contents) = result.get("contents").and_then(|v| v.as_array()) {
            contents
                .iter()
                .filter_map(|c| {
                    let mime_type = c.get("mimeType").and_then(|v| v.as_str()).unwrap_or("text/plain");
                    // Prioritize "text" field, fall back to "blob" (but blob is base64 so we note it)
                    if let Some(t) = c.get("text").and_then(|v| v.as_str()) {
                        Some(t.to_string())
                    } else if let Some(_b) = c.get("blob").and_then(|v| v.as_str()) {
                        Some(format!("[base64 blob: mimeType={}]", mime_type))
                    } else {
                        None
                    }
                })
                .collect::<Vec<_>>()
                .join("")
        } else {
            String::new()
        };

        let mime_type = result
            .get("contents")
            .and_then(|v| v.as_array())
            .and_then(|arr| arr.first())
            .and_then(|c| c.get("mimeType"))
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        Ok(super::client::ResourceContent {
            uri: uri.to_string(),
            mime_type,
            text,
        })
    }

    /// Watch for `ToolsListChanged` notifications and re-discover tools.
    ///
    /// Callers should spawn this as a background task, passing the receive
    /// side of the notification channel created at connect time.
    pub async fn watch_tool_changes(&mut self, mut rx: mpsc::Receiver<McpNotification>) {
        while let Some(notification) = rx.recv().await {
            if matches!(notification, McpNotification::ToolsListChanged) {
                let id = self.next_id;
                self.next_id += 1;
                match self
                    .transport
                    .request(id, "tools/list", serde_json::json!({}))
                    .await
                {
                    Ok(result) => {
                        let tools = Self::parse_tools(&result);
                        tracing::info!(
                            server = %self.server_name,
                            old_count = self.tools.len(),
                            new_count = tools.len(),
                            "MCP tools re-discovered after ToolsListChanged"
                        );
                        self.tools = tools;
                    }
                    Err(e) => {
                        tracing::warn!(
                            server = %self.server_name,
                            error = %e,
                            "Failed to re-discover tools after ToolsListChanged"
                        );
                    }
                }
            }
        }
    }

    /// Handle an MCP `elicitation/create` request from the server.
    ///
    /// The server sends this request when it needs user approval for an
    /// action. The client delegates to the configured [`ElicitationHandler`]
    /// if one is set; otherwise the elicitation is auto-denied (fail-safe).
    ///
    /// Returns an `elicitation/create` JSON-RPC response containing the
    /// user's decision: `"allow"` or `"deny"`.
    pub async fn handle_elicitation(&self, params: &Value) -> Result<Value> {
        let message = params
            .get("message")
            .and_then(|v| v.as_str())
            .unwrap_or("MCP server requires your approval");
        let mode = params
            .get("mode")
            .and_then(|v| v.as_str())
            .unwrap_or("once");

        let approved = match &self.elicitation_handler {
            Some(handler) => {
                match handler.handle_elicitation(message, mode).await {
                    Ok(allowed) => allowed,
                    Err(e) => {
                        tracing::warn!(
                            server = %self.server_name,
                            error = %e,
                            "Elicitation handler returned an error; defaulting to deny"
                        );
                        false
                    }
                }
            }
            None => {
                tracing::debug!(
                    server = %self.server_name,
                    message = %message,
                    mode = %mode,
                    "No elicitation handler configured; auto-denying elicitation request"
                );
                false
            }
        };

        Ok(serde_json::json!({
            "action": if approved { "allow" } else { "deny" }
        }))
    }
}

// ---------------------------------------------------------------------------
// McpElicitationHandler — bridges `ElicitationHandler` to `ApprovalGate`
// ---------------------------------------------------------------------------

/// An [`ElicitationHandler`] implementation that forwards elicitation
/// requests to the approval system (e.g. [`SocketApprovalGate`]).
///
/// Constructs an [`ApprovalRequest`] from the elicitation message and mode,
/// then delegates to the wrapped [`ApprovalGate`].
pub struct McpElicitationHandler {
    gate: Arc<dyn crate::security::approval::ApprovalGate>,
    server_name: String,
}

impl McpElicitationHandler {
    /// Create a new handler that forwards elicitation decisions to `gate`.
    pub fn new(
        gate: Arc<dyn crate::security::approval::ApprovalGate>,
        server_name: String,
    ) -> Self {
        Self { gate, server_name }
    }
}

#[async_trait]
impl ElicitationHandler for McpElicitationHandler {
    async fn handle_elicitation(&self, message: &str, mode: &str) -> Result<bool, String> {
        use crate::security::approval::{ApprovalDecision, ApprovalRequest};

        let req = ApprovalRequest {
            owner: fabric::ApprovalOwner::new(
                fabric::PrincipalId("mcp".into()),
                fabric::ThreadId("mcp".into()),
            ),
            connection_id: fabric::ConnectionId::new(),
            turn_id: fabric::TurnId::new(),
            call_id: format!("elicitation::{}", self.server_name),
            workspace: fabric::WorkspacePolicy::from_resolved_roots("/".into(), vec![])
                .unwrap_or_else(|_| {
                    fabric::WorkspacePolicy::from_resolved_roots("/tmp".into(), vec![]).unwrap()
                }),
            tool: format!("mcp::{}", self.server_name),
            action_summary: message.to_string(),
            risk_level: "medium".to_string(),
            detail: Some(format!("mode={}", mode)),
        };

        match self.gate.request(&req).await {
            ApprovalDecision::Approve | ApprovalDecision::ApproveForSession => Ok(true),
            ApprovalDecision::Deny => Ok(false),
        }
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

    /// Get all tool wrappers from all connected servers, subject to allowlist/denylist.
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

                // Apply allowlist/denylist filtering
                if !self.should_register_tool(&normalized_name) {
                    continue;
                }

                wrappers.push(McpToolWrapper {
                    normalized_name,
                    mcp_tool: tool.clone(),
                    client: client_arc.clone(),
                    trust_level: client.trust_level,
                    server_name: server_name.clone(),
                    overrides: self.config.permission_overrides.clone(),
                    supports_parallel: client.supports_parallel_tool_calls,
                });
            }
        }
        wrappers
    }

    /// Determine whether a tool should be registered based on allowlist/denylist.
    fn should_register_tool(&self, tool_name: &str) -> bool {
        // Denylist takes precedence — prefix match
        if self
            .config
            .tool_denylist
            .iter()
            .any(|d| tool_name == d || tool_name.starts_with(d))
        {
            return false;
        }
        // If allowlist is non-empty, tool must be in it — prefix match
        if !self.config.tool_allowlist.is_empty() {
            return self
                .config
                .tool_allowlist
                .iter()
                .any(|a| tool_name == a || tool_name.starts_with(a));
        }
        true
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

    /// Get all resource providers from all connected servers.
    pub fn get_all_resource_providers(&self) -> Vec<McpResourceProvider> {
        let mut providers = Vec::new();
        for (server_name, client_arc) in &self.clients {
            let Ok(client) = client_arc.try_lock() else {
                tracing::warn!(server = %server_name, "MCP client busy during resource discovery");
                continue;
            };
            for resource in &client.resources {
                // Build normalized name: mcp.{server_name}.resource.{resource_name}
                let normalized_name = format!("mcp.{}.resource.{}", server_name, resource.name);
                providers.push(McpResourceProvider {
                    uri: resource.uri.clone(),
                    normalized_name,
                    mcp_resource: resource.clone(),
                    client: client_arc.clone(),
                    server_name: server_name.clone(),
                    overrides: self.config.permission_overrides.clone(),
                });
            }
        }
        providers
    }

    /// List resources from a named server.
    pub async fn list_resources(
        &self,
        server_name: &str,
    ) -> Result<Vec<McpResource>> {
        let client_arc = self
            .clients
            .get(server_name)
            .with_context(|| format!("MCP server '{}' is not connected", server_name))?;
        let mut client = client_arc.lock().await;
        client.list_resources().await
    }

    /// Read a resource from a named server by URI.
    pub async fn read_resource(
        &self,
        server_name: &str,
        uri: &str,
    ) -> Result<super::client::ResourceContent> {
        let client_arc = self
            .clients
            .get(server_name)
            .with_context(|| format!("MCP server '{}' is not connected", server_name))?;
        let mut client = client_arc.lock().await;
        client.read_resource(uri).await
    }

    /// Set the elicitation handler on all connected clients.
    ///
    /// The handler is shared across all clients; if `None`, elicitation
    /// requests will be auto-denied.
    pub fn set_elicitation_handler(&self, handler: Option<Arc<dyn ElicitationHandler>>) {
        for client_arc in self.clients.values() {
            let Ok(mut client) = client_arc.try_lock() else {
                continue;
            };
            client.elicitation_handler = handler.clone();
        }
    }

    /// Handle an elicitation request from a specific server.
    ///
    /// Delegates to [`McpClient::handle_elicitation`].
    /// Returns an error if the server is not connected.
    pub async fn handle_elicitation(
        &self,
        server_name: &str,
        params: &Value,
    ) -> Result<Value> {
        let client_arc = self
            .clients
            .get(server_name)
            .with_context(|| format!("MCP server '{}' is not connected", server_name))?;
        let client = client_arc.lock().await;
        client.handle_elicitation(params).await
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::approval::{ApprovalDecision, ApprovalGate, ApprovalRequest};

    /// A test-only handler that always returns the configured value.
    struct FixedElicitationHandler {
        pub approved: bool,
    }

    #[async_trait]
    impl ElicitationHandler for FixedElicitationHandler {
        async fn handle_elicitation(&self, _message: &str, _mode: &str) -> Result<bool, String> {
            Ok(self.approved)
        }
    }

    /// A test-only handler that returns an error.
    struct FailingElicitationHandler;

    #[async_trait]
    impl ElicitationHandler for FailingElicitationHandler {
        async fn handle_elicitation(&self, _message: &str, _mode: &str) -> Result<bool, String> {
            Err("simulated handler error".to_string())
        }
    }

    /// A test-only approval gate that always returns the configured decision.
    struct FixedDecisionGate {
        decision: ApprovalDecision,
    }

    #[async_trait]
    impl ApprovalGate for FixedDecisionGate {
        async fn request(&self, _req: &ApprovalRequest) -> ApprovalDecision {
            self.decision
        }
    }

    #[test]
    fn test_fixed_elicitation_handler_approves() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let handler = FixedElicitationHandler { approved: true };
            let res = handler.handle_elicitation("Test message", "once").await;
            assert_eq!(res, Ok(true));
        });
    }

    #[test]
    fn test_fixed_elicitation_handler_denies() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let handler = FixedElicitationHandler { approved: false };
            let res = handler.handle_elicitation("Test message", "once").await;
            assert_eq!(res, Ok(false));
        });
    }

    #[test]
    fn test_failing_elicitation_handler_returns_error() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let handler = FailingElicitationHandler;
            let res = handler.handle_elicitation("Test message", "once").await;
            assert!(res.is_err());
        });
    }

    #[test]
    fn test_mcp_elicitation_handler_approves_via_gate() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let gate = Arc::new(FixedDecisionGate {
                decision: ApprovalDecision::Approve,
            });
            let handler = McpElicitationHandler::new(gate, "test-server".to_string());
            let res = handler.handle_elicitation("do a thing", "once").await;
            assert_eq!(res, Ok(true));
        });
    }

    #[test]
    fn test_mcp_elicitation_handler_denies_via_gate() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let gate = Arc::new(FixedDecisionGate {
                decision: ApprovalDecision::Deny,
            });
            let handler = McpElicitationHandler::new(gate, "test-server".to_string());
            let res = handler.handle_elicitation("do a thing", "once").await;
            assert_eq!(res, Ok(false));
        });
    }

    #[test]
    fn test_mcp_elicitation_handler_approve_for_session_treated_as_approve() {
        let rt = tokio::runtime::Runtime::new().unwrap();
        rt.block_on(async {
            let gate = Arc::new(FixedDecisionGate {
                decision: ApprovalDecision::ApproveForSession,
            });
            let handler = McpElicitationHandler::new(gate, "test-server".to_string());
            let res = handler.handle_elicitation("do a thing", "once").await;
            assert_eq!(res, Ok(true));
        });
    }
}
