//! ExecdClient — daemon-side adapter for execd JSON-RPC transport.
//!
//! Spawns the execd binary as a child process and communicates
//! via newline-delimited JSON-RPC over private stdin/stdout pipes.

use std::path::PathBuf;
use std::process::Stdio;
use std::time::Duration;

use anyhow::{anyhow, bail, Context, Result};
use serde::Deserialize;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader, BufWriter, Lines};
use tokio::process::{Child, ChildStdin, ChildStdout, Command};

const PROTOCOL_VERSION: u64 = 1;

/// Configuration for spawning and communicating with the execd process.
#[allow(dead_code)]
#[derive(Clone)]
pub(crate) struct ExecdConfig {
    pub binary_path: String,
    pub shared_secret: String,
    pub startup_timeout: Duration,
    pub request_timeout: Duration,
    pub workspace_roots: Vec<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub(crate) struct ProcessReadChunk {
    pub data: String,
    pub stream: String,
    pub eof: bool,
    #[serde(default)]
    pub exit_code: Option<i32>,
}

#[derive(Debug, Deserialize)]
struct ProcessHandle {
    handle_id: String,
}

#[derive(Debug, Deserialize)]
struct ApplyPatchResponse {
    exit_code: i32,
    stdout: String,
    stderr: String,
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

/// Wraps a spawned execd child process and serializes requests over its
/// single request/response stream. Callers must not expose these pipes.
#[allow(dead_code)]
pub(crate) struct ExecdClient {
    child: Child,
    stdin: BufWriter<ChildStdin>,
    stdout: Lines<BufReader<ChildStdout>>,
    next_id: u64,
    request_timeout: Duration,
}

#[allow(dead_code)]
impl ExecdClient {
    /// Spawn the execd and perform the exact-secret handshake.
    pub async fn spawn(config: ExecdConfig) -> Result<Self> {
        if config.binary_path.is_empty() {
            bail!("execd binary path must not be empty");
        }
        if config.shared_secret.is_empty() {
            bail!("execd shared secret must not be empty");
        }
        if config.workspace_roots.is_empty() {
            bail!("execd requires at least one workspace root");
        }
        let workspace_roots = serde_json::to_string(&config.workspace_roots)
            .context("encode execd workspace roots")?;
        let mut child = Command::new(&config.binary_path)
            .env("ALETHEON_EXECD_SECRET", &config.shared_secret)
            .env("ALETHEON_EXECD_WORKSPACE_ROOTS", workspace_roots)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .kill_on_drop(true)
            .spawn()
            .with_context(|| format!("spawn execd at {}", config.binary_path))?;
        let stdin = child.stdin.take().context("execd stdin unavailable")?;
        let stdout = child.stdout.take().context("execd stdout unavailable")?;
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
            .context("execd handshake failed")?;
        if handshake
            .get("protocol_version")
            .and_then(serde_json::Value::as_u64)
            != Some(PROTOCOL_VERSION)
        {
            let _ = client.child.kill().await;
            bail!("execd protocol version mismatch");
        }
        Ok(client)
    }

    pub async fn ping(&mut self) -> Result<()> {
        let response = self.request("ping", serde_json::json!({})).await?;
        if response.get("status").and_then(serde_json::Value::as_str) != Some("ok") {
            bail!("execd ping returned an invalid response");
        }
        Ok(())
    }

    pub async fn process_read(&mut self, handle_id: &str) -> Result<Vec<ProcessReadChunk>> {
        if handle_id.is_empty() {
            bail!("execd process handle must not be empty");
        }
        let response = self
            .request("process/read", serde_json::json!({"handle_id": handle_id}))
            .await?;
        serde_json::from_value(response).context("decode execd process/read response")
    }

