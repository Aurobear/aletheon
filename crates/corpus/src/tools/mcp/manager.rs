//! High-level MCP facade for the daemon: connect configured servers and expose
//! their tools as `Box<dyn Tool>` ready to register into the ToolRegistry.

use anyhow::Result;
use serde_json::Value;

use super::client::McpConnectionManager;
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

    /// Number of servers that were successfully connected.
    pub fn connected_count(&self) -> usize {
        self.inner.connected_count()
    }

    pub fn server_has_tools(&self, server_name: &str, required: &[&str]) -> bool {
        self.inner.server_has_tools(server_name, required)
    }
}

#[cfg(test)]
mod tests {
    use super::super::config::{McpServerConfig, McpTransportConfig, McpTrustLevel};
    use super::*;
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
            }
        }

        fn with_401() -> Self {
            let mut state = Self::with_token("test-token");
            state.return_401 = true;
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
            let (return_401, expected_token, tools_list, search_resp) = {
                let s = state.lock().unwrap();
                (
                    s.return_401,
                    s.expected_token.clone(),
                    s.tools_list_response.clone(),
                    s.search_response.clone(),
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
                "initialize" => json!({
                    "protocolVersion": "2024-11-05",
                    "capabilities": {"tools": {}},
                    "serverInfo": {"name": "test-mock", "version": "1.0.0"}
                }),
                "tools/list" => tools_list,
                "tools/call" => search_resp,
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
            }],
            ..McpConfig::default()
        };

        let mut mgr = McpManager::new(config);
        mgr.connect_all().await.unwrap();
        assert_eq!(mgr.connected_count(), 0);

        std::env::remove_var("TEST_DISABLED_TOKEN");
    }
}
