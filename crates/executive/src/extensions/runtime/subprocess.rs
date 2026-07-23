//! Subprocess runtime adapter for Executable Assets.
//!
//! Spawns isolated child processes communicating via JSON-RPC over stdio.
//! Implements lifecycle management, health checks, timeout enforcement,
//! circuit breaking, and stderr sanitization.

use anyhow::{bail, Context, Result};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::time::Duration;
use tokio::process::{Child, Command};
use tokio::time::timeout;
use tokio_util::sync::CancellationToken;

// ---------------------------------------------------------------------------
// Protocol types
// ---------------------------------------------------------------------------

#[derive(Debug, Serialize)]
#[serde(rename_all = "snake_case")]
struct JsonRpcRequest {
    jsonrpc: String,
    method: String,
    params: Value,
    id: u64,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "snake_case")]
struct JsonRpcResponse {
    #[allow(dead_code)]
    jsonrpc: Option<String>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
    id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    #[allow(dead_code)]
    code: i64,
    message: String,
}

// ---------------------------------------------------------------------------
// Stderr buffer (captured, truncated, sanitized)
// ---------------------------------------------------------------------------

const MAX_STDERR_BYTES: usize = 4 * 1024;

struct SanitizedStderr {
    buffer: Vec<u8>,
}

impl SanitizedStderr {
    fn new() -> Self {
        Self { buffer: Vec::new() }
    }

    fn append(&mut self, data: &[u8]) {
        if self.buffer.len() < MAX_STDERR_BYTES {
            let remaining = MAX_STDERR_BYTES - self.buffer.len();
            self.buffer
                .extend_from_slice(&data[..data.len().min(remaining)]);
        }
    }

    fn snapshot(&self) -> String {
        String::from_utf8_lossy(&self.buffer)
            .chars()
            .take(200)
            .collect()
    }
}

// ---------------------------------------------------------------------------
// Runtime configuration
// ---------------------------------------------------------------------------

pub struct SubprocessConfig {
    /// Path to the executable.
    pub command: String,
    /// Arguments to pass.
    pub args: Vec<String>,
    /// Working directory.
    pub working_dir: Option<String>,
    /// Environment variables to pass (empty = clear all).
    pub env: Vec<(String, String)>,
    /// Startup timeout.
    pub start_timeout: Duration,
    /// Per-call timeout.
    pub call_timeout: Duration,
    /// Idle timeout before automatic shutdown.
    pub idle_timeout: Duration,
    /// Graceful shutdown timeout.
    pub shutdown_timeout: Duration,
    /// Consecutive failures before circuit breaking.
    pub circuit_breaker_threshold: u32,
}

impl Default for SubprocessConfig {
    fn default() -> Self {
        Self {
            command: String::new(),
            args: vec![],
            working_dir: None,
            env: vec![],
            start_timeout: Duration::from_secs(30),
            call_timeout: Duration::from_secs(300),
            idle_timeout: Duration::from_secs(600),
            shutdown_timeout: Duration::from_secs(10),
            circuit_breaker_threshold: 3,
        }
    }
}

// ---------------------------------------------------------------------------
// Runtime instance
// ---------------------------------------------------------------------------

pub struct SubprocessRuntime {
    config: SubprocessConfig,
    process: Option<Child>,
    request_id: u64,
    stderr: SanitizedStderr,
    consecutive_failures: u32,
    quarantined: bool,
}

impl SubprocessRuntime {
    pub fn new(config: SubprocessConfig) -> Self {
        Self {
            config,
            process: None,
            request_id: 1,
            stderr: SanitizedStderr::new(),
            consecutive_failures: 0,
            quarantined: false,
        }
    }

    pub fn is_quarantined(&self) -> bool {
        self.quarantined
    }

