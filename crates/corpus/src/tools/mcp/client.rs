use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};

use super::auth::BearerTokenAuth;
use super::config::{McpConfig, McpServerConfig, McpTransportConfig, McpTrustLevel};
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

async fn connect_mcp_server(server: &McpServerConfig, global_timeout_ms: u64) -> Result<McpClient> {
    let timeout_ms = server.request_timeout_ms.unwrap_or(global_timeout_ms);
    let auth = server
        .bearer_token_env
        .as_ref()
        .and_then(|env_var| match &server.transport {
            McpTransportConfig::StreamableHttp { url } => {
                Some(BearerTokenAuth::with_endpoint_scoping(
                    env_var.clone(),
                    mnemosyne::credential::EmbeddingCredentialGrant::new(
                        format!("mcp:{}", server.name),
                        url,
                        server.name.clone(),
                        u64::MAX,
                        0,
                        "environment-backed-mcp-token",
                    ),
                    Arc::new(aletheon_kernel::chronos::SystemClock::new()),
                ))
            }
            McpTransportConfig::Sse { url } => {
                let principal = format!("mcp:{}", server.name);
                Some(
                    BearerTokenAuth::with_endpoint_scoping(
                        env_var.clone(),
                        mnemosyne::credential::EmbeddingCredentialGrant::new(
                            principal.clone(),
                            url,
                            server.name.clone(),
                            u64::MAX,
                            0,
                            "environment-backed-mcp-token",
                        ),
                        Arc::new(aletheon_kernel::chronos::SystemClock::new()),
                    )
                    .allow_endpoint(
                        mnemosyne::credential::EmbeddingCredentialGrant::new(
                            principal,
                            &format!("{}/sse", url.trim_end_matches('/')),
                            server.name.clone(),
                            u64::MAX,
                            0,
                            "environment-backed-mcp-token",
                        ),
                    ),
                )
            }
            McpTransportConfig::Stdio { .. } => None,
        });
    match &server.transport {
        McpTransportConfig::Stdio { command, args } => {
            McpClient::connect_stdio(server.name.clone(), command, args, server.trust, timeout_ms)
                .await
        }
        McpTransportConfig::StreamableHttp { url } => {
            McpClient::connect_http(server.name.clone(), url, auth, server.trust, timeout_ms).await
        }
        McpTransportConfig::Sse { url } => {
            McpClient::connect_sse(server.name.clone(), url, auth, server.trust, timeout_ms).await
        }
    }
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

/// URI template advertised by an MCP server.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct McpResourceTemplate {
    pub uri_template: String,
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
    notification_rx: Option<mpsc::Receiver<McpNotification>>,
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
        request_timeout_ms: u64,
    ) -> Result<Self> {
        let (notification_tx, notification_rx) = mpsc::channel(64);
        let mut transport =
            McpTransport::stdio(command, args, request_timeout_ms, notification_tx.clone()).await?;

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

        // Discover tools
        let tools_result = transport
            .request(2, "tools/list", serde_json::json!({}))
            .await?;
        let tools = Self::parse_tools(&tools_result);

        tracing::info!(server = %server_name, count = tools.len(), "MCP tools discovered");

        // Discover resources
        let resources = Self::discover_resources(&mut transport, &server_name, 3)
            .await
            .unwrap_or_default();

        Ok(Self {
            server_name,
            transport,
            next_id: 4,
            trust_level,
            tools,
            resources,
            notification_tx,
            notification_rx: Some(notification_rx),
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
        request_timeout_ms: u64,
    ) -> Result<Self> {
        let (notification_tx, notification_rx) = mpsc::channel(64);
        let mut transport = McpTransport::streamable_http(url, auth, request_timeout_ms);

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
        let resources = Self::discover_resources(&mut transport, &server_name, 3)
            .await
            .unwrap_or_default();

        Ok(Self {
            server_name,
            transport,
            next_id: 4,
            trust_level,
            tools,
            resources,
            notification_tx,
            notification_rx: Some(notification_rx),
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
        request_timeout_ms: u64,
    ) -> Result<Self> {
        let (notification_tx, notification_rx) = mpsc::channel(64);
        let mut transport =
            McpTransport::sse(url, auth, request_timeout_ms, notification_tx.clone()).await?;

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
        let resources = Self::discover_resources(&mut transport, &server_name, 3)
            .await
            .unwrap_or_default();

        Ok(Self {
            server_name,
            transport,
            next_id: 4,
            trust_level,
            tools,
            resources,
            notification_tx,
            notification_rx: Some(notification_rx),
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

    pub async fn list_resource_templates(&mut self) -> Result<Vec<McpResourceTemplate>> {
        let id = self.next_id;
        self.next_id += 1;
        let result = self
            .transport
            .request(id, "resources/templates/list", serde_json::json!({}))
            .await?;
        Ok(result
            .get("resourceTemplates")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|value| {
                Some(McpResourceTemplate {
                    uri_template: value.get("uriTemplate")?.as_str()?.to_string(),
                    name: value.get("name")?.as_str()?.to_string(),
                    description: value
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                    mime_type: value
                        .get("mimeType")
                        .and_then(Value::as_str)
                        .map(str::to_string),
                })
            })
            .collect())
    }

    /// Read a specific resource by URI.
    pub async fn read_resource(&mut self, uri: &str) -> Result<super::client::ResourceContent> {
        let id = self.next_id;
        self.next_id += 1;
        let result = self
            .transport
            .request(id, "resources/read", serde_json::json!({"uri": uri}))
            .await?;

        // MCP resources/read returns the content array
        let text = if let Some(contents) = result.get("contents").and_then(|v| v.as_array()) {
            contents
                .iter()
                .filter_map(|c| {
                    let mime_type = c
                        .get("mimeType")
                        .and_then(|v| v.as_str())
                        .unwrap_or("text/plain");
                    if let Some(t) = c.get("text").and_then(|v| v.as_str()) {
                        Some(t.to_string())
                    } else {
                        c.get("blob")
                            .and_then(|v| v.as_str())
                            .map(|_| format!("[base64 blob: mimeType={}]", mime_type))
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
    pub async fn watch_tool_changes(
        client: Arc<Mutex<Self>>,
        mut rx: mpsc::Receiver<McpNotification>,
    ) {
        while let Some(notification) = rx.recv().await {
            if let McpNotification::ElicitationCreate { id, params } = notification {
                let mut client = client.lock().await;
                let result = client
                    .handle_elicitation(&params)
                    .await
                    .unwrap_or_else(|error| {
                        tracing::warn!(server = %client.server_name, %error, "MCP elicitation failed closed");
                        serde_json::json!({"action": "deny"})
                    });
                if let Err(error) = client.transport.send_response(id, result).await {
                    tracing::warn!(server = %client.server_name, %error, "failed to send MCP elicitation response");
                }
            } else if matches!(notification, McpNotification::ToolsListChanged) {
                let mut client = client.lock().await;
                let id = client.next_id;
                client.next_id += 1;
                match client
                    .transport
                    .request(id, "tools/list", serde_json::json!({}))
                    .await
                {
                    Ok(result) => {
                        let tools = Self::parse_tools(&result);
                        tracing::info!(
                            server = %client.server_name,
                            old_count = client.tools.len(),
                            new_count = tools.len(),
                            "MCP tools re-discovered after ToolsListChanged"
                        );
                        client.tools = tools;
                    }
                    Err(e) => {
                        tracing::warn!(
                            server = %client.server_name,
                            error = %e,
                            "Failed to re-discover tools after ToolsListChanged"
                        );
                    }
                }
            }
        }
    }

    /// Handle an MCP `elicitation/create` request from the server.
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
            Some(handler) => match handler.handle_elicitation(message, mode).await {
                Ok(allowed) => allowed,
                Err(e) => {
                    tracing::warn!(
                        server = %self.server_name,
                        error = %e,
                        "Elicitation handler returned an error; defaulting to deny"
                    );
                    false
                }
            },
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

pub struct McpElicitationHandler {
    gate: Arc<dyn crate::security::approval::ApprovalGate>,
    server_name: String,
}

impl McpElicitationHandler {
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

    fn install_client(
        &mut self,
        server_config: McpServerConfig,
        global_timeout_ms: u64,
        mut client: McpClient,
    ) {
        let notification_rx = client.notification_rx.take();
        let client = Arc::new(Mutex::new(client));
        self.clients
            .insert(server_config.name.clone(), client.clone());
        if let Some(rx) = notification_rx {
            tokio::spawn(McpClient::watch_tool_changes(client.clone(), rx));
        }
        if server_config.health_check_interval_sec > 0 {
            tokio::spawn(async move {
                let interval =
                    std::time::Duration::from_secs(server_config.health_check_interval_sec.max(1));
                loop {
                    tokio::time::sleep(interval).await;
                    let (healthy, elicitation_handler) = {
                        let mut current = client.lock().await;
                        let handler = current.elicitation_handler.clone();
                        let id = current.next_id;
                        current.next_id += 1;
                        let healthy = current
                            .transport
                            .request(id, "ping", serde_json::json!({}))
                            .await
                            .is_ok();
                        (healthy, handler)
                    };
                    if healthy {
                        continue;
                    }
                    match connect_mcp_server(&server_config, global_timeout_ms).await {
                        Ok(mut replacement) => {
                            replacement.elicitation_handler = elicitation_handler;
                            let notification_rx = replacement.notification_rx.take();
                            *client.lock().await = replacement;
                            if let Some(rx) = notification_rx {
                                tokio::spawn(McpClient::watch_tool_changes(client.clone(), rx));
                            }
                            tracing::info!(server = %server_config.name, "reconnected unhealthy MCP server");
                        }
                        Err(error) => {
                            tracing::warn!(server = %server_config.name, %error, "MCP reconnect attempt failed")
                        }
                    }
                }
            });
        }
    }

    /// Connect to all enabled servers in the config.
    pub async fn connect_all(&mut self) -> Result<()> {
        let global_timeout_ms = self.config.request_timeout_ms;

        for server_config in self.config.servers.clone() {
            if !server_config.enabled {
                continue;
            }

            let auth: Option<BearerTokenAuth> =
                server_config.bearer_token_env.as_ref().and_then(|env_var| {
                    match &server_config.transport {
                        McpTransportConfig::StreamableHttp { url } => {
                            Some(BearerTokenAuth::with_endpoint_scoping(
                                env_var.clone(),
                                mnemosyne::credential::EmbeddingCredentialGrant::new(
                                    format!("mcp:{}", server_config.name),
                                    url,
                                    server_config.name.clone(),
                                    u64::MAX,
                                    0,
                                    "environment-backed-mcp-token",
                                ),
                                Arc::new(aletheon_kernel::chronos::SystemClock::new()),
                            ))
                        }
                        McpTransportConfig::Sse { url } => {
                            let principal = format!("mcp:{}", server_config.name);
                            Some(
                                BearerTokenAuth::with_endpoint_scoping(
                                    env_var.clone(),
                                    mnemosyne::credential::EmbeddingCredentialGrant::new(
                                        principal.clone(),
                                        url,
                                        server_config.name.clone(),
                                        u64::MAX,
                                        0,
                                        "environment-backed-mcp-token",
                                    ),
                                    Arc::new(aletheon_kernel::chronos::SystemClock::new()),
                                )
                                .allow_endpoint(
                                    mnemosyne::credential::EmbeddingCredentialGrant::new(
                                        principal,
                                        &format!("{}/sse", url.trim_end_matches('/')),
                                        server_config.name.clone(),
                                        u64::MAX,
                                        0,
                                        "environment-backed-mcp-token",
                                    ),
                                ),
                            )
                        }
                        McpTransportConfig::Stdio { .. } => None,
                    }
                });

            let timeout_ms = server_config
                .request_timeout_ms
                .unwrap_or(global_timeout_ms);

            match &server_config.transport {
                McpTransportConfig::Stdio { command, args } => {
                    match McpClient::connect_stdio(
                        server_config.name.clone(),
                        command,
                        args,
                        server_config.trust,
                        timeout_ms,
                    )
                    .await
                    {
                        Ok(client) => {
                            self.install_client(server_config.clone(), global_timeout_ms, client);
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
                        timeout_ms,
                    )
                    .await
                    {
                        Ok(client) => {
                            self.install_client(server_config.clone(), global_timeout_ms, client);
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
                        timeout_ms,
                    )
                    .await
                    {
                        Ok(client) => {
                            self.install_client(server_config.clone(), global_timeout_ms, client);
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

    fn should_register_tool(&self, tool_name: &str) -> bool {
        if self
            .config
            .tool_denylist
            .iter()
            .any(|d| tool_name == d || tool_name.starts_with(d))
        {
            return false;
        }
        if !self.config.tool_allowlist.is_empty() {
            return self
                .config
                .tool_allowlist
                .iter()
                .any(|a| tool_name == a || tool_name.starts_with(a));
        }
        true
    }

    pub fn get_client(&self, server_name: &str) -> Option<&Arc<Mutex<McpClient>>> {
        self.clients.get(server_name)
    }

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

    pub fn connected_count(&self) -> usize {
        self.clients.len()
    }

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

    pub fn server_tools(&self, server_name: &str) -> Option<Vec<McpTool>> {
        let client = self.clients.get(server_name)?.try_lock().ok()?;
        Some(client.tools.clone())
    }

    pub fn get_all_resource_providers(&self) -> Vec<McpResourceProvider> {
        let mut providers = Vec::new();
        for (server_name, client_arc) in &self.clients {
            let Ok(client) = client_arc.try_lock() else {
                tracing::warn!(server = %server_name, "MCP client busy during resource discovery");
                continue;
            };
            for resource in &client.resources {
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

    pub async fn list_resources(&self, server_name: &str) -> Result<Vec<McpResource>> {
        let client_arc = self
            .clients
            .get(server_name)
            .with_context(|| format!("MCP server '{}' is not connected", server_name))?;
        let mut client = client_arc.lock().await;
        client.list_resources().await
    }

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

    pub async fn list_resource_templates(
        &self,
        server_name: &str,
    ) -> Result<Vec<McpResourceTemplate>> {
        let client_arc = self
            .clients
            .get(server_name)
            .with_context(|| format!("MCP server '{}' is not connected", server_name))?;
        client_arc.lock().await.list_resource_templates().await
    }

    pub fn set_elicitation_approval_gate(
        &self,
        gate: Arc<dyn crate::security::approval::ApprovalGate>,
    ) {
        for (server_name, client_arc) in &self.clients {
            let Ok(mut client) = client_arc.try_lock() else {
                continue;
            };
            client.elicitation_handler = Some(Arc::new(McpElicitationHandler::new(
                gate.clone(),
                server_name.clone(),
            )));
        }
    }

    pub fn set_elicitation_handler(&self, handler: Option<Arc<dyn ElicitationHandler>>) {
        for client_arc in self.clients.values() {
            let Ok(mut client) = client_arc.try_lock() else {
                continue;
            };
            client.elicitation_handler = handler.clone();
        }
    }

    pub async fn handle_elicitation(&self, server_name: &str, params: &Value) -> Result<Value> {
        let client_arc = self
            .clients
            .get(server_name)
            .with_context(|| format!("MCP server '{}' is not connected", server_name))?;
        let client = client_arc.lock().await;
        client.handle_elicitation(params).await
    }
}
