use std::collections::HashMap;
use std::sync::Arc;

use anyhow::{Context, Result};
use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::sync::{mpsc, Mutex};

use super::auth::{
    BearerTokenAuth, McpEndpointCredentialGrant, McpHttpAuth, McpOAuthProvider,
    OAuthClientAuthMethod, OAuthEndpoints, TokenStore,
};
use super::config::{
    McpConfig, McpOAuthClientAuthMethod, McpPermissionLevel, McpServerConfig, McpTransportConfig,
    McpTrustLevel,
};
use super::supervisor::{
    McpHealthSnapshot, McpShutdownReport, McpTaskExitPolicy, McpTaskSupervisor,
};
use super::transport::{McpNotification, McpTransport};
use super::wrapper::{McpResourceProvider, McpResourceReadTool, McpToolWrapper};

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
    if let McpTransportConfig::StreamableHttp { url } | McpTransportConfig::Sse { url } =
        &server.transport
    {
        endpoint_policy(server.trust)
            .approve(url)
            .await
            .context("MCP endpoint policy denied connection")?;
    }
    let bearer_auth = server
        .bearer_token_env
        .as_ref()
        .and_then(|env_var| match &server.transport {
            McpTransportConfig::StreamableHttp { url } => {
                Some(BearerTokenAuth::with_endpoint_scoping(
                    env_var.clone(),
                    McpEndpointCredentialGrant::new(
                        format!("mcp:{}", server.name),
                        url,
                        server.name.clone(),
                        u64::MAX,
                        0,
                    ),
                    Arc::new(kernel::chronos::SystemClock::new()),
                ))
            }
            McpTransportConfig::Sse { url } => {
                let principal = format!("mcp:{}", server.name);
                Some(
                    BearerTokenAuth::with_endpoint_scoping(
                        env_var.clone(),
                        McpEndpointCredentialGrant::new(
                            principal.clone(),
                            url,
                            server.name.clone(),
                            u64::MAX,
                            0,
                        ),
                        Arc::new(kernel::chronos::SystemClock::new()),
                    )
                    .allow_endpoint(McpEndpointCredentialGrant::new(
                        principal,
                        &format!("{}/sse", url.trim_end_matches('/')),
                        server.name.clone(),
                        u64::MAX,
                        0,
                    )),
                )
            }
            McpTransportConfig::Stdio { .. } => None,
        })
        .map(McpHttpAuth::from);
    // Static bearer credentials deliberately win over OAuth. OAuth is only
    // activated by `oauth.enabled = true` and only for HTTP transports.
    let auth = match bearer_auth {
        Some(auth) => Some(auth),
        None => match &server.transport {
            McpTransportConfig::StreamableHttp { url } | McpTransportConfig::Sse { url } => {
                configured_oauth(server, url).await?
            }
            McpTransportConfig::Stdio { .. } => None,
        },
    };
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

fn endpoint_policy(trust: McpTrustLevel) -> crate::tools::outbound::EndpointPolicy {
    match trust {
        McpTrustLevel::LocalTrusted => crate::tools::outbound::EndpointPolicy::local_loopback(),
        McpTrustLevel::RemoteTrusted => {
            crate::tools::outbound::EndpointPolicy::trusted_private_network()
        }
        McpTrustLevel::Untrusted => crate::tools::outbound::EndpointPolicy::public(),
    }
}

#[cfg(test)]
fn selected_http_auth_kind(server: &McpServerConfig) -> &'static str {
    let is_http = matches!(
        server.transport,
        McpTransportConfig::StreamableHttp { .. } | McpTransportConfig::Sse { .. }
    );
    if is_http && server.bearer_token_env.is_some() {
        "bearer"
    } else if is_http && server.oauth.as_ref().is_some_and(|oauth| oauth.enabled) {
        "oauth"
    } else {
        "none"
    }
}

