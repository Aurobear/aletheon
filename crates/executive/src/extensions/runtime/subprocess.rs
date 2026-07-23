//! Subprocess runtime adapter for Executable Assets.
//!
//! Spawns isolated child processes communicating via JSON-RPC over stdio.
//! Implements lifecycle management, health checks, timeout enforcement,
//! circuit breaking, stderr sanitization, and cancellation.

use anyhow::{bail, Context, Result};
use fabric::include::extension_provider::AgentRuntimeProvider;
use fabric::{AgentHandle, AgentSpawnRequest, IsolationLevel, SandboxBackend, SandboxConfig};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;
use tokio::time::{timeout, Instant};
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
struct JsonRpcResponse {
    jsonrpc: Option<String>,
    #[serde(default)]
    result: Option<Value>,
    #[serde(default)]
    error: Option<JsonRpcError>,
    id: Option<u64>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

// ---------------------------------------------------------------------------
// Response line size limit (10 MB)
// ---------------------------------------------------------------------------

const MAX_RESPONSE_LINE_BYTES: usize = 10 * 1024 * 1024;

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
        let raw: String = String::from_utf8_lossy(&self.buffer)
            .chars()
            .take(200)
            .collect();
        raw.lines()
            .map(|line| {
                let lower = line.to_ascii_lowercase();
                if ["token", "secret", "api_key", "authorization"]
                    .iter()
                    .any(|marker| lower.contains(marker))
                {
                    "[REDACTED]"
                } else {
                    line
                }
            })
            .collect::<Vec<_>>()
            .join("\n")
    }
}

// ---------------------------------------------------------------------------
// Runtime configuration
// ---------------------------------------------------------------------------

#[derive(Clone)]
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
    /// Hard CPU time limit enforced before the extension command starts.
    pub cpu_time_seconds: u64,
    /// Hard virtual-memory limit in bytes.
    pub memory_bytes: u64,
    /// Hard process-count limit.
    pub max_processes: u32,
}

/// Production provider adapter. Every Agent handle owns a distinct isolated
/// protocol process; callers never access the subprocess implementation.
pub struct SubprocessAgentRuntimeProvider {
    config: SubprocessConfig,
    sandbox_backend: Arc<dyn SandboxBackend>,
    sandbox_config: SandboxConfig,
    sessions: Mutex<HashMap<fabric::AgentId, ProviderSession>>,
    circuit: Mutex<ProviderCircuit>,
}

#[derive(Clone)]
struct ProviderSession {
    process: Arc<Mutex<SubprocessRuntime>>,
    cancel: CancellationToken,
}

#[derive(Default)]
struct ProviderCircuit {
    consecutive_failures: u32,
    quarantined: bool,
}

impl SubprocessAgentRuntimeProvider {
    pub fn new(
        config: SubprocessConfig,
        sandbox_backend: Arc<dyn SandboxBackend>,
        sandbox_config: SandboxConfig,
    ) -> Result<Self> {
        // Validate the backend during composition, not after a task is admitted.
        let _ = SubprocessRuntime::new_isolated(
            config.clone(),
            sandbox_backend.as_ref(),
            &sandbox_config,
        )?;
        Ok(Self {
            config,
            sandbox_backend,
            sandbox_config,
            sessions: Mutex::new(HashMap::new()),
            circuit: Mutex::new(ProviderCircuit::default()),
        })
    }

    async fn ensure_available(&self) -> Result<()> {
        anyhow::ensure!(
            !self.circuit.lock().await.quarantined,
            "extension runtime provider is quarantined"
        );
        Ok(())
    }

    async fn record_business_result<T>(&self, result: &Result<T>) {
        let mut circuit = self.circuit.lock().await;
        if result.is_ok() {
            circuit.consecutive_failures = 0;
            return;
        }
        circuit.consecutive_failures = circuit.consecutive_failures.saturating_add(1);
        if circuit.consecutive_failures >= self.config.circuit_breaker_threshold {
            circuit.quarantined = true;
            tracing::error!(
                command = %self.config.command,
                failures = circuit.consecutive_failures,
                "Extension runtime provider circuit breaker tripped"
            );
        }
    }