    pub async fn process_start(
        &mut self,
        command: &str,
        args: &[String],
        working_dir: &std::path::Path,
        env: &std::collections::BTreeMap<String, String>,
        timeout: Duration,
        policy: Option<&fabric::ResolvedSandboxPolicy>,
    ) -> Result<String> {
        let result = self
            .request(
                "process/start",
                serde_json::json!({
                    "command": command,
                    "args": args,
                    "working_dir": working_dir,
                    "env": env,
                    "sandbox_policy": policy.map(|policy| serde_json::json!({
                        "name": policy.name,
                        "read_only_roots": policy.read_only_roots,
                        "read_write_roots": policy.read_write_roots,
                        "deny_exact": policy.deny_exact,
                        "deny_globs": policy.deny_globs,
                        "restrict_network": policy.restrict_network,
                    })),
                    // The caller enforces the exact wall-clock deadline. Keep
                    // the server-side watchdog one second behind it so it is
                    // an orphan-safety backstop rather than racing the client
                    // and turning a normal timeout into PROCESS_NOT_FOUND.
                    "timeout_secs": timeout.as_secs().saturating_add(1).max(1),
                }),
            )
            .await?;
        Ok(serde_json::from_value::<ProcessHandle>(result)?.handle_id)
    }

    pub async fn process_kill(&mut self, handle_id: &str) -> Result<()> {
        if handle_id.is_empty() {
            bail!("execd process handle must not be empty");
        }
        let response = self
            .request("process/kill", serde_json::json!({"handle_id": handle_id}))
            .await?;
        if response.get("status").and_then(serde_json::Value::as_str) != Some("terminated") {
            bail!("execd process/kill returned an invalid response");
        }
        Ok(())
    }

    pub async fn process_write(&mut self, handle_id: &str, data: &str) -> Result<()> {
        self.request(
            "process/write",
            serde_json::json!({"handle_id": handle_id, "data": data}),
        )
        .await?;
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
            bail!("execd method must not be empty");
        }
        let id = self.next_id;
        self.next_id = self
            .next_id
            .checked_add(1)
            .context("execd request ID overflow")?;
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
                .ok_or_else(|| anyhow!("execd closed its response stream"))?;
            decode_response(id, &line)
        };
        tokio::time::timeout(timeout, exchange)
            .await
            .map_err(|_| anyhow!("execd request '{method}' timed out"))?
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
                bail!("execd exited unsuccessfully: {status}")
            }
            Ok(Err(error)) => {
                let _ = self.child.kill().await;
                let _ = self.child.wait().await;
                request_result?;
                Err(error).context("wait for execd shutdown")
            }
            Err(_) => {
                let _ = self.child.kill().await;
                let _ = self.child.wait().await;
                request_result?;
                bail!("execd shutdown timed out")
            }
        }
    }
}

/// Sandbox backend backed by a reconnecting execd client. A failed
/// health preflight reconnects once before process/start; failures after start
/// never replay the command, preventing duplicate side effects.
pub(crate) struct ExecdSandboxBackend {
    config: ExecdConfig,
    client: std::sync::Arc<tokio::sync::Mutex<Option<ExecdClient>>>,
}

impl Clone for ExecdSandboxBackend {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            client: self.client.clone(),
        }
    }
}

impl ExecdSandboxBackend {
    pub fn new(config: ExecdConfig) -> Self {
        Self {
            config,
            client: std::sync::Arc::new(tokio::sync::Mutex::new(None)),
        }
    }

    async fn ensure_connected(
        &self,
        guard: &mut tokio::sync::MutexGuard<'_, Option<ExecdClient>>,
    ) -> Result<()> {
        let healthy = match guard.as_mut() {
            Some(client) => client.ping().await.is_ok(),
            None => false,
        };
        if !healthy {
            **guard = Some(ExecdClient::spawn(self.config.clone()).await?);
        }
        Ok(())
    }

    async fn execute_inner(
        &self,
        cmd: &str,
        config: &fabric::SandboxConfig,
        timeout: Duration,
        sink: Option<&fabric::ToolEventSink>,
    ) -> Result<fabric::SandboxResult> {
        self.execute_process(
            "/bin/bash",
            &["-c".into(), cmd.into()],
            config,
            timeout,
            sink,
            &std::collections::BTreeMap::new(),
        )
        .await
    }

