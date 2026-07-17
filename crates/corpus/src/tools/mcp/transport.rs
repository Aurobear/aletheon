use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

use super::auth::BearerTokenAuth;
use super::auth::McpAuth;

const MAX_HTTP_RESPONSE_BYTES: usize = 1024 * 1024;

// ---------------------------------------------------------------------------
// Transport enum
// ---------------------------------------------------------------------------

/// MCP transport abstraction.
///
/// Three transport variants:
/// - **Stdio** — subprocess with stdin/stdout pipes.
/// - **StreamableHttp** — HTTP POST for requests, SSE for streaming responses.
///   Falls back to `Sse` on non-auth connection failures.
/// - **Sse** — HTTP GET long-poll connection (legacy fallback).
pub enum McpTransport {
    Stdio {
        stdin_tx: mpsc::Sender<String>,
        stdout_rx: mpsc::Receiver<String>,
        _child: Child,
    },
    StreamableHttp {
        client: reqwest::Client,
        base_url: String,
        auth: Option<BearerTokenAuth>,
    },
    Sse {
        client: reqwest::Client,
        base_url: String,
        auth: Option<BearerTokenAuth>,
        event_rx: mpsc::Receiver<String>,
        _event_handle: tokio::task::JoinHandle<()>,
    },
}

// ---------------------------------------------------------------------------
// Tool name normalization
// ---------------------------------------------------------------------------

/// Strategy for resolving tool name collisions across MCP servers.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CollisionStrategy {
    /// Prefix tool names with the server name: `<server>__<tool>`.
    PrefixServer,
    /// Append a numeric suffix: `<tool>`, `<tool>_2`, `<tool>_3`, ...
    NumericSuffix,
    /// First server wins; later duplicates are silently skipped.
    FirstWins,
}

/// Configuration for tool name normalization.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ToolNameConfig {
    /// Whether to apply the server-name prefix at all.
    pub enable_prefix: bool,
    /// Maximum length for a normalized tool name.
    pub max_length: usize,
    /// How to handle name collisions between servers.
    pub collision_strategy: CollisionStrategy,
}

impl Default for ToolNameConfig {
    fn default() -> Self {
        Self {
            enable_prefix: true,
            max_length: 64,
            collision_strategy: CollisionStrategy::PrefixServer,
        }
    }
}

impl ToolNameConfig {
    /// Normalize a tool name given its server name and a set of already-seen names.
    ///
    /// Returns the normalized name. `seen` is mutated in-place to track
    /// collisions.
    pub fn normalize(
        &self,
        server_name: &str,
        tool_name: &str,
        seen: &mut std::collections::HashSet<String>,
    ) -> String {
        let candidate = if self.enable_prefix {
            format!("{}__{}", server_name, tool_name)
        } else {
            tool_name.to_string()
        };

        let candidate = truncate_to(candidate, self.max_length);

        match self.collision_strategy {
            CollisionStrategy::PrefixServer => {
                // With prefix, collisions are rare but possible if two servers
                // share the same name. Fall through to numeric suffix.
                insert_with_numeric_suffix(&candidate, seen)
            }
            CollisionStrategy::NumericSuffix => insert_with_numeric_suffix(tool_name, seen),
            CollisionStrategy::FirstWins => {
                if seen.contains(&candidate) {
                    // Return the name but do NOT insert — caller can skip.
                    candidate
                } else {
                    seen.insert(candidate.clone());
                    candidate
                }
            }
        }
    }
}

fn truncate_to(mut s: String, max_len: usize) -> String {
    fabric::truncate_utf8_bytes(&mut s, max_len);
    s
}

fn insert_with_numeric_suffix(base: &str, seen: &mut std::collections::HashSet<String>) -> String {
    if !seen.contains(base) {
        seen.insert(base.to_string());
        return base.to_string();
    }
    for i in 2.. {
        let candidate = format!("{}_{}", base, i);
        if !seen.contains(&candidate) {
            seen.insert(candidate.clone());
            return candidate;
        }
    }
    unreachable!()
}

// ---------------------------------------------------------------------------
// Notifications
// ---------------------------------------------------------------------------

/// Parsed MCP notification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum McpNotification {
    /// `notifications/tools/list_changed` — the server's tool list changed.
    ToolsListChanged,
    /// Unknown notification method.
    Unknown(String),
}

