//! ExecServerClient — daemon-side adapter for exec-server JSON-RPC transport.
//!
//! Spawns the exec-server binary as a child process and communicates
//! via newline-delimited JSON-RPC over private stdin/stdout pipes.

use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const PROTOCOL_VERSION: u64 = 1;

/// Configuration for spawning and communicating with the exec-server process.
#[allow(dead_code)]
pub(crate) struct ExecServerConfig {
    pub binary_path: String,
    pub shared_secret: String,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct ProcessReadChunk {
    pub data: String,
    pub stream: String,
    pub eof: bool,
}

#[derive(Debug, Deserialize)]
struct RpcResponse {
    jsonrpc: String,
    id: serde_json::Value,
    #[serde(default)]
    result: Option<serde_json::Value>,
    #[serde(default)]
    error: Option<RpcError>,
}

#[derive(Debug, Deserialize)]
struct RpcError {
    code: i64,
    message: String,
    #[allow(dead_code)]
    data: Option<serde_json::Value>,
}

/// Wraps a spawned exec-server child process and serializes requests over its
/// single request/response stream. Callers must not expose these pipes.
#[allow(dead_code)]
pub(crate) struct ExecServerClient {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    request_timeout: Duration,
}

#[allow(dead_code)]
impl ExecServerClient {
    /// Spawn the exec-server and perform the exact-secret handshake.
    pub async fn spawn(config: ExecServerConfig) -> Result<Self> {
        if config.shared_secret.is_empty() {
            bail!("exec-server shared secret must not be empty");
        }
        let mut child = Command::new(&config.binary_path)
            .env("ALETHEON_EXEC_SERVER_SECRET", &config.shared_secret)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawn exec-server at {}", config.binary_path))?;
        let stdin = child
            .stdin
            .take()
            .context("exec-server stdin unavailable")?;
        let stdout = child
            .stdout
            .take()
            .context("exec-server stdout unavailable")?;
        let mut client = Self {
            child,
            stdin: BufWriter::new(stdin),
            stdout: BufReader::new(stdout).lines(),
            next_id: 1,
            request_timeout: config.request_timeout,
        };
        let handshake = client
            .request_with_timeout(
                "handshake",
                serde_json::json!({"secret": config.shared_secret}),
                config.startup_timeout,
            )
            .await
            .context("exec-server handshake failed")?;
        if handshake
            .get("protocol_version")
            .and_then(serde_json::Value::as_u64)
            != Some(PROTOCOL_VERSION)
        {
            let _ = client.child.kill().await;
            bail!("exec-server protocol version mismatch");
        }
        Ok(client)
    }

    pub async fn ping(&mut self) -> Result<()> {
        let response = self.request("ping", serde_json::json!({})).await?;
        if response.get("status").and_then(serde_json::Value::as_str) != Some("ok") {
            bail!("exec-server ping returned an invalid response");
        }
        Ok(())
    }

    pub async fn process_read(&mut self, handle_id: &str) -> Result<Vec<ProcessReadChunk>> {
        if handle_id.is_empty() {
            bail!("exec-server process handle must not be empty");
        }
        let response = self
            .request("process/read", serde_json::json!({"handle_id": handle_id}))
            .await?;
        serde_json::from_value(response).context("decode exec-server process/read response")
    }

    pub async fn process_kill(&mut self, handle_id: &str) -> Result<()> {
        if handle_id.is_empty() {
            bail!("exec-server process handle must not be empty");
        }
        let response = self
            .request("process/kill", serde_json::json!({"handle_id": handle_id}))
            .await?;
        if response.get("status").and_then(serde_json::Value::as_str) != Some("terminated") {
            bail!("exec-server process/kill returned an invalid response");
        }
        Ok(())
    }

    /// Send one JSON-RPC request and await its matching response.
    pub async fn request(
        &mut self,
        method: &str,
        params: serde_json::Value,
    ) -> Result<serde_json::Value> {
        self.request_with_timeout(method, params, self.request_timeout)
            .await
    }