    async fn execute_process(
        &self,
        command: &str,
        args: &[String],
        config: &fabric::SandboxConfig,
        timeout: Duration,
        sink: Option<&fabric::ToolEventSink>,
        extra_env: &std::collections::BTreeMap<String, String>,
    ) -> Result<fabric::SandboxResult> {
        let mut guard = self.client.lock().await;
        self.ensure_connected(&mut guard).await?;
        let client = guard.as_mut().expect("client installed");
        let started = std::time::Instant::now();
        let mut environment = config.environment.clone();
        environment.extend(extra_env.clone());
        let handle = client
            .process_start(
                command,
                args,
                config.working_dir(),
                &environment,
                timeout,
                config.policy.as_ref(),
            )
            .await?;
        let mut stdout = String::new();
        let mut stderr = String::new();
        let read_result = tokio::time::timeout(timeout, async {
            loop {
                let chunks = client.process_read(&handle).await?;
                let mut terminal = None;
                for chunk in chunks {
                    if !chunk.data.is_empty() {
                        match chunk.stream.as_str() {
                            "stdout" => stdout.push_str(&chunk.data),
                            "stderr" => stderr.push_str(&chunk.data),
                            _ => anyhow::bail!("execd returned unknown output stream"),
                        }
                        if let Some(sink) = sink {
                            for line in chunk.data.lines() {
                                let _ = sink.progress(fabric::ToolProgress::Text(line.to_owned()));
                            }
                        }
                    }
                    terminal = terminal.or(chunk.exit_code);
                }
                if let Some(code) = terminal {
                    break Ok(code);
                }
                tokio::time::sleep(Duration::from_millis(10)).await;
            }
        })
        .await;
        let exit_code = match read_result {
            Ok(Ok(code)) => {
                client
                    .process_kill(&handle)
                    .await
                    .context("release completed execd process handle")?;
                code
            }
            Ok(Err(error)) => {
                // Do not leak a live handle when output decoding or transport
                // fails after process/start. The original failure remains the
                // authoritative result; cleanup is best-effort.
                let _ = client.process_kill(&handle).await;
                return Err(error);
            }
            Err(_) => {
                client
                    .process_kill(&handle)
                    .await
                    .context("terminate timed-out execd process")?;
                -1
            }
        };
        Ok(fabric::SandboxResult {
            stdout,
            stderr,
            exit_code,
            backend_used: "execd".into(),
            isolation_level: fabric::IsolationLevel::Process,
            elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        })
    }
}

#[derive(Debug, PartialEq, Eq)]
struct TrustedToolInvocation {
    command: std::path::PathBuf,
    args: Vec<String>,
    environment: std::collections::BTreeMap<String, String>,
}