impl McpNotification {
    /// Parse a JSON-RPC notification (a message with `method` but no `id`).
    pub fn parse(msg: &Value) -> Option<Self> {
        // Notifications have no "id" field.
        if msg.get("id").is_some() {
            return None;
        }
        let method = msg.get("method")?.as_str()?;
        match method {
            "notifications/tools/list_changed" => Some(Self::ToolsListChanged),
            other => Some(Self::Unknown(other.to_string())),
        }
    }
}

// ---------------------------------------------------------------------------
// Auth-error detection (used by fallback logic)
// ---------------------------------------------------------------------------

/// Returns `true` if the error represents an authentication/authorization
/// failure (HTTP 401 or 403). Fallback must NOT happen for these.
pub fn is_auth_error(err: &anyhow::Error) -> bool {
    let msg = format!("{:?}", err);
    msg.contains("401") || msg.contains("403")
}

// ---------------------------------------------------------------------------
// Constructors
// ---------------------------------------------------------------------------

impl McpTransport {
    /// Create a stdio transport by spawning a subprocess.
    pub async fn stdio(command: &str, args: &[String]) -> Result<Self> {
        let mut child = Command::new(command)
            .args(args)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()?;

        let stdin = child.stdin.take().expect("stdin was piped");
        let stdout = child.stdout.take().expect("stdout was piped");

        let (stdin_tx, mut stdin_rx) = mpsc::channel::<String>(32);
        let (stdout_tx, stdout_rx) = mpsc::channel::<String>(32);

        // stdin writer task
        tokio::spawn(async move {
            let mut stdin = stdin;
            while let Some(msg) = stdin_rx.recv().await {
                if stdin.write_all(msg.as_bytes()).await.is_err() {
                    break;
                }
                if stdin.write_all(b"\n").await.is_err() {
                    break;
                }
                if stdin.flush().await.is_err() {
                    break;
                }
            }
        });

        // stdout reader task
        tokio::spawn(async move {
            let mut reader = BufReader::new(stdout);
            let mut line = String::new();
            loop {
                line.clear();
                match reader.read_line(&mut line).await {
                    Ok(0) => break,
                    Ok(_) => {
                        let _ = stdout_tx.send(line.trim().to_string()).await;
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self::Stdio {
            stdin_tx,
            stdout_rx,
            _child: child,
        })
    }

    /// Create a StreamableHttp transport.
    ///
    /// Uses HTTP POST to send JSON-RPC requests and reads the response
    /// body (optionally as SSE). Auth token is read from the env var
    /// `MCP_BEARER_TOKEN` (or `None` if unset).
    pub fn streamable_http(base_url: impl Into<String>, auth: Option<BearerTokenAuth>) -> Self {
        Self::StreamableHttp {
            client: reqwest::Client::new(),
            base_url: base_url.into(),
            auth,
        }
    }

    /// Create an SSE transport.
    ///
    /// Opens an HTTP GET long-poll connection to `<base_url>/sse` and
    /// reads events. Requests are sent as HTTP POST to `<base_url>`.
    pub async fn sse(base_url: impl Into<String>, auth: Option<BearerTokenAuth>) -> Result<Self> {
        let base_url = base_url.into();
        let client = reqwest::Client::new();

        let sse_url = format!("{}/sse", base_url.trim_end_matches('/'));
        let mut req_builder = client.get(&sse_url);
        if let Some(ref a) = auth {
            if let Some(hv) = a.header_value() {
                req_builder = req_builder.header("Authorization", hv);
            }
        }

        let response = req_builder.send().await.context("SSE connection failed")?;

        if !response.status().is_success() {
            let status = response.status().as_u16();
            anyhow::bail!("SSE connection returned HTTP {}", status);
        }

        let (event_tx, event_rx) = mpsc::channel::<String>(128);

        let handle = tokio::spawn(async move {
            let mut stream = response.bytes_stream();
            let mut buffer = String::new();
            while let Some(chunk_result) = stream.next().await {
                let chunk = match chunk_result {
                    Ok(c) => c,
                    Err(_) => break,
                };
                buffer.push_str(&String::from_utf8_lossy(&chunk));
                // Process complete SSE events (delimited by \n\n)
                while let Some(pos) = buffer.find("\n\n") {
                    let event_str = buffer[..pos].to_string();
                    buffer = buffer[pos + 2..].to_string();
                    // Extract "data:" lines
                    for line in event_str.lines() {
                        if let Some(data) = line.strip_prefix("data:") {
                            let data = data.trim();
                            if !data.is_empty() {
                                let _ = event_tx.send(data.to_string()).await;
                            }
                        }
                    }
                }
            }
        });

        Ok(Self::Sse {
            client,
            base_url,
            auth,
            event_rx,
            _event_handle: handle,
        })
    }

    // -----------------------------------------------------------------------
    // Request / response
    // -----------------------------------------------------------------------

    /// Send a JSON-RPC request and wait for the response.
    pub async fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value> {
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });

        match self {
            Self::Stdio {
                stdin_tx,
                stdout_rx,
                ..
            } => {
                stdin_tx
                    .send(serde_json::to_string(&request)?)
                    .await
                    .map_err(|_| anyhow::anyhow!("Transport stdin closed"))?;
                let response_str = stdout_rx
                    .recv()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Transport closed"))?;
                Self::parse_response(&response_str)
            }

            Self::StreamableHttp {
                client,
                base_url,
                auth,
            } => {
                let url = base_url.trim_end_matches('/').to_string();
                Self::http_post(client, &url, auth, &request).await
            }

            Self::Sse {
                client,
                base_url,
                auth,
                event_rx,
                ..
            } => {
                let url = base_url.trim_end_matches('/').to_string();
                // Fire the POST request.
                let _post_result = Self::http_post_no_response(client, &url, auth, &request).await;
                // Wait for a response event from the SSE stream.
                // For simplicity we read one event and parse it as JSON-RPC response.
                let event_str = event_rx
                    .recv()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("SSE stream closed"))?;
                Self::parse_response(&event_str)
            }
        }
    }

    /// Send a JSON-RPC notification (no response expected).
    pub async fn notify(&mut self, method: &str, params: Value) -> Result<()> {
        let notification = serde_json::json!({
            "jsonrpc": "2.0",
            "method": method,
            "params": params,
        });

        match self {
            Self::Stdio { stdin_tx, .. } => {
                stdin_tx
                    .send(serde_json::to_string(&notification)?)
                    .await
                    .map_err(|_| anyhow::anyhow!("Transport stdin closed"))?;
                Ok(())
            }
            Self::StreamableHttp {
                client,
                base_url,
                auth,
            } => {
                let url = base_url.trim_end_matches('/').to_string();
                Self::http_post_no_response(client, &url, auth, &notification).await
            }
            Self::Sse {
                client,
                base_url,
                auth,
                ..
            } => {
                let url = base_url.trim_end_matches('/').to_string();
                Self::http_post_no_response(client, &url, auth, &notification).await
            }
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn parse_response(raw: &str) -> Result<Value> {
        let response: Value =
            serde_json::from_str(raw).context("Failed to parse JSON-RPC response")?;
        if let Some(error) = response.get("error") {
            return Err(anyhow::anyhow!("MCP error: {}", error));
        }
        Ok(response.get("result").cloned().unwrap_or(Value::Null))
    }

    async fn http_post(
        client: &reqwest::Client,
        url: &str,
        auth: &Option<BearerTokenAuth>,
        body: &Value,
    ) -> Result<Value> {
        let mut req = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(body);

        if let Some(a) = auth {
            let headers = a.get_headers(Some(url));
            if let Some(hv) = headers.get("Authorization") {
                req = req.header("Authorization", hv);
            }
        }

        let response = req.send().await.context("HTTP POST failed")?;
        let status = response.status();

        if status == reqwest::StatusCode::UNAUTHORIZED || status == reqwest::StatusCode::FORBIDDEN {
            anyhow::bail!("Authentication failed: HTTP {}", status);
        }

        if !status.is_success() {
            anyhow::bail!("HTTP request failed: {}", status);
        }

        // Check content type: if SSE, read event stream; otherwise read JSON.
        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("")
            .to_string();

        if content_type.contains("text/event-stream") {
            // Parse SSE response: read data lines from the body.
            let body_text = Self::read_bounded_http_body(response).await?;
            for line in body_text.lines() {
                if let Some(data) = line.strip_prefix("data:") {
                    let data = data.trim();
                    if !data.is_empty() {
                        return Self::parse_response(data);
                    }
                }
            }
            anyhow::bail!("No data event found in SSE response");
        } else {
            let body_text = Self::read_bounded_http_body(response).await?;
            Self::parse_response(&body_text)
        }
    }

    async fn read_bounded_http_body(response: reqwest::Response) -> Result<String> {
        if response
            .content_length()
            .is_some_and(|length| length > MAX_HTTP_RESPONSE_BYTES as u64)
        {
            anyhow::bail!("MCP HTTP response exceeds byte limit");
        }
        let mut stream = response.bytes_stream();
        let mut bytes = Vec::new();
        while let Some(chunk) = stream.next().await {
            let chunk = chunk?;
            if bytes.len().saturating_add(chunk.len()) > MAX_HTTP_RESPONSE_BYTES {
                anyhow::bail!("MCP HTTP response exceeds byte limit");
            }
            bytes.extend_from_slice(&chunk);
        }
        String::from_utf8(bytes).context("MCP HTTP response is not UTF-8")
    }

    async fn http_post_no_response(
        client: &reqwest::Client,
        url: &str,
        auth: &Option<BearerTokenAuth>,
        body: &Value,
    ) -> Result<()> {
        let mut req = client
            .post(url)
            .header("Content-Type", "application/json")
            .json(body);

        if let Some(a) = auth {
            let headers = a.get_headers(Some(url));
            if let Some(hv) = headers.get("Authorization") {
                req = req.header("Authorization", hv);
            }
        }

        let response = req.send().await.context("HTTP POST failed")?;
        if !response.status().is_success() {
            anyhow::bail!("HTTP POST failed: {}", response.status());
        }
        Ok(())
    }
}

// ---------------------------------------------------------------------------
// Fallback helper (StreamableHttp -> Sse)
// ---------------------------------------------------------------------------

/// Attempt a StreamableHttp connection; on non-auth failure, fall back to SSE.
///
/// This is a convenience for callers. Auth errors (401/403) propagate
/// immediately without fallback.
pub async fn connect_with_fallback(
    base_url: &str,
    auth: Option<BearerTokenAuth>,
) -> Result<McpTransport> {
    // Try StreamableHttp first by sending a probe POST.
    let transport = McpTransport::streamable_http(base_url, auth.clone());
    if let McpTransport::StreamableHttp { ref client, .. } = transport {
        let probe = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 0,
            "method": "initialize",
            "params": {
                "protocolVersion": "2024-11-05",
                "capabilities": {},
                "clientInfo": { "name": "aletheon", "version": "0.1.0" }
            }
        });
        match McpTransport::http_post(client, base_url.trim_end_matches('/'), &auth, &probe).await {
            Ok(_) => return Ok(transport),
            Err(e) if is_auth_error(&e) => return Err(e),
            Err(_) => {
                tracing::info!(
                    "StreamableHttp failed for {}, falling back to SSE",
                    base_url
                );
            }
        }
    }

    // Fallback to SSE.
    McpTransport::sse(base_url, auth).await
}

// ===========================================================================
// Tests
// ===========================================================================

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_collision_strategy_prefix_server() {
        let config = ToolNameConfig {
            enable_prefix: true,
            max_length: 64,
            collision_strategy: CollisionStrategy::PrefixServer,
        };
        let mut seen = std::collections::HashSet::new();

        let n1 = config.normalize("server_a", "read_file", &mut seen);
        assert_eq!(n1, "server_a__read_file");

        let n2 = config.normalize("server_b", "read_file", &mut seen);
        assert_eq!(n2, "server_b__read_file");

        // Same server + same tool = collision -> numeric suffix.
        let n3 = config.normalize("server_a", "read_file", &mut seen);
        assert_eq!(n3, "server_a__read_file_2");
    }

