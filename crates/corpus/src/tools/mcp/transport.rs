use anyhow::{Context, Result};
use futures::StreamExt;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::mpsc;

#[cfg(test)]
use super::auth::BearerTokenAuth;
use super::auth::McpHttpAuth;

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
        request_timeout_ms: u64,
    },
    StreamableHttp {
        client: reqwest::Client,
        base_url: String,
        auth: Option<McpHttpAuth>,
        request_timeout_ms: u64,
    },
    Sse {
        client: reqwest::Client,
        base_url: String,
        auth: Option<McpHttpAuth>,
        event_rx: mpsc::Receiver<String>,
        _event_handle: tokio::task::JoinHandle<()>,
        request_timeout_ms: u64,
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
    /// Server-to-client approval request.
    ElicitationCreate { id: u64, params: Value },
    /// Unknown notification method.
    Unknown(String),
}

impl McpNotification {
    /// Parse a supported inbound notification or server-to-client request.
    pub fn parse(msg: &Value) -> Option<Self> {
        let method = msg.get("method")?.as_str()?;
        if let Some(id) = msg.get("id").and_then(Value::as_u64) {
            return (method == "elicitation/create").then(|| Self::ElicitationCreate {
                id,
                params: msg.get("params").cloned().unwrap_or(Value::Null),
            });
        }
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
    pub async fn stdio(
        command: &str,
        args: &[String],
        request_timeout_ms: u64,
        notification_tx: mpsc::Sender<McpNotification>,
    ) -> Result<Self> {
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
                        let message = line.trim();
                        if let Ok(value) = serde_json::from_str::<Value>(message) {
                            if let Some(notification) = McpNotification::parse(&value) {
                                let _ = notification_tx.send(notification).await;
                                continue;
                            }
                        }
                        let _ = stdout_tx.send(message.to_string()).await;
                    }
                    Err(_) => break,
                }
            }
        });

        Ok(Self::Stdio {
            stdin_tx,
            stdout_rx,
            _child: child,
            request_timeout_ms,
        })
    }

    /// Create a StreamableHttp transport.
    ///
    /// Uses HTTP POST to send JSON-RPC requests and reads the response
    /// body (optionally as SSE). Auth token is read from the env var
    /// `MCP_BEARER_TOKEN` (or `None` if unset).
    pub fn streamable_http(
        base_url: impl Into<String>,
        auth: Option<McpHttpAuth>,
        request_timeout_ms: u64,
    ) -> Self {
        Self::StreamableHttp {
            client: reqwest::Client::builder()
                .redirect(reqwest::redirect::Policy::none())
                .build()
                .expect("static MCP HTTP client configuration must build"),
            base_url: base_url.into(),
            auth,
            request_timeout_ms,
        }
    }

    /// Create an SSE transport.
    ///
    /// Opens an HTTP GET long-poll connection to `<base_url>/sse` and
    /// reads events. Requests are sent as HTTP POST to `<base_url>`.
    pub async fn sse(
        base_url: impl Into<String>,
        auth: Option<McpHttpAuth>,
        request_timeout_ms: u64,
        notification_tx: mpsc::Sender<McpNotification>,
    ) -> Result<Self> {
        let base_url = base_url.into();
        let client = reqwest::Client::builder()
            .redirect(reqwest::redirect::Policy::none())
            .build()
            .context("build MCP SSE client")?;

        let sse_url = format!("{}/sse", base_url.trim_end_matches('/'));
        let mut req_builder = client.get(&sse_url);
        if let Some(ref a) = auth {
            if let Some(hv) = a.header_value_for(Some(&sse_url)) {
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
                                if let Ok(value) = serde_json::from_str::<Value>(data) {
                                    if let Some(notification) = McpNotification::parse(&value) {
                                        let _ = notification_tx.send(notification).await;
                                        continue;
                                    }
                                }
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
            request_timeout_ms,
        })
    }

    // -----------------------------------------------------------------------
    // Request / response
    // -----------------------------------------------------------------------

    /// Send a JSON-RPC request and wait for the response.
    ///
    /// For HTTP-based transports, transient failures (5xx, connection errors,
    /// timeouts) are retried up to 2 times with exponential backoff (1s, 2s, 4s).
    /// Non-retryable errors (4xx except 429) fail immediately.
    pub async fn request(&mut self, id: u64, method: &str, params: Value) -> Result<Value> {
        let timeout_ms = self.request_timeout_ms();
        let timeout_dur = std::time::Duration::from_millis(timeout_ms);

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
            } => tokio::time::timeout(timeout_dur, async {
                stdin_tx
                    .send(serde_json::to_string(&request)?)
                    .await
                    .map_err(|_| anyhow::anyhow!("Transport stdin closed"))?;
                let response_str = stdout_rx
                    .recv()
                    .await
                    .ok_or_else(|| anyhow::anyhow!("Transport closed"))?;
                Self::parse_response(&response_str)
            })
            .await
            .map_err(|_elapsed| {
                anyhow::anyhow!("MCP stdio request timed out after {}ms", timeout_ms)
            })?,

            Self::StreamableHttp {
                client,
                base_url,
                auth,
                ..
            } => {
                let url = base_url.trim_end_matches('/').to_string();
                Self::http_request_with_retry(client, &url, auth, &request, timeout_ms).await
            }

            Self::Sse {
                client,
                base_url,
                auth,
                event_rx,
                ..
            } => {
                let url = base_url.trim_end_matches('/').to_string();
                Self::sse_request_with_retry(client, &url, auth, &request, event_rx, timeout_ms)
                    .await
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
                ..
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

    /// Respond to a server-to-client JSON-RPC request.
    pub async fn send_response(&mut self, id: u64, result: Value) -> Result<()> {
        let response = serde_json::json!({ "jsonrpc": "2.0", "id": id, "result": result });
        match self {
            Self::Stdio { stdin_tx, .. } => stdin_tx
                .send(serde_json::to_string(&response)?)
                .await
                .map_err(|_| anyhow::anyhow!("Transport stdin closed")),
            Self::StreamableHttp {
                client,
                base_url,
                auth,
                ..
            }
            | Self::Sse {
                client,
                base_url,
                auth,
                ..
            } => {
                Self::http_post_no_response(client, base_url.trim_end_matches('/'), auth, &response)
                    .await
            }
        }
    }

    // -----------------------------------------------------------------------
    // Internal helpers
    // -----------------------------------------------------------------------

    fn request_timeout_ms(&self) -> u64 {
        match self {
            Self::Stdio {
                request_timeout_ms, ..
            } => *request_timeout_ms,
            Self::StreamableHttp {
                request_timeout_ms, ..
            } => *request_timeout_ms,
            Self::Sse {
                request_timeout_ms, ..
            } => *request_timeout_ms,
        }
    }

    /// Returns `true` when the error represents a transient failure that
    /// should be retried (5xx, 429, connection errors, timeouts).
    fn is_retryable(err: &anyhow::Error) -> bool {
        let msg = format!("{:?}", err);
        // 5xx server errors are retryable
        if msg.contains("500") || msg.contains("502") || msg.contains("503") || msg.contains("504")
        {
            return true;
        }
        // 429 Too Many Requests is retryable
        if msg.contains("429") {
            return true;
        }
        // Connection errors are retryable
        if msg.contains("connection refused")
            || msg.contains("connection reset")
            || msg.contains("broken pipe")
            || msg.contains("tcp connect error")
            || msg.contains("dns error")
            || msg.contains("error trying to connect")
        {
            return true;
        }
        // Timeout errors are retryable
        if msg.contains("timed out") || msg.contains("timeout") {
            return true;
        }
        // 4xx client errors (except 429) are not retryable
        false
    }

    /// Execute an HTTP POST with retry for transient failures.
    ///
    /// Retries up to 2 times (3 total attempts) with exponential backoff:
    /// 100ms then 200ms between attempts, bounded by the per-server timeout.
    async fn http_request_with_retry(
        client: &reqwest::Client,
        url: &str,
        auth: &Option<McpHttpAuth>,
        body: &Value,
        request_timeout_ms: u64,
    ) -> Result<Value> {
        const MAX_RETRIES: u32 = 2;
        let timeout_dur = std::time::Duration::from_millis(request_timeout_ms);
        let deadline = tokio::time::Instant::now() + timeout_dur;
        let mut delay_ms: u64 = 100;
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let delay = std::time::Duration::from_millis(delay_ms);
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if delay >= remaining {
                    break;
                }
                tokio::time::sleep(delay).await;
                delay_ms = delay_ms.saturating_mul(2);
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            let result =
                tokio::time::timeout(remaining, Self::http_post(client, url, auth, body)).await;

            match result {
                Ok(Ok(value)) => return Ok(value),
                Ok(Err(e)) => {
                    if !Self::is_retryable(&e) {
                        return Err(e);
                    }
                    last_error = Some(e);
                }
                Err(_elapsed) => {
                    last_error = Some(anyhow::anyhow!(
                        "MCP HTTP request timed out after {}ms",
                        request_timeout_ms
                    ));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("MCP HTTP request failed")))
    }

    /// Execute an SSE request with retry for transient failures.
    ///
    /// Fires a POST (no-body read), then reads the response from the SSE
    /// event stream. On transient POST failures, retries the POST; if the
    /// event stream closes, that is not retryable.
    async fn sse_request_with_retry(
        client: &reqwest::Client,
        url: &str,
        auth: &Option<McpHttpAuth>,
        body: &Value,
        event_rx: &mut mpsc::Receiver<String>,
        request_timeout_ms: u64,
    ) -> Result<Value> {
        const MAX_RETRIES: u32 = 2;
        let timeout_dur = std::time::Duration::from_millis(request_timeout_ms);
        let deadline = tokio::time::Instant::now() + timeout_dur;
        let mut delay_ms: u64 = 100;
        let mut last_error: Option<anyhow::Error> = None;

        for attempt in 0..=MAX_RETRIES {
            if attempt > 0 {
                let delay = std::time::Duration::from_millis(delay_ms);
                let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                if delay >= remaining {
                    break;
                }
                tokio::time::sleep(delay).await;
                delay_ms = delay_ms.saturating_mul(2);
            }

            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                break;
            }
            // POST the request
            let post_result = tokio::time::timeout(
                remaining,
                Self::http_post_no_response(client, url, auth, body),
            )
            .await;

            match post_result {
                Ok(Ok(())) => {
                    // POST succeeded — read from the SSE event stream
                    let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
                    let event_result = tokio::time::timeout(remaining, event_rx.recv()).await;
                    match event_result {
                        Ok(Some(event_str)) => {
                            return Self::parse_response(&event_str);
                        }
                        Ok(None) => {
                            return Err(anyhow::anyhow!("SSE stream closed"));
                        }
                        Err(_elapsed) => {
                            last_error = Some(anyhow::anyhow!(
                                "MCP SSE request timed out after {}ms",
                                request_timeout_ms
                            ));
                        }
                    }
                }
                Ok(Err(e)) => {
                    if !Self::is_retryable(&e) {
                        return Err(e);
                    }
                    last_error = Some(e);
                }
                Err(_elapsed) => {
                    last_error = Some(anyhow::anyhow!(
                        "MCP SSE POST timed out after {}ms",
                        request_timeout_ms
                    ));
                }
            }
        }

        Err(last_error.unwrap_or_else(|| anyhow::anyhow!("MCP SSE request failed")))
    }

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
        auth: &Option<McpHttpAuth>,
        body: &Value,
    ) -> Result<Value> {
        let mut req = client
            .post(url)
            .header("Content-Type", "application/json")
            .header("Accept", "application/json, text/event-stream")
            .json(body);

        if let Some(a) = auth {
            if let Some(hv) = a.header_value_for(Some(url)) {
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
        auth: &Option<McpHttpAuth>,
        body: &Value,
    ) -> Result<()> {
        let mut req = client
            .post(url)
            .header("Content-Type", "application/json")
            .json(body);

        if let Some(a) = auth {
            if let Some(hv) = a.header_value_for(Some(url)) {
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
    auth: Option<McpHttpAuth>,
    request_timeout_ms: u64,
) -> Result<McpTransport> {
    // Try StreamableHttp first by sending a probe POST.
    let transport = McpTransport::streamable_http(base_url, auth.clone(), request_timeout_ms);
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
    let (notification_tx, _notification_rx) = mpsc::channel(64);
    McpTransport::sse(base_url, auth, request_timeout_ms, notification_tx).await
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
    fn test_parse_unsolicited_elicitation_request() {
        let params = serde_json::json!({"message": "Approve?", "mode": "once"});
        let msg = serde_json::json!({
            "jsonrpc": "2.0",
            "id": 41,
            "method": "elicitation/create",
            "params": params
        });
        assert_eq!(
            McpNotification::parse(&msg),
            Some(McpNotification::ElicitationCreate { id: 41, params })
        );
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
        let transport =
            McpTransport::streamable_http("http://localhost:8080/mcp", Some(auth.into()), 30_000);
        match &transport {
            McpTransport::StreamableHttp { base_url, .. } => {
                assert_eq!(base_url, "http://localhost:8080/mcp");
            }
            _ => panic!("Expected StreamableHttp variant"),
        }
    }

    #[test]
    fn test_streamable_http_no_auth() {
        let transport = McpTransport::streamable_http("http://localhost:3000", None, 30_000);
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

    // -- Retry / timeout helpers -------------------------------------------

    #[test]
    fn test_is_retryable_5xx() {
        assert!(McpTransport::is_retryable(&anyhow::anyhow!(
            "HTTP request failed: 500 Internal Server Error"
        )));
        assert!(McpTransport::is_retryable(&anyhow::anyhow!(
            "HTTP request failed: 502 Bad Gateway"
        )));
        assert!(McpTransport::is_retryable(&anyhow::anyhow!(
            "HTTP request failed: 503 Service Unavailable"
        )));
        assert!(McpTransport::is_retryable(&anyhow::anyhow!(
            "HTTP request failed: 504 Gateway Timeout"
        )));
    }

    #[test]
    fn test_is_retryable_429() {
        assert!(McpTransport::is_retryable(&anyhow::anyhow!(
            "HTTP request failed: 429 Too Many Requests"
        )));
    }

    #[test]
    fn test_is_retryable_connection_errors() {
        assert!(McpTransport::is_retryable(&anyhow::anyhow!(
            "connection refused"
        )));
        assert!(McpTransport::is_retryable(&anyhow::anyhow!(
            "connection reset"
        )));
        assert!(McpTransport::is_retryable(&anyhow::anyhow!(
            "tcp connect error"
        )));
        assert!(McpTransport::is_retryable(&anyhow::anyhow!(
            "error trying to connect"
        )));
    }

    #[test]
    fn test_is_retryable_4xx_not_retryable() {
        assert!(!McpTransport::is_retryable(&anyhow::anyhow!(
            "HTTP request failed: 400 Bad Request"
        )));
        assert!(!McpTransport::is_retryable(&anyhow::anyhow!(
            "HTTP request failed: 401 Unauthorized"
        )));
        assert!(!McpTransport::is_retryable(&anyhow::anyhow!(
            "HTTP request failed: 403 Forbidden"
        )));
        assert!(!McpTransport::is_retryable(&anyhow::anyhow!(
            "HTTP request failed: 404 Not Found"
        )));
    }

    #[test]
    fn test_is_retryable_timeout() {
        assert!(McpTransport::is_retryable(&anyhow::anyhow!(
            "MCP HTTP request timed out after 500ms"
        )));
    }

    #[test]
    fn test_default_timeout_is_30_seconds() {
        assert_eq!(
            crate::tools::mcp::config::default_request_timeout_ms(),
            30_000
        );
    }
}