/// Resolve a host-authored descriptor into a fixed executable identity.
///
/// Model input may populate narrowly validated data arguments, but it can
/// never select the executable or introduce shell syntax. Destructive kernel
/// installation/module loading deliberately remains unavailable until a
/// separate privileged authority exists.
fn trusted_tool_invocation(
    descriptor: &fabric::tool::ToolExecutionDescriptor,
    input: &serde_json::Value,
    session_id: &str,
) -> std::result::Result<TrustedToolInvocation, String> {
    fn required_string<'a>(
        input: &'a serde_json::Value,
        field: &str,
    ) -> std::result::Result<&'a str, String> {
        input
            .get(field)
            .and_then(serde_json::Value::as_str)
            .filter(|value| !value.is_empty() && !value.contains(['\0', '\n', '\r']))
            .ok_or_else(|| format!("trusted descriptor requires valid string field '{field}'"))
    }

    match descriptor {
        fabric::tool::ToolExecutionDescriptor::EbpfCompile => {
            let source = required_string(input, "source_path")?;
            if source.starts_with('-') {
                return Err("eBPF source path cannot be interpreted as an option".into());
            }
            let output = input
                .get("output_path")
                .and_then(serde_json::Value::as_str)
                .filter(|value| !value.is_empty() && !value.contains(['\0', '\n', '\r']))
                .map(ToOwned::to_owned)
                .unwrap_or_else(|| {
                    std::path::Path::new(source)
                        .with_extension("o")
                        .to_string_lossy()
                        .into_owned()
                });
            if output.starts_with('-') {
                return Err("eBPF output path cannot be interpreted as an option".into());
            }
            let target = input
                .get("target_arch")
                .and_then(serde_json::Value::as_str)
                .unwrap_or("bpf");
            if !matches!(target, "bpf" | "x86" | "arm64") {
                return Err("unsupported eBPF target architecture".into());
            }
            Ok(TrustedToolInvocation {
                command: "/usr/bin/clang".into(),
                args: vec![
                    "-target".into(),
                    target.into(),
                    "-O2".into(),
                    "-g".into(),
                    "-c".into(),
                    source.into(),
                    "-o".into(),
                    output,
                ],
                environment: Default::default(),
            })
        }
        fabric::tool::ToolExecutionDescriptor::ModuleBuild => {
            let source = required_string(input, "source_dir")?;
            let version = required_string(input, "kernel_version")?;
            if !version
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '.' | '_' | '+' | '-'))
            {
                return Err("invalid kernel_version for module build".into());
            }
            Ok(TrustedToolInvocation {
                command: "/usr/bin/make".into(),
                args: vec![
                    "-C".into(),
                    format!("/lib/modules/{version}/build"),
                    format!("M={source}"),
                    "modules".into(),
                ],
                environment: Default::default(),
            })
        }
        fabric::tool::ToolExecutionDescriptor::Script { canonical_path } => {
            if !canonical_path.is_absolute() {
                return Err("trusted script descriptor path must be absolute".into());
            }
            let canonical = std::fs::canonicalize(canonical_path)
                .map_err(|_| "trusted script descriptor path is no longer resolvable".to_string())?;
            if canonical != *canonical_path {
                return Err("trusted script descriptor path changed after registration".into());
            }
            let encoded = serde_json::to_string(input)
                .map_err(|_| "failed to encode trusted script input".to_string())?;
            Ok(TrustedToolInvocation {
                command: canonical,
                args: Vec::new(),
                environment: std::collections::BTreeMap::from([
                    ("ALETHEON_SESSION_ID".into(), session_id.into()),
                    ("ALETHEON_TOOL_INPUT".into(), encoded),
                ]),
            })
        }
        fabric::tool::ToolExecutionDescriptor::KernelBuild => Err(
            "kernel build/install descriptor requires an unavailable privileged execution authority"
                .into(),
        ),
        fabric::tool::ToolExecutionDescriptor::ModuleLoad => Err(
            "kernel module load/unload descriptor requires an unavailable privileged execution authority"
                .into(),
        ),
    }
}