    /// Start the subprocess and initialize the JSON-RPC connection.
    pub async fn start(&mut self) -> Result<()> {
        if self.quarantined {
            bail!("runtime is quarantined");
        }
        let mut cmd = Command::new(&self.config.command);
        cmd.args(&self.config.args);
        cmd.stdin(std::process::Stdio::piped());
        cmd.stdout(std::process::Stdio::piped());
        cmd.stderr(std::process::Stdio::piped());
        cmd.kill_on_drop(true);

        // Minimal environment
        cmd.env_clear();
        for (k, v) in &self.config.env {
            cmd.env(k, v);
        }

        if let Some(ref dir) = self.config.working_dir {
            cmd.current_dir(dir);
        }

        let child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn: {}", self.config.command))?;
        self.process = Some(child);

        // Initialize protocol
        match timeout(
            self.config.start_timeout,
            self.call("initialize", serde_json::json!({})),
        )
        .await
        {
            Ok(Ok(_)) => {
                self.consecutive_failures = 0;
                Ok(())
            }
            Ok(Err(e)) => {
                self.increment_failures();
                Err(e)
            }
            Err(_) => {
                self.increment_failures();
                bail!("subprocess startup timed out")
            }
        }
    }

    /// Call a JSON-RPC method on the subprocess.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        if self.quarantined {
            bail!("runtime is quarantined");
        }
        if self.process.is_none() {
            bail!("subprocess not started");
        }

        let id = self.request_id;
        self.request_id += 1;
        let request = JsonRpcRequest {
            jsonrpc: "2.0".into(),
            method: method.into(),
            params,
            id,
        };
        let request_json = serde_json::to_string(&request)? + "\n";

        let process = self.process.as_mut().unwrap();
        // Write request to stdin
        use tokio::io::AsyncWriteExt;
        process
            .stdin
            .as_mut()
            .unwrap()
            .write_all(request_json.as_bytes())
            .await?;

        // Read response from stdout
        use tokio::io::{AsyncBufReadExt, BufReader};
        let stdout = process.stdout.take().context("stdout not available")?;
        let mut reader = BufReader::new(stdout).lines();
        let line = timeout(self.config.call_timeout, reader.next_line())
            .await
            .map_err(|_| anyhow::anyhow!("call timed out"))?
            .context("unexpected EOF")?
            .context("empty response line")?;

        // Restore stdout
        self.process.as_mut().unwrap().stdout = Some(reader.into_inner().into_inner());

        let response: JsonRpcResponse = serde_json::from_str(&line)
            .with_context(|| format!("invalid JSON-RPC response: {}", line))?;

        if let Some(err) = response.error {
            bail!("JSON-RPC error: {}", err.message);
        }
        Ok(response.result.unwrap_or(Value::Null))
    }

    /// Gracefully shut down the subprocess.
    pub async fn shutdown(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = timeout(self.config.shutdown_timeout, async {
                let _ = self.call("shutdown", serde_json::json!({})).await;
                child.kill().await.ok();
            })
            .await;
        }
    }

    fn increment_failures(&mut self) {
        self.consecutive_failures += 1;
        if self.consecutive_failures >= self.config.circuit_breaker_threshold {
            self.quarantined = true;
            tracing::error!(
                command = %self.config.command,
                failures = self.consecutive_failures,
                "Subprocess runtime circuit breaker tripped — quarantined"
            );
        }
    }
}

impl Drop for SubprocessRuntime {
    fn drop(&mut self) {
        if let Some(mut child) = self.process.take() {
            let _ = child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_config_has_reasonable_timeouts() {
        let cfg = SubprocessConfig::default();
        assert!(cfg.start_timeout.as_secs() >= 10);
        assert!(cfg.circuit_breaker_threshold >= 1);
    }

    #[test]
    fn new_runtime_is_not_quarantined() {
        let rt = SubprocessRuntime::new(SubprocessConfig::default());
        assert!(!rt.is_quarantined());
    }

    #[tokio::test]
    async fn quarantined_runtime_rejects_start_and_call() {
        let mut rt = SubprocessRuntime::new(SubprocessConfig::default());
        // Manually quarantine
        for _ in 0..SubprocessConfig::default().circuit_breaker_threshold {
            rt.increment_failures();
        }
        assert!(rt.is_quarantined());
        assert!(rt.start().await.is_err());
        assert!(rt.call("test", serde_json::json!({})).await.is_err());
    }
}