    async fn session(&self, handle: &AgentHandle) -> Result<ProviderSession> {
        self.sessions
            .lock()
            .await
            .get(&handle.agent_id)
            .cloned()
            .context("unknown extension runtime Agent handle")
    }

    pub async fn probe(&self) -> Result<()> {
        let mut runtime = SubprocessRuntime::new_isolated(
            self.config.clone(),
            self.sandbox_backend.as_ref(),
            &self.sandbox_config,
        )?;
        runtime.start().await?;
        let result = runtime.call("health", serde_json::json!({})).await;
        runtime.shutdown().await;
        result.map(|_| ())
    }
}

#[async_trait::async_trait]
impl AgentRuntimeProvider for SubprocessAgentRuntimeProvider {
    async fn start(&self, request: AgentSpawnRequest) -> Result<AgentHandle> {
        self.ensure_available().await?;
        request
            .validate()
            .map_err(|error| anyhow::anyhow!(error.to_string()))?;
        let result = async {
            let mut runtime = SubprocessRuntime::new_isolated(
                self.config.clone(),
                self.sandbox_backend.as_ref(),
                &self.sandbox_config,
            )?;
            runtime.start().await?;
            let value = runtime
                .call("start", serde_json::to_value(request)?)
                .await?;
            let handle: AgentHandle = serde_json::from_value(value)
                .context("runtime returned an invalid Agent handle")?;
            Ok::<_, anyhow::Error>((handle, runtime))
        }
        .await;
        if result.is_err() {
            self.record_business_result(&result).await;
        }
        let (handle, runtime) = result?;
        let session = ProviderSession {
            cancel: runtime.cancellation_token(),
            process: Arc::new(Mutex::new(runtime)),
        };
        anyhow::ensure!(
            self.sessions
                .lock()
                .await
                .insert(handle.agent_id.clone(), session)
                .is_none(),
            "runtime returned a duplicate Agent handle"
        );
        Ok(handle)
    }

    async fn observe(&self, handle: &AgentHandle) -> Result<Value> {
        self.ensure_available().await?;
        let result = self
            .session(handle)
            .await?
            .process
            .lock()
            .await
            .call("observe", serde_json::to_value(handle)?)
            .await;
        self.record_business_result(&result).await;
        result
    }

    async fn steer(&self, handle: &AgentHandle, input: Value) -> Result<()> {
        self.ensure_available().await?;
        let result = self
            .session(handle)
            .await?
            .process
            .lock()
            .await
            .call(
                "steer",
                serde_json::json!({"handle": handle, "input": input}),
            )
            .await;
        self.record_business_result(&result).await;
        result?;
        Ok(())
    }

    async fn follow_up(&self, handle: &AgentHandle, input: Value) -> Result<Value> {
        self.ensure_available().await?;
        let result = self
            .session(handle)
            .await?
            .process
            .lock()
            .await
            .call(
                "follow_up",
                serde_json::json!({"handle": handle, "input": input}),
            )
            .await;
        self.record_business_result(&result).await;
        result
    }

    async fn cancel(&self, handle: &AgentHandle, reason: &str) -> Result<()> {
        let session = self
            .sessions
            .lock()
            .await
            .remove(&handle.agent_id)
            .context("unknown extension runtime Agent handle")?;
        session.cancel.cancel();
        let mut runtime = session.process.lock().await;
        let _ = runtime
            .call(
                "cancel",
                serde_json::json!({"handle": handle, "reason": reason}),
            )
            .await;
        runtime.cancel();
        runtime.shutdown().await;
        Ok(())
    }