    #[test]
    fn test_collision_strategy_numeric_suffix() {
        let config = ToolNameConfig {
            enable_prefix: false,
            max_length: 64,
            collision_strategy: CollisionStrategy::NumericSuffix,
        };
        let mut seen = std::collections::HashSet::new();

        let n1 = config.normalize("s1", "list", &mut seen);
        assert_eq!(n1, "list");

        let n2 = config.normalize("s2", "list", &mut seen);
        assert_eq!(n2, "list_2");

        let n3 = config.normalize("s3", "list", &mut seen);
        assert_eq!(n3, "list_3");
    }

    #[test]
    fn test_collision_strategy_first_wins() {
        let config = ToolNameConfig {
            enable_prefix: false,
            max_length: 64,
            collision_strategy: CollisionStrategy::FirstWins,
        };
        let mut seen = std::collections::HashSet::new();

        let n1 = config.normalize("s1", "deploy", &mut seen);
        assert_eq!(n1, "deploy");
        assert!(seen.contains("deploy"));

        // Second server with same tool -- name returned but NOT inserted.
        let n2 = config.normalize("s2", "deploy", &mut seen);
        assert_eq!(n2, "deploy");
        assert!(!seen.contains("deploy_2"));
    }

    #[test]
    fn test_tool_name_truncation() {
        let config = ToolNameConfig {
            enable_prefix: true,
            max_length: 10,
            collision_strategy: CollisionStrategy::PrefixServer,
        };
        let mut seen = std::collections::HashSet::new();

        let name = config.normalize("my_server", "very_long_tool_name", &mut seen);
        assert!(name.len() <= 10);
    }

