//! High-level MCP facade for the daemon: connect configured servers and expose
//! their tools and resources as `Box<dyn Tool>` ready to register into the ToolRegistry.

use anyhow::Result;
use serde_json::Value;
use std::sync::Arc;

use super::client::{ElicitationHandler, McpConnectionManager, McpResource, ResourceContent};
use super::client::McpTool;
use super::config::McpConfig;
use crate::tools::Tool;

/// Thin facade that owns an [`McpConnectionManager`] and exposes a
/// register-friendly interface.
pub struct McpManager {
    inner: McpConnectionManager,
}

impl McpManager {
    pub fn new(config: McpConfig) -> Self {
        Self {
            inner: McpConnectionManager::new(config),
        }
    }

    /// Connect to every enabled server in the config.
    pub async fn connect_all(&mut self) -> Result<()> {
        self.inner.connect_all().await
    }

    /// Call a tool on a named server and return the raw result.
    ///
    /// Delegates to [`McpConnectionManager::call_tool`].
    pub async fn call_tool(
        &self,
        server_name: &str,
        tool_name: &str,
        args: Value,
    ) -> Result<Value> {
        self.inner.call_tool(server_name, tool_name, args).await
    }

    /// Return discovered tools as boxed [`Tool`] trait objects, ready to be
    /// inserted into a `ToolRegistry`.
    pub fn tool_wrappers(&self) -> Vec<Box<dyn Tool>> {
        self.inner
            .get_all_tools()
            .into_iter()
            .map(|w| w.boxed_clone())
            .collect()
    }

    /// Return discovered resources as boxed [`Tool`] trait objects.
    pub fn resource_wrappers(&self) -> Vec<Box<dyn Tool>> {
        self.inner
            .get_all_resource_providers()
            .into_iter()
            .map(|p| p.boxed_clone())
            .collect()
    }

    /// List resources from a named server.
    pub async fn list_resources(&self, server_name: &str) -> Result<Vec<McpResource>> {
        self.inner.list_resources(server_name).await
    }

    /// Read a resource from a named server by URI.
    pub async fn read_resource(&self, server_name: &str, uri: &str) -> Result<ResourceContent> {
        self.inner.read_resource(server_name, uri).await
    }

    /// Number of servers that were successfully connected.
    pub fn connected_count(&self) -> usize {
        self.inner.connected_count()
    }

    pub fn server_has_tools(&self, server_name: &str, required: &[&str]) -> bool {
        self.inner.server_has_tools(server_name, required)
    }

    /// Return the advertised tool descriptors for contract validation.
    pub fn server_tools(&self, server_name: &str) -> Option<Vec<McpTool>> {
        self.inner.server_tools(server_name)
    }

    /// Set the elicitation handler on all connected clients.
    ///
    /// If `None`, elicitation requests will be auto-denied.
    pub fn set_elicitation_handler(&self, handler: Option<Arc<dyn ElicitationHandler>>) {
        self.inner.set_elicitation_handler(handler);
    }

    /// Handle an elicitation request from a specific server.
    ///
    /// Delegates to the client's elicitation handler; auto-denies if no
    /// handler is configured.
    pub async fn handle_elicitation(
        &self,
        server_name: &str,
        params: &Value,
    ) -> Result<Value> {
        self.inner.handle_elicitation(server_name, params).await
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::{McpServerConfig, McpTransportConfig, McpTrustLevel};
    use super::*;
    use fabric::tool::ConcurrencyClass;
    use http_body_util::{BodyExt, Full};
    use hyper::body::{Bytes, Incoming};
    use hyper::server::conn::http1;
    use hyper::service::service_fn;
    use hyper::{Method, Request, Response, StatusCode};
    use hyper_util::rt::TokioIo;
    use serde_json::json;
    use std::convert::Infallible;
    use std::net::SocketAddr;
    use std::sync::{Arc, Mutex};
    use tokio::net::TcpListener;

    /// Shared state for the mock MCP server.
    struct MockMcpState {
        expected_token: Option<String>,
        tools_list_response: serde_json::Value,
        search_response: serde_json::Value,
        return_401: bool,
        /// Captured authorization headers.
        auth_headers: Vec<Option<String>>,
        /// Custom initialize response (defaults to standard one if None).
        initialize_response: Option<serde_json::Value>,
    }

    impl MockMcpState {
        fn with_token(token: &str) -> Self {
            Self {
                expected_token: Some(format!("Bearer {}", token)),
                tools_list_response: json!({
                    "tools": [
                        {
                            "name": "search",
                            "description": "Search memory",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "query": {"type": "string"},
                                    "source": {"type": "string"}
                                }
                            }
                        },
                        {
                            "name": "get_page",
                            "description": "Get a page by slug",
                            "inputSchema": {
                                "type": "object",
                                "properties": {
                                    "source": {"type": "string"},
                                    "slug": {"type": "string"}
                                }
                            }
                        }
                    ]
                }),
                search_response: json!({
                    "content": [{"type": "text", "text": "search result"}]
                }),
                return_401: false,
                auth_headers: Vec::new(),
                initialize_response: None,
            }
        }