    async fn wait(&self, handle: &AgentHandle) -> Result<Value> {
        let session = self.session(handle).await?;
        let result = session
            .process
            .lock()
            .await
            .call("wait", serde_json::to_value(handle)?)
            .await;
        self.record_business_result(&result).await;
        if result.is_ok() {
            self.sessions.lock().await.remove(&handle.agent_id);
        }
        result
    }

    async fn health(&self) -> Result<()> {
        self.ensure_available().await?;
        for session in self.sessions.lock().await.values() {
            session.process.lock().await.health()?;
        }
        Ok(())
    }
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
            cpu_time_seconds: 300,
            memory_bytes: 1024 * 1024 * 1024,
            max_processes: 32,
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
    stderr: Arc<Mutex<SanitizedStderr>>,
    consecutive_failures: u32,
    quarantined: bool,
    /// Token to signal cancellation — cancel fires → kill child process.
    cancel_token: CancellationToken,
    /// Timestamp of the most recent `call()` for idle-timeout tracking.
    last_activity: Instant,
    /// Guard against concurrent `call()` invocations.
    in_flight: bool,
    isolation_verified: bool,
}

impl SubprocessRuntime {
    pub fn new(config: SubprocessConfig) -> Self {
        Self {
            config,
            process: None,
            request_id: 1,
            stderr: Arc::new(Mutex::new(SanitizedStderr::new())),
            consecutive_failures: 0,
            quarantined: false,
            cancel_token: CancellationToken::new(),
            last_activity: Instant::now(),
            in_flight: false,
            isolation_verified: false,
        }
    }

    /// Build a runtime from an argv-safe, capability-complete isolation backend.
    /// Missing namespace, filesystem, network, or resource isolation fails closed.
    pub fn new_isolated(
        mut config: SubprocessConfig,
        backend: &dyn SandboxBackend,
        sandbox: &SandboxConfig,
    ) -> Result<Self> {
        anyhow::ensure!(backend.is_available(), "isolation backend is unavailable");
        anyhow::ensure!(
            backend.isolation_level() == IsolationLevel::Namespace
                || backend.isolation_level() == IsolationLevel::Container,
            "extension subprocess requires namespace or container isolation"
        );
        let capabilities = backend.capabilities();
        anyhow::ensure!(
            capabilities.filesystem_isolation
                && capabilities.network_isolation
                && capabilities.resource_limits,
            "isolation backend lacks required filesystem, network, or resource controls"
        );
        anyhow::ensure!(
            config.cpu_time_seconds > 0 && config.memory_bytes > 0 && config.max_processes > 0,
            "extension subprocess resource limits must be nonzero"
        );
        let limiter = which::which("prlimit")
            .context("prlimit is required for extension subprocess resource governance")?;
        let mut limited_args = vec![
            format!("--cpu={}", config.cpu_time_seconds),
            format!("--as={}", config.memory_bytes),
            format!("--nproc={}", config.max_processes),
            "--".into(),
            config.command.clone(),
        ];
        limited_args.extend(config.args.clone());
        let wrapped = backend.wrap_argv(&limiter, &limited_args, sandbox)?;
        config.command = wrapped.program.to_string_lossy().into_owned();
        config.args = wrapped.args;
        config.env = wrapped.environment.into_iter().collect();
        let mut runtime = Self::new(config);
        runtime.isolation_verified = true;
        Ok(runtime)
    }

    pub fn is_quarantined(&self) -> bool {
        self.quarantined
    }

    pub fn cancellation_token(&self) -> CancellationToken {
        self.cancel_token.clone()
    }