#[async_trait::async_trait]
impl corpus::security::StructuredToolSandbox for ExecdSandboxBackend {
    fn backend_name(&self) -> &'static str {
        "execd"
    }

    fn supports_tool(&self, tool_name: &str) -> bool {
        matches!(
            tool_name,
            "file_write"
                | "apply_patch"
                | "ebpf_compile"
                | "kernel_build"
                | "module_build"
                | "module_load"
                | "script_tool"
        )
    }

    async fn execute(
        &self,
        tool_name: &str,
        descriptor: Option<&fabric::tool::ToolExecutionDescriptor>,
        input: serde_json::Value,
        context: &fabric::ToolContext,
        sandbox: &fabric::SandboxConfig,
    ) -> std::result::Result<fabric::ToolResult, String> {
        if let Some(descriptor) = descriptor {
            let descriptor_matches_tool = matches!(
                (tool_name, descriptor),
                (
                    "ebpf_compile",
                    fabric::tool::ToolExecutionDescriptor::EbpfCompile
                ) | (
                    "kernel_build",
                    fabric::tool::ToolExecutionDescriptor::KernelBuild
                ) | (
                    "module_build",
                    fabric::tool::ToolExecutionDescriptor::ModuleBuild
                ) | (
                    "module_load",
                    fabric::tool::ToolExecutionDescriptor::ModuleLoad
                ) | (_, fabric::tool::ToolExecutionDescriptor::Script { .. })
            );
            if !descriptor_matches_tool {
                return Err(format!(
                    "trusted execution descriptor does not match registered tool '{tool_name}'"
                ));
            }
            let invocation = trusted_tool_invocation(descriptor, &input, &context.session_id)?;
            let result = self
                .execute_process(
                    invocation
                        .command
                        .to_str()
                        .ok_or_else(|| "trusted executable path is not UTF-8".to_string())?,
                    &invocation.args,
                    sandbox,
                    Duration::from_secs(300),
                    None,
                    &invocation.environment,
                )
                .await
                .map_err(|error| error.to_string())?;
            return Ok(fabric::ToolResult {
                content: format!("{}\n{}", result.stdout, result.stderr)
                    .trim()
                    .to_string(),
                is_error: result.exit_code != 0,
                metadata: fabric::ToolResultMeta {
                    execution_time_ms: result.elapsed_ms,
                    truncated: false,
                    patch_delta: None,
                },
            });
        }
        if tool_name == "apply_patch" {
            if input
                .get("base_dir")
                .and_then(serde_json::Value::as_str)
                .is_some()
            {
                return Err("execd apply_patch does not yet support base_dir override".into());
            }
            let patch = input
                .get("patch")
                .and_then(serde_json::Value::as_str)
                .ok_or_else(|| "execd apply_patch requires textual patch input".to_string())?;
            let started = context.clock.mono_now();
            let mut guard = self.client.lock().await;
            self.ensure_connected(&mut guard)
                .await
                .map_err(|error| error.to_string())?;
            let response = guard
                .as_mut()
                .expect("client installed")
                .request(
                    "fs/applyPatch",
                    serde_json::json!({
                        "patch": patch,
                        "working_dir": sandbox.working_dir(),
                        "deny_exact": sandbox
                            .policy
                            .as_ref()
                            .map(|policy| policy.deny_exact.as_slice())
                            .unwrap_or(&[]),
                        "write_roots": sandbox
                            .policy
                            .as_ref()
                            .map(|policy| policy.read_write_roots.as_slice()),
                    }),
                )
                .await
                .map_err(|error| error.to_string())?;
            let result: ApplyPatchResponse =
                serde_json::from_value(response).map_err(|error| error.to_string())?;
            return Ok(fabric::ToolResult {
                content: format!("{}\n{}", result.stdout, result.stderr)
                    .trim()
                    .to_string(),
                is_error: result.exit_code != 0,
                metadata: fabric::ToolResultMeta {
                    execution_time_ms: context.clock.mono_now().0.saturating_sub(started.0),
                    truncated: false,
                    patch_delta: None,
                },
            });
        }
        if tool_name != "file_write" {
            return Err(format!(
                "execd structured adapter does not support {tool_name}; refusing in-process fallback"
            ));
        }
        let path = input
            .get("path")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "file_write path is required".to_string())?;
        let content = input
            .get("content")
            .and_then(serde_json::Value::as_str)
            .ok_or_else(|| "file_write content is required".to_string())?;
        let workspace = context
            .effective_workspace_policy()
            .map_err(|e| e.to_string())?;
        let path = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            workspace.cwd().join(path)
        };
        let started = context.clock.mono_now();
        let mut guard = self.client.lock().await;
        self.ensure_connected(&mut guard)
            .await
            .map_err(|error| error.to_string())?;
        let result = guard
            .as_mut()
            .unwrap()
            .request(
                "fs/write",
                serde_json::json!({
                    "path": path,
                    "content": content,
                    "deny_exact": sandbox
                        .policy
                        .as_ref()
                        .map(|policy| policy.deny_exact.as_slice())
                        .unwrap_or(&[]),
                    "write_roots": sandbox
                        .policy
                        .as_ref()
                        .map(|policy| policy.read_write_roots.as_slice()),
                }),
            )
            .await
            .map_err(|e| e.to_string())?;
        let bytes = result
            .get("bytes_written")
            .and_then(serde_json::Value::as_u64)
            .unwrap_or(0);
        Ok(fabric::ToolResult {
            content: format!("Wrote {bytes} bytes to {}", path.display()),
            is_error: false,
            metadata: fabric::ToolResultMeta {
                execution_time_ms: context.clock.mono_now().0.saturating_sub(started.0),
                truncated: false,
                patch_delta: None,
            },
        })
    }
}

#[async_trait::async_trait]
impl fabric::SandboxBackend for ExecdSandboxBackend {
    fn name(&self) -> &str {
        "execd"
    }
    fn isolation_level(&self) -> fabric::IsolationLevel {
        fabric::IsolationLevel::Process
    }
    fn is_available(&self) -> bool {
        true
    }
    fn capabilities(&self) -> fabric::SandboxCapabilities {
        fabric::SandboxCapabilities {
            filesystem_isolation: true,
            network_isolation: true,
            resource_limits: false,
            seccomp_filter: false,
            limitations: vec!["resolved policies require bubblewrap on the execd host".into()],
        }
    }
    async fn execute(
        &self,
        cmd: &str,
        config: &fabric::SandboxConfig,
        timeout: Duration,
    ) -> Result<fabric::SandboxResult> {
        self.execute_inner(cmd, config, timeout, None).await
    }
    async fn execute_streaming(
        &self,
        cmd: &str,
        config: &fabric::SandboxConfig,
        timeout: Duration,
        sink: &fabric::ToolEventSink,
    ) -> Result<fabric::SandboxResult> {
        self.execute_inner(cmd, config, timeout, Some(sink)).await
    }
}