async fn configured_oauth(
    server: &McpServerConfig,
    resource_url: &str,
) -> Result<Option<McpHttpAuth>> {
    let Some(config) = server.oauth.as_ref().filter(|config| config.enabled) else {
        return Ok(None);
    };
    validate_oauth_redirect_uri(&config.redirect_uri)?;
    let policy = endpoint_policy(server.trust);
    let discovered = match config.issuer.as_deref() {
        Some(issuer) => {
            Some(super::auth::discover_oauth_metadata_guarded(issuer, policy.clone()).await?)
        }
        None => None,
    };
    let auth_url = discovered
        .as_ref()
        .map(|value| value.authorization_endpoint.clone())
        .or_else(|| config.authorization_endpoint.clone())
        .context("OAuth requires issuer discovery or authorization_endpoint")?;
    let token_url = discovered
        .as_ref()
        .map(|value| value.token_endpoint.clone())
        .or_else(|| config.token_endpoint.clone())
        .context("OAuth requires issuer discovery or token_endpoint")?;
    policy
        .approve(&auth_url)
        .await
        .context("MCP OAuth authorization endpoint denied")?;
    policy
        .approve(&token_url)
        .await
        .context("MCP OAuth token endpoint denied")?;

    // Resolve credential material only after every credential-bearing endpoint
    // has passed identity and post-DNS address policy.
    let client_id = std::env::var(&config.client_id_env).with_context(|| {
        format!(
            "OAuth client id environment variable {} is unavailable",
            config.client_id_env
        )
    })?;
    let client_secret = config
        .client_secret_env
        .as_deref()
        .map(|name| {
            std::env::var(name).with_context(|| {
                format!("OAuth client secret environment variable {name} is unavailable")
            })
        })
        .transpose()?;
    let method = match config.token_endpoint_auth_method {
        McpOAuthClientAuthMethod::None => OAuthClientAuthMethod::None,
        McpOAuthClientAuthMethod::ClientSecretBasic => OAuthClientAuthMethod::ClientSecretBasic,
        McpOAuthClientAuthMethod::ClientSecretPost => OAuthClientAuthMethod::ClientSecretPost,
    };
    if let Some(metadata) = &discovered {
        let method_name = match method {
            OAuthClientAuthMethod::None => "none",
            OAuthClientAuthMethod::ClientSecretBasic => "client_secret_basic",
            OAuthClientAuthMethod::ClientSecretPost => "client_secret_post",
        };
        anyhow::ensure!(
            metadata.token_endpoint_auth_methods_supported.is_empty()
                || metadata
                    .token_endpoint_auth_methods_supported
                    .iter()
                    .any(|candidate| candidate == method_name),
            "OAuth discovery does not support configured token endpoint auth method"
        );
    }
    let mut provider = McpOAuthProvider::new(
        client_id,
        OAuthEndpoints {
            auth_url,
            token_url,
            redirect_uri: config.redirect_uri.clone(),
        },
        config.scopes.clone(),
        server.name.clone(),
        TokenStore::open_mcp_server(&server.name)?,
        Arc::new(kernel::chronos::SystemClock::new()),
    )
    .with_endpoint_scoping(resource_url);
    if let Some(secret) = client_secret {
        provider = provider.with_client_secret(secret);
    }
    provider = provider.with_client_auth_method(method)?;
    Ok(Some(McpHttpAuth::OAuth(Arc::new(parking_lot::Mutex::new(
        provider,
    )))))
}