    /// Start the subprocess and initialize the JSON-RPC connection.
    pub async fn start(&mut self) -> Result<()> {
        if self.quarantined {
            bail!("runtime is quarantined");
        }
        if !self.isolation_verified {
            self.increment_failures();
            bail!("extension subprocess has no verified isolation backend");
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

        let mut child = cmd
            .spawn()
            .with_context(|| format!("failed to spawn: {}", self.config.command))?;

        // --- stderr drain ---------------------------------------------------
        // Read stderr line-by-line in a background task so the pipe buffer
        // never fills up. The drain stops when the child exits (stderr pipe
        // closes) or when the cancellation token fires.
        let stderr_pipe = child.stderr.take().context("stderr not available")?;
        let stderr_buf = Arc::clone(&self.stderr);
        let cancel = self.cancel_token.clone();
        tokio::spawn(async move {
            let mut reader = BufReader::new(stderr_pipe);
            let mut line = String::new();
            loop {
                line.clear();
                tokio::select! {
                    _ = cancel.cancelled() => {
                        break;
                    }
                    read_result = reader.read_line(&mut line) => {
                        match read_result {
                            Ok(0) => break, // EOF — process exited
                            Ok(_) => {
                                let mut buf = stderr_buf.lock().await;
                                buf.append(line.as_bytes());
                            }
                            Err(_) => break,
                        }
                    }
                }
            }
        });

        self.process = Some(child);
        self.last_activity = Instant::now();

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
    ///
    /// Enforces: no concurrent calls, idle-timeout gate, response-id match,
    /// jsonrpc version check, and a 10 MB response line cap.
    pub async fn call(&mut self, method: &str, params: Value) -> Result<Value> {
        if self.quarantined {
            bail!("runtime is quarantined");
        }
        if self.process.is_none() {
            bail!("subprocess not started");
        }
        if self.in_flight {
            bail!("a previous JSON-RPC call is still in flight");
        }

        // --- idle timeout ---------------------------------------------------
        if self.last_activity.elapsed() > self.config.idle_timeout {
            bail!(
                "subprocess idle timeout exceeded ({:?})",
                self.config.idle_timeout
            );
        }

        self.in_flight = true;
        let result = self.call_inner(method, params).await;
        self.in_flight = false;
        self.last_activity = Instant::now();
        match &result {
            Ok(_) => self.consecutive_failures = 0,
            Err(error) => {
                self.increment_failures();
                let stderr = self.stderr.lock().await.snapshot();
                tracing::warn!(
                    error = %error,
                    stderr = %stderr,
                    "Extension subprocess call failed; protocol channel will be terminated"
                );
                if let Some(mut child) = self.process.take() {
                    let _ = child.kill().await;
                }
            }
        }
        result
    }

    /// Inner call logic (called after guard checks).
    async fn call_inner(&mut self, method: &str, params: Value) -> Result<Value> {
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

        // Check cancellation before writing
        if self.cancel_token.is_cancelled() {
            bail!("runtime cancelled");
        }

        // Write request to stdin
        process
            .stdin
            .as_mut()
            .unwrap()
            .write_all(request_json.as_bytes())
            .await?;

        // Read response from stdout (one newline-delimited line)
        let stdout = process.stdout.take().context("stdout not available")?;
        let mut reader = BufReader::new(stdout);
        let mut line = Vec::new();

        // Read one bounded line without allowing `read_line` to allocate an
        // attacker-controlled amount before the size check.
        let read_result = timeout(self.config.call_timeout, async {
            tokio::select! {
                result = async {
                    loop {
                        let available = reader.fill_buf().await?;
                        if available.is_empty() {
                            return Ok::<usize, std::io::Error>(line.len());
                        }
                        let take = available
                            .iter()
                            .position(|byte| *byte == b'\n')
                            .map_or(available.len(), |position| position + 1);
                        if line.len().saturating_add(take) > MAX_RESPONSE_LINE_BYTES {
                            return Err(std::io::Error::new(
                                std::io::ErrorKind::InvalidData,
                                "JSON-RPC response line exceeds size limit",
                            ));
                        }
                        line.extend_from_slice(&available[..take]);
                        reader.consume(take);
                        if line.last() == Some(&b'\n') {
                            return Ok(line.len());
                        }
                    }
                } => result,
                _ = self.cancel_token.cancelled() => Err(std::io::Error::new(
                    std::io::ErrorKind::Interrupted,
                    "JSON-RPC operation cancelled",
                )),
            }
        })
        .await;
        match read_result {
            Ok(Ok(0)) => {
                let _inner = reader.into_inner();
                self.process.as_mut().unwrap().stdout = Some(_inner);
                bail!("unexpected EOF — no response from subprocess");
            }
            Ok(Ok(_)) => {} // got a line
            Ok(Err(e)) => {
                let _inner = reader.into_inner();
                self.process.as_mut().unwrap().stdout = Some(_inner);
                return Err(e).context("failed to read response from subprocess");
            }
            Err(_) => {
                let _inner = reader.into_inner();
                self.process.as_mut().unwrap().stdout = Some(_inner);
                bail!("JSON-RPC call timed out");
            }
        }

        // Restore stdout (BufReader may have pre-buffered more data;
        // for request-response protocol this is acceptable loss).
        let _inner = reader.into_inner();
        self.process.as_mut().unwrap().stdout = Some(_inner);

        // --- response line length guard (10 MB) ------------------------------
        let line = std::str::from_utf8(&line).context("JSON-RPC response is not UTF-8")?;
        let line = line.trim_end();
        let response: JsonRpcResponse =
            serde_json::from_str(line).context("invalid JSON-RPC response")?;

        // --- jsonrpc version check -----------------------------------------
        anyhow::ensure!(
            response.jsonrpc.as_deref() == Some("2.0"),
            "JSON-RPC response has an invalid protocol version"
        );

        // --- response id check ---------------------------------------------
        if response.id != Some(id) {
            bail!(
                "JSON-RPC response id mismatch: expected {}, got {:?}",
                id,
                response.id
            );
        }

        if let Some(err) = response.error {
            bail!("JSON-RPC error {}: {}", err.code, err.message);
        }
        Ok(response.result.unwrap_or(Value::Null))
    }

    /// Signal cancellation — kills the child process.
    pub fn cancel(&self) {
        self.cancel_token.cancel();
    }

    /// Gracefully shut down the subprocess.
    pub async fn shutdown(&mut self) {
        if self.process.is_some() {
            let _ = timeout(
                self.config.shutdown_timeout,
                self.call("shutdown", serde_json::json!({})),
            )
            .await;
        }
        if let Some(mut child) = self.process.take() {
            let _ = child.kill().await;
        }
    }

    pub fn health(&self) -> Result<()> {
        anyhow::ensure!(!self.quarantined, "runtime is quarantined");
        anyhow::ensure!(self.isolation_verified, "runtime isolation is not verified");
        anyhow::ensure!(self.process.is_some(), "runtime process is not running");
        Ok(())
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
        self.cancel_token.cancel();
        if let Some(mut child) = self.process.take() {
            let _ = child.start_kill();
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::ResolvedSandboxPolicy;
    use std::collections::BTreeMap;
    use tempfile::TempDir;

    async fn isolated_fixture(
        mode: &str,
        call_timeout: Duration,
        forbidden: Option<&std::path::Path>,
    ) -> (SubprocessRuntime, TempDir) {
        let work = TempDir::new().unwrap();
        let helper = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/extension_jsonrpc_runtime.py")
            .canonicalize()
            .unwrap();
        let backend = corpus::security::sandbox::BubblewrapBackend::probe_async(Arc::new(
            kernel::chronos::TestClock::default(),
        ))
        .await
        .expect("bubblewrap must be available for the extension runtime gate");
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots(work.path().to_path_buf(), vec![])
                .unwrap();
        let mut environment = BTreeMap::from([("EXTENSION_FIXTURE_MODE".into(), mode.to_string())]);
        if let Some(path) = forbidden {
            environment.insert(
                "EXTENSION_FORBIDDEN_PATH".into(),
                path.to_string_lossy().into_owned(),
            );
        }
        let policy = ResolvedSandboxPolicy {
            name: "extension-runtime-test".into(),
            read_only_roots: vec![
                "/usr".into(),
                "/lib".into(),
                "/lib64".into(),
                "/bin".into(),
                "/etc".into(),
                helper.parent().unwrap().to_path_buf(),
            ],
            read_write_roots: vec![work.path().to_path_buf()],
            deny_exact: Vec::new(),
            deny_globs: Vec::new(),
            restrict_network: true,
        };
        let config = SubprocessConfig {
            command: "/usr/bin/python3".into(),
            args: vec![helper.to_string_lossy().into_owned()],
            working_dir: Some(work.path().to_string_lossy().into_owned()),
            call_timeout,
            start_timeout: Duration::from_secs(5),
            shutdown_timeout: Duration::from_secs(1),
            ..Default::default()
        };
        let runtime = SubprocessRuntime::new_isolated(
            config,
            &backend,
            &SandboxConfig {
                workspace,
                environment,
                policy: Some(policy),
            },
        )
        .unwrap();
        (runtime, work)
    }

    async fn isolated_provider(
        mode: &str,
        threshold: u32,
    ) -> (SubprocessAgentRuntimeProvider, TempDir) {
        let work = TempDir::new().unwrap();
        let helper = std::path::PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/extension_jsonrpc_runtime.py")
            .canonicalize()
            .unwrap();
        let backend = corpus::security::sandbox::BubblewrapBackend::probe_async(Arc::new(
            kernel::chronos::TestClock::default(),
        ))
        .await
        .unwrap();
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots(work.path().to_path_buf(), vec![])
                .unwrap();
        let policy = ResolvedSandboxPolicy {
            name: "extension-provider-test".into(),
            read_only_roots: vec![
                "/usr".into(),
                "/lib".into(),
                "/lib64".into(),
                "/bin".into(),
                "/etc".into(),
                helper.parent().unwrap().to_path_buf(),
            ],
            read_write_roots: vec![work.path().to_path_buf()],
            deny_exact: Vec::new(),
            deny_globs: Vec::new(),
            restrict_network: true,
        };
        let provider = SubprocessAgentRuntimeProvider::new(
            SubprocessConfig {
                command: "/usr/bin/python3".into(),
                args: vec![helper.to_string_lossy().into_owned()],
                working_dir: Some(work.path().to_string_lossy().into_owned()),
                call_timeout: Duration::from_secs(2),
                circuit_breaker_threshold: threshold,
                ..Default::default()
            },
            Arc::new(backend),
            SandboxConfig {
                workspace,
                environment: BTreeMap::from([("EXTENSION_FIXTURE_MODE".into(), mode.into())]),
                policy: Some(policy),
            },
        )
        .unwrap();
        (provider, work)
    }

    fn spawn_request() -> AgentSpawnRequest {
        AgentSpawnRequest {
            root_agent_id: fabric::AgentId(uuid::Uuid::new_v4()),
            parent_agent_id: None,
            parent_process_id: None,
            profile_id: fabric::AgentProfileId("test".into()),
            runtime_id: fabric::RuntimeId("runtime.generic".into()),
            trusted_workspace: None,
            task: "test".into(),
            context: fabric::AgentContextFork::default(),
            broadcast_refs: Vec::new(),
            allowed_tools: Vec::new(),
            budget: fabric::AgentBudget {
                max_input_tokens: 1,
                max_output_tokens: 1,
                max_tool_calls: 1,
                max_elapsed_ms: 1,
                max_cost_usd: None,
                max_depth: 1,
            },
            background_decls: Vec::new(),
        }
    }

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
        assert!(rt.health().is_err());
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

    #[test]
    fn stderr_snapshot_redacts_secret_bearing_lines() {
        let mut stderr = SanitizedStderr::new();
        stderr.append(b"normal diagnostic\napi_key=should-not-leak\n");
        let snapshot = stderr.snapshot();
        assert!(snapshot.contains("normal diagnostic"));
        assert!(snapshot.contains("[REDACTED]"));
        assert!(!snapshot.contains("should-not-leak"));
    }

    #[tokio::test]
    async fn unisolated_runtime_fails_closed_before_spawn() {
        let mut config = SubprocessConfig::default();
        config.command = "/bin/true".into();
        let mut runtime = SubprocessRuntime::new(config);
        let error = runtime.start().await.unwrap_err().to_string();
        assert!(error.contains("no verified isolation backend"));
    }

    #[tokio::test]
    async fn real_isolated_helper_covers_protocol_and_secret_redaction() {
        let (mut normal, _work) = isolated_fixture("normal", Duration::from_secs(2), None).await;
        normal.start().await.unwrap();
        assert!(
            normal.call("observe", serde_json::json!({})).await.unwrap()["ok"]
                .as_bool()
                .unwrap()
        );
        normal.shutdown().await;

        for mode in ["wrong_id", "wrong_version", "oversized", "crash"] {
            let (mut runtime, _work) = isolated_fixture(mode, Duration::from_secs(2), None).await;
            runtime.start().await.unwrap();
            assert!(
                runtime
                    .call("observe", serde_json::json!({}))
                    .await
                    .is_err(),
                "{mode} must fail closed"
            );
            assert!(
                runtime.process.is_none(),
                "{mode} process was not terminated"
            );
        }

        let (mut stderr, _work) =
            isolated_fixture("stderr_secret", Duration::from_secs(2), None).await;
        stderr.start().await.unwrap();
        stderr.call("observe", serde_json::json!({})).await.unwrap();
        tokio::time::sleep(Duration::from_millis(50)).await;
        let snapshot = stderr.stderr.lock().await.snapshot();
        assert!(snapshot.contains("[REDACTED]"));
        assert!(!snapshot.contains("fixture-must-be-redacted"));
        stderr.shutdown().await;
    }

    #[tokio::test]
    async fn real_isolated_helper_enforces_timeout_cancel_filesystem_and_network() {
        let forbidden = TempDir::new().unwrap();
        let secret = forbidden.path().join("secret");
        std::fs::write(&secret, "must not be visible").unwrap();
        let (mut filesystem, _work) =
            isolated_fixture("filesystem_probe", Duration::from_secs(2), Some(&secret)).await;
        filesystem.start().await.unwrap();
        assert_eq!(
            filesystem
                .call("probe", serde_json::json!({}))
                .await
                .unwrap()["allowed"],
            false
        );
        filesystem.shutdown().await;

        let (mut network, _work) =
            isolated_fixture("network_probe", Duration::from_secs(2), None).await;
        network.start().await.unwrap();
        assert_eq!(
            network.call("probe", serde_json::json!({})).await.unwrap()["allowed"],
            false
        );
        network.shutdown().await;

        let (mut hanging, _work) = isolated_fixture("hang", Duration::from_millis(100), None).await;
        hanging.start().await.unwrap();
        assert!(hanging.call("wait", serde_json::json!({})).await.is_err());
        assert!(hanging.process.is_none());

        let (mut cancelled, _work) = isolated_fixture("hang", Duration::from_secs(10), None).await;
        cancelled.start().await.unwrap();
        let token = cancelled.cancellation_token();
        tokio::spawn(async move {
            tokio::time::sleep(Duration::from_millis(50)).await;
            token.cancel();
        });
        let started = Instant::now();
        assert!(cancelled.call("wait", serde_json::json!({})).await.is_err());
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(cancelled.process.is_none());
    }

    #[tokio::test]
    async fn provider_circuit_breaker_accumulates_business_failures_across_sessions() {
        let (provider, _work) = isolated_provider("business_fail", 2).await;
        for _ in 0..2 {
            let handle = provider.start(spawn_request()).await.unwrap();
            assert!(provider.observe(&handle).await.is_err());
        }
        let error = provider
            .start(spawn_request())
            .await
            .unwrap_err()
            .to_string();
        assert!(error.contains("quarantined"));
        assert!(provider.health().await.is_err());
    }
}