    async fn request_with_timeout(
        &mut self,
        method: &str,
        params: serde_json::Value,
        timeout: Duration,
    ) -> Result<serde_json::Value> {
        if method.is_empty() {
            bail!("exec-server method must not be empty");
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .context("exec-server request ID overflow")?;
        let request = serde_json::json!({
            "jsonrpc": "2.0",
            "id": id,
            "method": method,
            "params": params,
        });
        let encoded = serde_json::to_vec(&request)?;
        let exchange = async {
            self.stdin.write_all(&encoded).await?;
            self.stdin.write_all(b"\n").await?;
            self.stdin.flush().await?;
            let line = self
                .stdout
                .next_line()
                .await?
                .ok_or_else(|| anyhow!("exec-server closed its response stream"))?;
            decode_response(id, &line)
        };
        tokio::time::timeout(timeout, exchange)
            .await
            .map_err(|_| anyhow!("exec-server request '{method}' timed out"))?
    }

    /// Graceful shutdown, bounded by request_timeout, with forced child kill on
    /// protocol or exit timeout.
    pub async fn shutdown(&mut self) -> Result<()> {
        let request_result = self.request("shutdown", serde_json::json!({})).await;
        let wait_result = tokio::time::timeout(self.request_timeout, self.child.wait()).await;
        match wait_result {
            Ok(Ok(status)) if status.success() => {
                request_result?;
                Ok(())
            }
            Ok(Ok(status)) => {
                let _ = self.child.kill().await;
                let _ = self.child.wait().await;
                request_result?;
                bail!("exec-server exited unsuccessfully: {status}")
            }
            Ok(Err(error)) => {
                let _ = self.child.kill().await;
                let _ = self.child.wait().await;
                request_result?;
                Err(error).context("wait for exec-server shutdown")
            }
            Err(_) => {
                let _ = self.child.kill().await;
                let _ = self.child.wait().await;
                request_result?;
                bail!("exec-server shutdown timed out")
            }
        }
    }
}

fn decode_response(expected_id: u64, line: &str) -> Result<serde_json::Value> {
    let response: RpcResponse =
        serde_json::from_str(line).context("decode exec-server response")?;
    if response.jsonrpc != "2.0" {
        bail!("exec-server returned an invalid JSON-RPC version");
    }
    if response.id != serde_json::json!(expected_id) {
        bail!("exec-server response ID mismatch");
    }
    match (response.result, response.error) {
        (Some(result), None) => Ok(result),
        (None, Some(error)) => Err(anyhow!(
            "exec-server RPC error {}: {}",
            error.code,
            error.message
        )),
        _ => bail!("exec-server response must contain exactly one of result or error"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_ping_process_read_and_process_kill_shapes() {
        assert_eq!(
            decode_response(1, r#"{"jsonrpc":"2.0","id":1,"result":{"status":"ok"}}"#).unwrap()
                ["status"],
            "ok"
        );
        let chunks: Vec<ProcessReadChunk> = serde_json::from_value(
            decode_response(
                2,
                r#"{"jsonrpc":"2.0","id":2,"result":[{"data":"out","stream":"stdout","eof":true}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(chunks[0].data, "out");
        assert!(chunks[0].eof);
        assert_eq!(
            decode_response(
                3,
                r#"{"jsonrpc":"2.0","id":3,"result":{"status":"terminated"}}"#,
            )
            .unwrap()["status"],
            "terminated"
        );
    }

    #[test]
    fn rejects_rpc_errors_and_mismatched_ids() {
        let error = decode_response(
            4,
            r#"{"jsonrpc":"2.0","id":4,"error":{"code":-32005,"message":"denied"}}"#,
        )
        .unwrap_err();
        assert!(error.to_string().contains("-32005"));
        assert!(
            decode_response(5, r#"{"jsonrpc":"2.0","id":6,"result":{"status":"ok"}}"#,).is_err()
        );
    }
}