fn validate_oauth_redirect_uri(redirect_uri: &str) -> Result<()> {
    let url = reqwest::Url::parse(redirect_uri).context("invalid OAuth redirect_uri")?;
    anyhow::ensure!(
        url.username().is_empty() && url.password().is_none() && url.fragment().is_none(),
        "OAuth redirect_uri must not contain credentials or a fragment"
    );
    let loopback = matches!(url.host_str(), Some("127.0.0.1" | "[::1]" | "localhost"));
    anyhow::ensure!(
        url.scheme() == "https" || (url.scheme() == "http" && loopback),
        "OAuth redirect_uri must use HTTPS or an HTTP loopback address"
    );
    Ok(())
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
    pub resource_templates: Vec<McpResourceTemplate>,
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
        let resource_templates = transport
            .request(4, "resources/templates/list", serde_json::json!({}))
            .await
            .map(|value| Self::parse_resource_templates(&value))
            .unwrap_or_default();

        Ok(Self {
            server_name,
            transport,
            next_id: 5,
            trust_level,
            tools,
            resources,
            resource_templates,
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
        auth: Option<McpHttpAuth>,
        trust_level: McpTrustLevel,
        request_timeout_ms: u64,
    ) -> Result<Self> {
        let (notification_tx, notification_rx) = mpsc::channel(64);
        endpoint_policy(trust_level)
            .approve(url)
            .await
            .context("MCP endpoint policy denied connection")?;
        let mut transport = McpTransport::streamable_http_guarded(
            url,
            auth,
            request_timeout_ms,
            endpoint_policy(trust_level),
        )?;

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
        let resource_templates = transport
            .request(4, "resources/templates/list", serde_json::json!({}))
            .await
            .map(|value| Self::parse_resource_templates(&value))
            .unwrap_or_default();

        Ok(Self {
            server_name,
            transport,
            next_id: 5,
            trust_level,
            tools,
            resources,
            resource_templates,
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
        auth: Option<McpHttpAuth>,
        trust_level: McpTrustLevel,
        request_timeout_ms: u64,
    ) -> Result<Self> {
        let (notification_tx, notification_rx) = mpsc::channel(64);
        endpoint_policy(trust_level)
            .approve(url)
            .await
            .context("MCP endpoint policy denied connection")?;
        let mut transport = McpTransport::sse_guarded(
            url,
            auth,
            request_timeout_ms,
            notification_tx.clone(),
            endpoint_policy(trust_level),
        )
        .await?;

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
        let resource_templates = transport
            .request(4, "resources/templates/list", serde_json::json!({}))
            .await
            .map(|value| Self::parse_resource_templates(&value))
            .unwrap_or_default();

        Ok(Self {
            server_name,
            transport,
            next_id: 5,
            trust_level,
            tools,
            resources,
            resource_templates,
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

    fn parse_resource_templates(result: &Value) -> Vec<McpResourceTemplate> {
        result
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
            .collect()
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
        Ok(Self::parse_resource_templates(&result))
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
        registry_change_tx: Option<mpsc::Sender<String>>,
        cancel: tokio_util::sync::CancellationToken,
    ) {
        loop {
            let notification = tokio::select! {
                _ = cancel.cancelled() => break,
                notification = rx.recv() => match notification {
                    Some(notification) => notification,
                    None => break,
                },
            };
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
                        let server_name = client.server_name.clone();
                        drop(client);
                        if let Some(tx) = &registry_change_tx {
                            let _ = tx.send(server_name).await;
                        }
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
            call_id: format!(
                "elicitation::{}::{}",
                self.server_name,
                uuid::Uuid::new_v4()
            ),
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

fn spawn_connected_health_supervisor(
    client: Arc<Mutex<McpClient>>,
    server_config: McpServerConfig,
    global_timeout_ms: u64,
    registry_change_tx: Option<mpsc::Sender<String>>,
    supervisor: Arc<McpTaskSupervisor>,
) {
    if server_config.health_check_interval_sec == 0 {
        return;
    }
    let task_name = format!("mcp:{}:health", server_config.name);
    let server_name = server_config.name.clone();
    let cancel = supervisor.cancellation_token();
    let task_supervisor = supervisor.clone();
    supervisor.spawn(task_name, server_name.clone(), async move {
        let interval =
            std::time::Duration::from_secs(server_config.health_check_interval_sec.max(1));
        loop {
            tokio::select! {
                _ = cancel.cancelled() => break,
                _ = tokio::time::sleep(interval) => {}
            }
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
                task_supervisor.mark_ping_healthy(&server_config.name);
                continue;
            }
            task_supervisor.mark_reconnecting(&server_config.name, "ping_failed");
            match connect_mcp_server(&server_config, global_timeout_ms).await {
                Ok(mut replacement) => {
                    replacement.elicitation_handler = elicitation_handler;
                    let notification_rx = replacement.notification_rx.take();
                    *client.lock().await = replacement;
                    if let Some(rx) = notification_rx {
                        task_supervisor.spawn_with_policy(
                            format!("mcp:{}:notifications", server_config.name),
                            server_config.name.clone(),
                            McpTaskExitPolicy::Complete,
                            McpClient::watch_tool_changes(
                                client.clone(),
                                rx,
                                registry_change_tx.clone(),
                                task_supervisor.cancellation_token(),
                            ),
                        );
                    }
                    if let Some(tx) = &registry_change_tx {
                        let _ = tx.send(server_config.name.clone()).await;
                    }
                    tracing::info!(server = %server_config.name, "reconnected unhealthy MCP server");
                    task_supervisor.mark_connected(&server_config.name);
                }
                Err(error) => {
                    tracing::warn!(server = %server_config.name, %error, "MCP reconnect attempt failed")
                }
            }
        }
    });
}

/// Manages connections to multiple MCP servers.
pub struct McpConnectionManager {
    clients: Arc<std::sync::RwLock<HashMap<String, Arc<Mutex<McpClient>>>>>,
    config: McpConfig,
    registry_change_tx: Option<mpsc::Sender<String>>,
    elicitation_gate:
        Arc<std::sync::RwLock<Option<Arc<dyn crate::security::approval::ApprovalGate>>>>,
    supervisor: Arc<McpTaskSupervisor>,
}

impl McpConnectionManager {
    pub fn new(config: McpConfig) -> Self {
        let supervisor = McpTaskSupervisor::new();
        for server in config.servers.iter().filter(|server| server.enabled) {
            supervisor.register_server(&server.name);
        }
        Self {
            clients: Arc::new(std::sync::RwLock::new(HashMap::new())),
            config,
            registry_change_tx: None,
            elicitation_gate: Arc::new(std::sync::RwLock::new(None)),
            supervisor,
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
            .write()
            .expect("MCP clients lock poisoned")
            .insert(server_config.name.clone(), client.clone());
        if let Some(rx) = notification_rx {
            self.supervisor.spawn_with_policy(
                format!("mcp:{}:notifications", server_config.name),
                server_config.name.clone(),
                McpTaskExitPolicy::Complete,
                McpClient::watch_tool_changes(
                    client.clone(),
                    rx,
                    self.registry_change_tx.clone(),
                    self.supervisor.cancellation_token(),
                ),
            );
        }
        self.supervisor.mark_connected(&server_config.name);
        spawn_connected_health_supervisor(
            client,
            server_config,
            global_timeout_ms,
            self.registry_change_tx.clone(),
            self.supervisor.clone(),
        );
    }

    /// Connect to all enabled servers in the config.
    pub async fn connect_all(&mut self) -> Result<()> {
        let global_timeout_ms = self.config.request_timeout_ms;

        for server_config in self.config.servers.clone() {
            if !server_config.enabled {
                continue;
            }

            match connect_mcp_server(&server_config, global_timeout_ms).await {
                Ok(client) => {
                    self.install_client(server_config.clone(), global_timeout_ms, client);
                }
                Err(error) => {
                    tracing::warn!(
                        server = %server_config.name,
                        error = %error,
                        "Failed to connect MCP server"
                    );
                    self.spawn_initial_reconnect(server_config.clone(), global_timeout_ms);
                }
            }
        }
        Ok(())
    }

    fn spawn_initial_reconnect(&self, server_config: McpServerConfig, global_timeout_ms: u64) {
        if server_config.health_check_interval_sec == 0 {
            self.supervisor.mark_degraded(
                &server_config.name,
                "initial_connect_failed_reconnect_disabled",
            );
            return;
        }
        let clients = self.clients.clone();
        let registry_change_tx = self.registry_change_tx.clone();
        let elicitation_gate = self.elicitation_gate.clone();
        self.supervisor
            .mark_reconnecting(&server_config.name, "initial_connect_failed");
        let supervisor = self.supervisor.clone();
        let task_supervisor = supervisor.clone();
        let task_name = format!("mcp:{}:initial_reconnect", server_config.name);
        let server_name = server_config.name.clone();
        let cancel = supervisor.cancellation_token();
        supervisor.spawn_with_policy(
            task_name,
            server_name,
            McpTaskExitPolicy::Complete,
            async move {
            let interval =
                std::time::Duration::from_secs(server_config.health_check_interval_sec.max(1));
            loop {
                tokio::select! {
                    _ = cancel.cancelled() => break,
                    _ = tokio::time::sleep(interval) => {}
                }
                match connect_mcp_server(&server_config, global_timeout_ms).await {
                    Ok(mut client) => {
                        if let Some(gate) = elicitation_gate
                            .read()
                            .expect("MCP elicitation gate lock poisoned")
                            .clone()
                        {
                            client.elicitation_handler = Some(Arc::new(
                                McpElicitationHandler::new(gate, server_config.name.clone()),
                            ));
                        }
                        let rx = client.notification_rx.take();
                        let client = Arc::new(Mutex::new(client));
                        clients
                            .write()
                            .expect("MCP clients lock poisoned")
                            .insert(server_config.name.clone(), client.clone());
                        if let Some(rx) = rx {
                            task_supervisor.spawn_with_policy(
                                format!("mcp:{}:notifications", server_config.name),
                                server_config.name.clone(),
                                McpTaskExitPolicy::Complete,
                                McpClient::watch_tool_changes(
                                    client.clone(),
                                    rx,
                                    registry_change_tx.clone(),
                                    task_supervisor.cancellation_token(),
                                ),
                            );
                        }
                        if let Some(tx) = &registry_change_tx {
                            let _ = tx.send(server_config.name.clone()).await;
                        }
                        spawn_connected_health_supervisor(
                            client,
                            server_config.clone(),
                            global_timeout_ms,
                            registry_change_tx.clone(),
                            task_supervisor.clone(),
                        );
                        task_supervisor.mark_connected(&server_config.name);
                        tracing::info!(server = %server_config.name, "connected MCP server after initial failure");
                        break;
                    }
                    Err(error) => {
                        tracing::warn!(server = %server_config.name, %error, "initial MCP reconnect attempt failed")
                    }
                }
            }
            },
        );
    }

    pub fn set_registry_change_sender(&mut self, sender: mpsc::Sender<String>) {
        self.registry_change_tx = Some(sender);
    }

    /// Get all tool wrappers from all connected servers, subject to allowlist/denylist.
    pub fn get_all_tools(&self) -> Vec<McpToolWrapper> {
        let mut wrappers = Vec::new();
        let clients = self.clients.read().expect("MCP clients lock poisoned");
        for (server_name, client_arc) in clients.iter() {
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

                if !self.should_register_tool(server_name, &tool.name, &normalized_name) {
                    continue;
                }

                let overrides = self.permission_overrides(server_name);

                wrappers.push(McpToolWrapper {
                    normalized_name,
                    mcp_tool: tool.clone(),
                    client: client_arc.clone(),
                    trust_level: client.trust_level,
                    server_name: server_name.clone(),
                    overrides,
                    supports_parallel: client.supports_parallel_tool_calls,
                });
            }
        }
        wrappers
    }

    fn should_register_tool(
        &self,
        server_name: &str,
        advertised_name: &str,
        registered_name: &str,
    ) -> bool {
        let server = self
            .config
            .servers
            .iter()
            .find(|server| server.name == server_name);
        let matches = |entry: &str| advertised_name == entry || registered_name == entry;
        if server.is_some_and(|server| server.denylist.iter().any(|entry| matches(entry))) {
            return false;
        }
        if let Some(server) = server {
            if !server.allowlist.is_empty() && !server.allowlist.iter().any(|entry| matches(entry))
            {
                return false;
            }
        }
        if self.config.tool_denylist.iter().any(|entry| matches(entry)) {
            return false;
        }
        if !self.config.tool_allowlist.is_empty() {
            return self
                .config
                .tool_allowlist
                .iter()
                .any(|entry| matches(entry));
        }
        true
    }

    fn permission_overrides(
        &self,
        server_name: &str,
    ) -> std::collections::HashMap<String, crate::tools::PermissionLevel> {
        let mut overrides = self.config.permission_overrides.clone();
        if let Some(server) = self
            .config
            .servers
            .iter()
            .find(|server| server.name == server_name)
        {
            overrides.extend(server.permission_overrides.iter().map(|(name, level)| {
                let level = match level {
                    McpPermissionLevel::L0 => crate::tools::PermissionLevel::L0,
                    McpPermissionLevel::L1 => crate::tools::PermissionLevel::L1,
                    McpPermissionLevel::L2 => crate::tools::PermissionLevel::L2,
                    McpPermissionLevel::L3 => crate::tools::PermissionLevel::L3,
                };
                (name.clone(), level)
            }));
        }
        overrides
    }

    pub fn get_client(&self, server_name: &str) -> Option<Arc<Mutex<McpClient>>> {
        self.clients.read().ok()?.get(server_name).cloned()
    }

    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        args: Value,
    ) -> Result<Value> {
        let client_arc = self
            .clients
            .read()
            .expect("MCP clients lock poisoned")
            .get(server_name)
            .cloned()
            .with_context(|| format!("MCP server '{}' is not connected", server_name))?;
        let mut client = client_arc.lock().await;
        client.call_tool(tool_name, args).await
    }

    pub fn connected_count(&self) -> usize {
        self.clients
            .read()
            .expect("MCP clients lock poisoned")
            .len()
    }

    pub fn health_snapshot(&self) -> McpHealthSnapshot {
        self.supervisor.snapshot()
    }

    pub async fn shutdown(&self, timeout: std::time::Duration) -> McpShutdownReport {
        self.supervisor.shutdown(timeout).await
    }

    pub fn server_has_tools(&self, server_name: &str, required: &[&str]) -> bool {
        self.clients
            .read()
            .expect("MCP clients lock poisoned")
            .get(server_name)
            .is_some_and(|client| {
                let Ok(client) = client.try_lock() else {
                    return false;
                };
                required
                    .iter()
                    .all(|name| client.tools.iter().any(|tool| tool.name == *name))
            })
    }

    pub fn server_tools(&self, server_name: &str) -> Option<Vec<McpTool>> {
        let client_arc = self.clients.read().ok()?.get(server_name)?.clone();
        let client = client_arc.try_lock().ok()?;
        Some(client.tools.clone())
    }

    pub fn get_all_resource_providers(&self) -> Vec<McpResourceProvider> {
        let mut providers = Vec::new();
        let clients = self.clients.read().expect("MCP clients lock poisoned");
        for (server_name, client_arc) in clients.iter() {
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
                    overrides: self.permission_overrides(server_name),
                });
            }
        }
        providers
    }

    pub fn get_resource_read_tools(&self) -> Vec<McpResourceReadTool> {
        let clients = self.clients.read().expect("MCP clients lock poisoned");
        clients
            .iter()
            .filter_map(|(server_name, client)| {
                let normalized_name = if self.config.tool_name_prefix {
                    format!("{}__mcp_resource_read", server_name)
                } else {
                    "mcp_resource_read".to_string()
                };
                self.should_register_tool(server_name, "mcp_resource_read", &normalized_name)
                    .then(|| McpResourceReadTool {
                        normalized_name,
                        client: client.clone(),
                        server_name: server_name.clone(),
                        overrides: self.permission_overrides(server_name),
                    })
            })
            .collect()
    }

    pub async fn list_resources(&self, server_name: &str) -> Result<Vec<McpResource>> {
        let client_arc = self
            .clients
            .read()
            .expect("MCP clients lock poisoned")
            .get(server_name)
            .cloned()
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
            .read()
            .expect("MCP clients lock poisoned")
            .get(server_name)
            .cloned()
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
            .read()
            .expect("MCP clients lock poisoned")
            .get(server_name)
            .cloned()
            .with_context(|| format!("MCP server '{}' is not connected", server_name))?;
        let mut client = client_arc.lock().await;
        client.list_resource_templates().await
    }

    pub fn set_elicitation_approval_gate(
        &self,
        gate: Arc<dyn crate::security::approval::ApprovalGate>,
    ) {
        *self
            .elicitation_gate
            .write()
            .expect("MCP elicitation gate lock poisoned") = Some(gate.clone());
        let clients = self.clients.read().expect("MCP clients lock poisoned");
        for (server_name, client_arc) in clients.iter() {
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
        let clients = self.clients.read().expect("MCP clients lock poisoned");
        for client_arc in clients.values() {
            let Ok(mut client) = client_arc.try_lock() else {
                continue;
            };
            client.elicitation_handler = handler.clone();
        }
    }

    pub async fn handle_elicitation(&self, server_name: &str, params: &Value) -> Result<Value> {
        let client_arc = self
            .clients
            .read()
            .expect("MCP clients lock poisoned")
            .get(server_name)
            .cloned()
            .with_context(|| format!("MCP server '{}' is not connected", server_name))?;
        let client = client_arc.lock().await;
        client.handle_elicitation(params).await
    }
}

#[cfg(test)]
mod oauth_selection_tests {
    use super::super::config::McpOAuthConfig;
    use super::*;

    fn http_server() -> McpServerConfig {
        McpServerConfig {
            name: "acceptance".into(),
            transport: McpTransportConfig::StreamableHttp {
                url: "https://mcp.example.test/rpc".into(),
            },
            oauth: Some(McpOAuthConfig {
                enabled: true,
                client_id_env: "MCP_CLIENT_ID".into(),
                client_secret_env: None,
                redirect_uri: "http://127.0.0.1:8765/callback".into(),
                scopes: vec!["tools:read".into()],
                token_endpoint_auth_method: McpOAuthClientAuthMethod::None,
                issuer: Some("https://issuer.example.test".into()),
                authorization_endpoint: None,
                token_endpoint: None,
            }),
            ..McpServerConfig::default()
        }
    }

    #[test]
    fn static_bearer_has_precedence_over_enabled_oauth() {
        let mut server = http_server();
        server.bearer_token_env = Some("MCP_STATIC_BEARER".into());
        assert_eq!(selected_http_auth_kind(&server), "bearer");
    }

    #[test]
    fn oauth_requires_explicit_enabled_opt_in_and_http_transport() {
        let mut server = http_server();
        assert_eq!(selected_http_auth_kind(&server), "oauth");

        server.oauth.as_mut().unwrap().enabled = false;
        assert_eq!(selected_http_auth_kind(&server), "none");

        server.oauth.as_mut().unwrap().enabled = true;
        server.transport = McpTransportConfig::Stdio {
            command: "mcp-server".into(),
            args: vec![],
        };
        assert_eq!(selected_http_auth_kind(&server), "none");
    }

    #[test]
    fn oauth_redirect_rejects_insecure_remote_and_ambiguous_uris() {
        assert!(validate_oauth_redirect_uri("http://example.test/callback").is_err());
        assert!(validate_oauth_redirect_uri("https://user@example.test/callback").is_err());
        assert!(validate_oauth_redirect_uri("https://example.test/callback#token").is_err());
        assert!(validate_oauth_redirect_uri("http://127.0.0.1:8765/callback").is_ok());
        assert!(validate_oauth_redirect_uri("https://app.example.test/callback").is_ok());
    }

    #[tokio::test]
    async fn oauth_credentials_are_not_resolved_before_endpoint_approval() {
        let mut server = http_server();
        server.trust = McpTrustLevel::RemoteTrusted;
        server.oauth = Some(McpOAuthConfig {
            enabled: true,
            client_id_env: "ALETHEON_TEST_MISSING_CLIENT_ID".into(),
            client_secret_env: Some("ALETHEON_TEST_MISSING_CLIENT_SECRET".into()),
            redirect_uri: "http://127.0.0.1:8765/callback".into(),
            scopes: vec!["tools:read".into()],
            token_endpoint_auth_method: McpOAuthClientAuthMethod::ClientSecretPost,
            issuer: None,
            authorization_endpoint: Some("http://127.0.0.1:9/authorize".into()),
            token_endpoint: Some("http://127.0.0.1:9/token".into()),
        });

        let error = match configured_oauth(&server, "https://mcp.example.test/rpc").await {
            Ok(_) => panic!("prohibited OAuth endpoint was accepted"),
            Err(error) => error.to_string(),
        };
        assert!(
            error.contains("endpoint denied"),
            "unexpected error: {error}"
        );
        assert!(!error.contains("environment variable"));
    }
}