    #[test]
    fn test_tool_name_truncation_is_utf8_safe() {
        let config = ToolNameConfig {
            enable_prefix: false,
            max_length: 10,
            collision_strategy: CollisionStrategy::FirstWins,
        };
        let mut seen = std::collections::HashSet::new();

        let name = config.normalize("server", "中文🙂中文🙂", &mut seen);
        assert!(name.len() <= 10);
        assert!(name.is_char_boundary(name.len()));
    }

    #[test]
    fn test_notification_parse_tools_list_changed() {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "notifications/tools/list_changed",
            "params": {}
        });
        let notif = McpNotification::parse(&msg);
        assert_eq!(notif, Some(McpNotification::ToolsListChanged));
    }

    #[test]
    fn test_notification_parse_ignores_requests() {
        // Has "id" -> it's a request, not a notification.
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "notifications/tools/list_changed"
        });
        assert!(McpNotification::parse(&msg).is_none());
    }

    #[test]
    fn test_notification_parse_unknown_method() {
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "method": "custom/event",
            "params": {}
        });
        let notif = McpNotification::parse(&msg);
        assert_eq!(
            notif,
            Some(McpNotification::Unknown("custom/event".to_string()))
        );
    }

    #[test]
    fn test_notification_parse_no_method() {
        let msg = serde_json::json!({ "jsonrpc": "2.0" });
        assert!(McpNotification::parse(&msg).is_none());
    }

    #[test]
    fn test_is_auth_error() {
        let auth_err = anyhow::anyhow!("HTTP 401 Unauthorized");
        assert!(is_auth_error(&auth_err));

        let auth_err2 = anyhow::anyhow!("HTTP 403 Forbidden");
        assert!(is_auth_error(&auth_err2));

        let other_err = anyhow::anyhow!("connection refused");
        assert!(!is_auth_error(&other_err));
    }

    #[test]
    fn test_streamable_http_config_construction() {
        let auth = BearerTokenAuth::new("TEST_TRANSPORT_TOKEN");
        let transport = McpTransport::streamable_http("http://localhost:8080/mcp", Some(auth));
        match &transport {
            McpTransport::StreamableHttp { base_url, .. } => {
                assert_eq!(base_url, "http://localhost:8080/mcp");
            }
            _ => panic!("Expected StreamableHttp variant"),
        }
    }

    #[test]
    fn test_streamable_http_no_auth() {
        let transport = McpTransport::streamable_http("http://localhost:3000", None);
        match &transport {
            McpTransport::StreamableHttp { auth, .. } => {
                assert!(auth.is_none());
            }
            _ => panic!("Expected StreamableHttp variant"),
        }
    }

    #[test]
    fn test_collision_strategy_serialization_roundtrip() {
        let strategies = vec![
            CollisionStrategy::PrefixServer,
            CollisionStrategy::NumericSuffix,
            CollisionStrategy::FirstWins,
        ];
        for s in strategies {
            let json = serde_json::to_string(&s).unwrap();
            let back: CollisionStrategy = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn test_tool_name_config_default() {
        let config = ToolNameConfig::default();
        assert!(config.enable_prefix);
        assert_eq!(config.max_length, 64);
        assert_eq!(config.collision_strategy, CollisionStrategy::PrefixServer);
    }
}