fn decode_response(expected_id: u64, line: &str) -> Result<serde_json::Value> {
    let response: RpcResponse = serde_json::from_str(line).context("decode execd response")?;
    if response.jsonrpc != "2.0" {
        bail!("execd returned an invalid JSON-RPC version");
    }
    if response.id != serde_json::json!(expected_id) {
        bail!("execd response ID mismatch");
    }
    match (response.result, response.error) {
        (Some(result), None) => Ok(result),
        (None, Some(error)) => Err(anyhow!("execd RPC error {}: {}", error.code, error.message)),
        _ => bail!("execd response must contain exactly one of result or error"),
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
                r#"{"jsonrpc":"2.0","id":2,"result":[{"data":"out","stream":"stdout","eof":true,"exit_code":7}]}"#,
            )
            .unwrap(),
        )
        .unwrap();
        assert_eq!(chunks[0].data, "out");
        assert!(chunks[0].eof);
        assert_eq!(chunks[0].exit_code, Some(7));
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

    #[test]
    fn trusted_descriptor_selects_fixed_executable_and_ignores_command_injection() {
        let invocation = trusted_tool_invocation(
            &fabric::tool::ToolExecutionDescriptor::EbpfCompile,
            &serde_json::json!({
                "command": "/tmp/model-command",
                "source_path": "program.c",
                "output_path": "program.o",
                "target_arch": "bpf"
            }),
            "session",
        )
        .unwrap();

        assert_eq!(invocation.command, std::path::Path::new("/usr/bin/clang"));
        assert!(!invocation
            .args
            .iter()
            .any(|arg| arg == "/tmp/model-command"));
        assert_eq!(
            invocation.args,
            [
                "-target",
                "bpf",
                "-O2",
                "-g",
                "-c",
                "program.c",
                "-o",
                "program.o"
            ]
        );
    }

    #[test]
    fn privileged_descriptors_fail_closed_without_authority() {
        for descriptor in [
            fabric::tool::ToolExecutionDescriptor::KernelBuild,
            fabric::tool::ToolExecutionDescriptor::ModuleLoad,
        ] {
            let error = trusted_tool_invocation(&descriptor, &serde_json::json!({}), "session")
                .unwrap_err();
            assert!(error.contains("privileged execution authority"));
        }
    }

    #[test]
    fn module_build_rejects_kernel_version_argument_injection() {
        let error = trusted_tool_invocation(
            &fabric::tool::ToolExecutionDescriptor::ModuleBuild,
            &serde_json::json!({
                "source_dir": "/workspace/module",
                "kernel_version": "6.9/../../evil"
            }),
            "session",
        )
        .unwrap_err();
        assert!(error.contains("invalid kernel_version"));
    }

    #[test]
    fn script_descriptor_rechecks_canonical_host_path() {
        let dir = tempfile::tempdir().unwrap();
        let script = dir.path().join("trusted-script");
        std::fs::write(&script, "#!/bin/sh\n").unwrap();
        let canonical = std::fs::canonicalize(&script).unwrap();
        let invocation = trusted_tool_invocation(
            &fabric::tool::ToolExecutionDescriptor::Script {
                canonical_path: canonical.clone(),
            },
            &serde_json::json!({"command": "/tmp/model-command"}),
            "trusted-session",
        )
        .unwrap();
        assert_eq!(invocation.command, canonical);
        assert_eq!(
            invocation.environment.get("ALETHEON_SESSION_ID"),
            Some(&"trusted-session".to_string())
        );
        assert!(invocation
            .environment
            .get("ALETHEON_TOOL_INPUT")
            .unwrap()
            .contains("model-command"));
    }
}