        fn with_401() -> Self {
            let mut state = Self::with_token("test-token");
            state.return_401 = true;
            state
        }

        fn with_parallel_tool_calls() -> Self {
            let mut state = Self::with_token("parallel-token");
            state.initialize_response = Some(json!({
                "protocolVersion": "2024-11-05",
                "capabilities": {
                    "tools": {
                        "supports_parallel_tool_calls": true
                    }
                },
                "serverInfo": {"name": "parallel-mock", "version": "1.0.0"}
            }));
            state
        }

        async fn handle_request(
            state: Arc<Mutex<Self>>,
            req: Request<Incoming>,
        ) -> Result<Response<Full<Bytes>>, Infallible> {
            // Record auth header synchronously
            let auth = req
                .headers()
                .get("Authorization")
                .and_then(|v| v.to_str().ok())
                .map(|s| s.to_string());
            {
                let mut s = state.lock().unwrap();
                s.auth_headers.push(auth.clone());
            }

            // Check return_401 and expected token synchronously
            let (return_401, expected_token, tools_list, search_resp, initialize_resp) = {
                let s = state.lock().unwrap();
                (
                    s.return_401,
                    s.expected_token.clone(),
                    s.tools_list_response.clone(),
                    s.search_response.clone(),
                    s.initialize_response.clone(),
                )
            };

            if return_401 {
                return Ok(Response::builder()
                    .status(StatusCode::UNAUTHORIZED)
                    .body(Full::new(Bytes::from(
                        r#"{"jsonrpc":"2.0","error":{"code":-32001,"message":"Unauthorized"}}"#,
                    )))
                    .unwrap());
            }

            // Check auth
            if let Some(ref expected) = expected_token {
                if auth.as_deref() != Some(expected.as_str()) {
                    return Ok(Response::builder()
                        .status(StatusCode::UNAUTHORIZED)
                        .body(Full::new(Bytes::from(
                            r#"{"jsonrpc":"2.0","error":{"code":-32001,"message":"Unauthorized"}}"#,
                        )))
                        .unwrap());
                }
            }

            // Only handle POST
            if req.method() != Method::POST {
                return Ok(Response::builder()
                    .status(StatusCode::METHOD_NOT_ALLOWED)
                    .body(Full::new(Bytes::from("")))
                    .unwrap());
            }

            // Parse body
            let collected = req
                .collect()
                .await
                .map(|b| b.to_bytes())
                .unwrap_or_default();
            let req_json: serde_json::Value =
                serde_json::from_slice(&collected).unwrap_or(Value::Null);

            let method = req_json
                .get("method")
                .and_then(|v| v.as_str())
                .unwrap_or("");
            let id = req_json.get("id").cloned().unwrap_or(Value::Null);

            let result = match method {
                "initialize" => initialize_resp.unwrap_or_else(|| json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "test-mock", "version": "1.0.0"}
                })),
                "tools/list" => tools_list,
                "tools/call" => search_resp,
                "resources/list" => json!({
                    "resources": [
                        {
                            "uri": "file:///docs/readme.md",
                            "name": "docs_readme",
                            "description": "Project readme",
                            "mimeType": "text/markdown"
                        },
                        {
                            "uri": "file:///config/app.toml",
                            "name": "app_config",
                            "description": "Application configuration"
                        }
                    ]
                }),
                "resources/read" => {
                    // Extract URI from params
                    let uri = req_json
                        .get("params")
                        .and_then(|p| p.get("uri"))
                        .and_then(|v| v.as_str())
                        .unwrap_or("");
                    json!({
                        "contents": [{
                            "uri": uri,
                            "mimeType": "text/plain",
                            "text": format!("Content of {}", uri)
                        }]
                    })
                },
                _ => json!({}),
            };

            let response = json!({
                "jsonrpc": "2.0",
                "id": id,
                "result": result,
            });

            Ok(Response::builder()
                .status(StatusCode::OK)
                .header("Content-Type", "application/json")
                .body(Full::new(Bytes::from(response.to_string())))
                .unwrap())
        }
    }

    async fn spawn_mock_server(state: Arc<Mutex<MockMcpState>>) -> SocketAddr {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let addr = listener.local_addr().unwrap();

        tokio::spawn(async move {
            loop {
                let (stream, _) = match listener.accept().await {
                    Ok(conn) => conn,
                    Err(_) => break,
                };
                let state = state.clone();
                tokio::spawn(async move {
                    let io = TokioIo::new(stream);
                    let svc =
                        service_fn(move |req| MockMcpState::handle_request(state.clone(), req));
                    let _ = http1::Builder::new().serve_connection(io, svc).await;
                });
            }
        });

        addr
    }

    // ---------------------------------------------------------------------------
    // Tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn empty_config_connect_all_ok() {
        let config = McpConfig::default();
        let mut mgr = McpManager::new(config);
        let result = mgr.connect_all().await;
        assert!(result.is_ok());
    }

    #[tokio::test]
    async fn empty_config_tool_wrappers_empty() {
        let config = McpConfig::default();
        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();
        assert!(mgr.tool_wrappers().is_empty());
    }

    #[tokio::test]
    async fn empty_config_connected_count_zero() {
        let config = McpConfig::default();
        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();
        assert_eq!(mgr.connected_count(), 0);
    }

    #[tokio::test]
    async fn call_tool_unknown_server_returns_error() {
        let config = McpConfig::default();
        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();

        let result = mgr
            .call_tool("nonexistent", "search", json!({"query": "test"}))
            .await;
        assert!(result.is_err());
        let err_msg = format!("{}", result.unwrap_err());
        assert!(err_msg.contains("not connected"));
    }

    #[tokio::test]
    async fn http_connect_with_bearer_token_sends_auth_header() {
        let state = Arc::new(Mutex::new(MockMcpState::with_token("test-token-123")));
        let addr = spawn_mock_server(state.clone()).await;

        std::env::set_var("TEST_HTTP_TOKEN", "test-token-123");

        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "gbrain".into(),
                transport: McpTransportConfig::StreamableHttp {
                    url: format!("http://{}/mcp", addr),
                },
                trust: McpTrustLevel::RemoteTrusted,
                enabled: true,
                bearer_token_env: Some("TEST_HTTP_TOKEN".into()),
                request_timeout_ms: None,
            }],
            ..McpConfig::default()
        };

        let mut mgr = McpManager::new(config);
        let result = mgr.connect_all().await;
        assert!(result.is_ok(), "connect_all failed: {:?}", result);
        assert_eq!(mgr.connected_count(), 1);

        // Verify auth header was sent
        let s = state.lock().unwrap();
        let auth_sent = s
            .auth_headers
            .iter()
            .any(|h| h.as_deref() == Some("Bearer test-token-123"));
        assert!(
            auth_sent,
            "Expected Bearer token in auth headers, got: {:?}",
            s.auth_headers
        );

        std::env::remove_var("TEST_HTTP_TOKEN");
    }

    #[tokio::test]
    async fn http_connect_then_call_tool() {
        std::env::set_var("TEST_CALL_TOKEN", "call-token-456");

        let state = Arc::new(Mutex::new(MockMcpState::with_token("call-token-456")));
        let addr = spawn_mock_server(state.clone()).await;

        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "gbrain".into(),
                transport: McpTransportConfig::StreamableHttp {
                    url: format!("http://{}/mcp", addr),
                },
                trust: McpTrustLevel::RemoteTrusted,
                enabled: true,
                bearer_token_env: Some("TEST_CALL_TOKEN".into()),
                request_timeout_ms: None,
            }],
            ..McpConfig::default()
        };

        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();
        assert_eq!(mgr.connected_count(), 1);

        // Call a tool via the manager
        let result = mgr
            .call_tool(
                "gbrain",
                "search",
                json!({"query": "servo", "source": "aurb"}),
            )
            .await;
        assert!(result.is_ok(), "call_tool failed: {:?}", result);
        let value = result.unwrap();
        assert_eq!(
            value
                .get("content")
                .and_then(|c| c.get(0))
                .and_then(|c| c.get("text"))
                .and_then(|t| t.as_str()),
            Some("search result")
        );

        std::env::remove_var("TEST_CALL_TOKEN");
    }

    #[tokio::test]
    async fn tool_level_error_is_not_reported_as_success() {
        std::env::set_var("TEST_TOOL_ERROR_TOKEN", "tool-error-token");
        let mut mock = MockMcpState::with_token("tool-error-token");
        mock.search_response = json!({
            "content": [{"type": "text", "text": "secret server detail"}],
            "isError": true
        });
        let state = Arc::new(Mutex::new(mock));
        let addr = spawn_mock_server(state).await;
        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "gbrain".into(),
                transport: McpTransportConfig::StreamableHttp {
                    url: format!("http://{}/mcp", addr),
                },
                trust: McpTrustLevel::RemoteTrusted,
                enabled: true,
                bearer_token_env: Some("TEST_TOOL_ERROR_TOKEN".into()),
                request_timeout_ms: None,
            }],
            ..McpConfig::default()
        };
        let mut manager = McpManager::new(config);
        manager.connect_all().await.unwrap();
        let error = manager
            .call_tool("gbrain", "put_page", json!({"slug":"s","content":"c"}))
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("application error"));
        assert!(!error.contains("secret server detail"));
        std::env::remove_var("TEST_TOOL_ERROR_TOKEN");
    }

    #[tokio::test]
    async fn http_401_error_does_not_contain_token() {
        std::env::set_var("TEST_401_TOKEN", "secret-do-not-leak");

        let state = Arc::new(Mutex::new(MockMcpState::with_401()));
        let addr = spawn_mock_server(state.clone()).await;

        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "gbrain".into(),
                transport: McpTransportConfig::StreamableHttp {
                    url: format!("http://{}/mcp", addr),
                },
                trust: McpTrustLevel::RemoteTrusted,
                enabled: true,
                bearer_token_env: Some("TEST_401_TOKEN".into()),
                request_timeout_ms: None,
            }],
            ..McpConfig::default()
        };

        let mut mgr = McpManager::new(config);
        let result = mgr.connect_all().await;
        // connect_all never fails overall - it just logs warnings
        assert!(result.is_ok());
        // But the server should NOT be connected (401)
        assert_eq!(mgr.connected_count(), 0);

        // Verify error does NOT contain the token value
        let s = state.lock().unwrap();
        // Check that HTTP transport returned a proper auth error
        // The auth header was sent with the token (which is fine, that's how auth works)
        // but any error message from the connect failure should NOT leak the token
        drop(s);

        // Try to call a tool - it should fail because server isn't connected
        let call_result = mgr
            .call_tool("gbrain", "search", json!({"query": "test"}))
            .await;
        assert!(call_result.is_err());
        let err_msg = format!("{}", call_result.unwrap_err());
        assert!(
            !err_msg.contains("secret-do-not-leak"),
            "Error message leaked token: {}",
            err_msg
        );

        std::env::remove_var("TEST_401_TOKEN");
    }

    #[tokio::test]
    async fn bearer_token_env_config_serialization_roundtrip() {
        let config = McpServerConfig {
            name: "test".into(),
            transport: McpTransportConfig::StreamableHttp {
                url: "http://localhost:9020/mcp".into(),
            },
            trust: McpTrustLevel::RemoteTrusted,
            enabled: true,
            bearer_token_env: Some("GBRAIN_READ_TOKEN".into()),
                request_timeout_ms: None,
        };

        let json_str = serde_json::to_string(&config).unwrap();
        let deserialized: McpServerConfig = serde_json::from_str(&json_str).unwrap();

        assert_eq!(
            deserialized.bearer_token_env.as_deref(),
            Some("GBRAIN_READ_TOKEN")
        );
    }

    #[tokio::test]
    async fn bearer_token_env_none_by_default() {
        let json_str = r#"{
            "name": "test",
            "transport": {"StreamableHttp": {"url": "http://localhost:9020/mcp"}},
            "trust": "RemoteTrusted",
            "enabled": true
        }"#;

        let config: McpServerConfig = serde_json::from_str(json_str).unwrap();
        assert!(config.bearer_token_env.is_none());
    }

    #[tokio::test]
    async fn disabled_server_not_connected() {
        std::env::set_var("TEST_DISABLED_TOKEN", "token123");

        let state = Arc::new(Mutex::new(MockMcpState::with_token("token123")));
        let addr = spawn_mock_server(state.clone()).await;

        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "gbrain".into(),
                transport: McpTransportConfig::StreamableHttp {
                    url: format!("http://{}/mcp", addr),
                },
                trust: McpTrustLevel::RemoteTrusted,
                enabled: false,
                bearer_token_env: Some("TEST_DISABLED_TOKEN".into()),
                request_timeout_ms: None,
            }],
            ..McpConfig::default()
        };

        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();
        assert_eq!(mgr.connected_count(), 0);

        std::env::remove_var("TEST_DISABLED_TOKEN");
    }

    // ---------------------------------------------------------------------------
    // D3-T6: Resource access tests
    // ---------------------------------------------------------------------------

    #[tokio::test]
    async fn resources_discovered_during_connect() {
        std::env::set_var("TEST_RESOURCE_TOKEN", "resource-token");

        let state = Arc::new(Mutex::new(MockMcpState::with_token("resource-token")));
        let addr = spawn_mock_server(state.clone()).await;

        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "gbrain".into(),
                transport: McpTransportConfig::StreamableHttp {
                    url: format!("http://{}/mcp", addr),
                },
                trust: McpTrustLevel::RemoteTrusted,
                enabled: true,
                bearer_token_env: Some("TEST_RESOURCE_TOKEN".into()),
                request_timeout_ms: None,
            }],
            ..McpConfig::default()
        };

        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();
        assert_eq!(mgr.connected_count(), 1);

        // Resource wrappers should be available
        let resource_tools = mgr.resource_wrappers();
        assert!(!resource_tools.is_empty(), "Should have discovered resources");
        assert_eq!(resource_tools.len(), 2);

        // Check naming convention: mcp.{server_name}.resource.{resource_name}
        let names: Vec<&str> = resource_tools.iter().map(|t| t.name()).collect();
        assert!(names.contains(&"mcp.gbrain.resource.docs_readme"));
        assert!(names.contains(&"mcp.gbrain.resource.app_config"));

        // Resources should be permission L0 (read-only)
        for tool in &resource_tools {
            assert!(
                matches!(tool.permission_level(), crate::tools::PermissionLevel::L0),
                "Resource should have L0 permission, got {:?}",
                tool.permission_level()
            );
        }

        std::env::remove_var("TEST_RESOURCE_TOKEN");
    }

    #[tokio::test]
    async fn read_resource_returns_content() {
        std::env::set_var("TEST_READ_RESOURCE_TOKEN", "read-resource-token");

        let state = Arc::new(Mutex::new(MockMcpState::with_token("read-resource-token")));
        let addr = spawn_mock_server(state.clone()).await;

        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "gbrain".into(),
                transport: McpTransportConfig::StreamableHttp {
                    url: format!("http://{}/mcp", addr),
                },
                trust: McpTrustLevel::RemoteTrusted,
                enabled: true,
                bearer_token_env: Some("TEST_READ_RESOURCE_TOKEN".into()),
                request_timeout_ms: None,
            }],
            ..McpConfig::default()
        };

        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();

        let result = mgr
            .read_resource("gbrain", "file:///docs/readme.md")
            .await;
        assert!(result.is_ok(), "read_resource failed: {:?}", result);
        let content = result.unwrap();
        assert!(content.text.contains("Content of file:///docs/readme.md"));
        assert_eq!(content.uri, "file:///docs/readme.md");

        std::env::remove_var("TEST_READ_RESOURCE_TOKEN");
    }

    #[tokio::test]
    async fn list_resources_returns_known_resources() {
        std::env::set_var("TEST_LIST_RESOURCES_TOKEN", "list-resources-token");

        let state = Arc::new(Mutex::new(MockMcpState::with_token("list-resources-token")));
        let addr = spawn_mock_server(state.clone()).await;

        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "gbrain".into(),
                transport: McpTransportConfig::StreamableHttp {
                    url: format!("http://{}/mcp", addr),
                },
                trust: McpTrustLevel::RemoteTrusted,
                enabled: true,
                bearer_token_env: Some("TEST_LIST_RESOURCES_TOKEN".into()),
                request_timeout_ms: None,
            }],
            ..McpConfig::default()
        };

        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();

        let resources = mgr.list_resources("gbrain").await.unwrap();
        assert_eq!(resources.len(), 2);
        assert_eq!(resources[0].uri, "file:///docs/readme.md");
        assert_eq!(resources[1].uri, "file:///config/app.toml");

        std::env::remove_var("TEST_LIST_RESOURCES_TOKEN");
    }

    #[tokio::test]
    async fn resource_wrapper_executes_successfully() {
        std::env::set_var("TEST_RESOURCE_EXEC_TOKEN", "resource-exec-token");

        let state = Arc::new(Mutex::new(MockMcpState::with_token("resource-exec-token")));
        let addr = spawn_mock_server(state.clone()).await;

        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "gbrain".into(),
                transport: McpTransportConfig::StreamableHttp {
                    url: format!("http://{}/mcp", addr),
                },
                trust: McpTrustLevel::RemoteTrusted,
                enabled: true,
                bearer_token_env: Some("TEST_RESOURCE_EXEC_TOKEN".into()),
                request_timeout_ms: None,
            }],
            ..McpConfig::default()
        };

        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();

        let resource_tools = mgr.resource_wrappers();
        let readme_tool = resource_tools
            .iter()
            .find(|t| t.name() == "mcp.gbrain.resource.docs_readme")
            .expect("Should have docs_readme resource tool");

        use std::path::PathBuf;
        let ctx = fabric::ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: PathBuf::from("/tmp"),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        };
        let result = readme_tool.execute(serde_json::json!({}), &ctx).await;

        assert!(
            !result.is_error,
            "Resource execution should succeed, got: {}",
            result.content
        );
        assert!(
            result.content.contains("Content of file:///docs/readme.md"),
            "Should contain resource content, got: {}",
            result.content
        );

        std::env::remove_var("TEST_RESOURCE_EXEC_TOKEN");
    }

    #[tokio::test]
    async fn parallel_tool_calls_server_sets_concurrency_class_readonly() {
        std::env::set_var("TEST_PARALLEL_TOKEN", "parallel-token");

        let state = Arc::new(Mutex::new(MockMcpState::with_parallel_tool_calls()));
        let addr = spawn_mock_server(state).await;

        let config = McpConfig {
            servers: vec![McpServerConfig {
                name: "parallel-server".into(),
                transport: McpTransportConfig::StreamableHttp {
                    url: format!("http://{}/mcp", addr),
                },
                trust: McpTrustLevel::RemoteTrusted,
                enabled: true,
                bearer_token_env: Some("TEST_PARALLEL_TOKEN".into()),
                request_timeout_ms: None,
            }],
            ..McpConfig::default()
        };

        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();
        assert_eq!(mgr.connected_count(), 1);

        let wrappers = mgr.tool_wrappers();
        assert!(!wrappers.is_empty(), "Should have discovered tools");

        for tool in &wrappers {
            let cc = tool.concurrency_class();
            assert_eq!(
                cc,
                ConcurrencyClass::ReadOnly,
                "Tool from parallel-capable server should have ReadOnly concurrency class, got {:?}",
                cc
            );
        }

        std::env::remove_var("TEST_PARALLEL_TOKEN");
    }
}
