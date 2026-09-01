//! Supervised resident Pi RPC adapter for one live Agent.

use std::collections::BTreeMap;
use std::sync::{Arc, OnceLock, Weak};
use std::time::Duration;

use crate::composition::agent_control::{
    AgentEventSink, AgentHostEffects, AgentRuntimeEvent, AgentRuntimeInput, AgentRuntimeLauncher,
    RuntimeObservedAgentBackend,
};
use ::contracts::sandbox::{IsolationLevel, SandboxBackend, SandboxConfig};
use ::contracts::{
    AgentControlError, AgentControlErrorKind, AgentMessageKind, AgentResult, AgentRunStatus,
    AttemptEvidence, AttemptUsage, WorkspacePolicy,
};
use adapters_agent_backend::pi::{
    pi_environment_from_process, pi_sandbox_policy, resolve_pi_config, ResolvedPiConfig,
    PI_CODER_RUNTIME_ID,
};
use adapters_agent_backend::pi_protocol::{
    parse_rpc_record, validate_rpc_response, PiRpcCommand, PiRpcRecord,
};
use async_trait::async_trait;
use kernel::process::controller::{
    process_start_time_ticks, LinuxProcessController, ManagedProcess, ProcessController,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, ChildStderr, ChildStdin, ChildStdout};

pub const PI_DELEGATE_ALIAS: &str = "pi-rpc";
const REQUIRED_ISOLATION_FLAGS: &[&str] = &[
    "--no-session",
    "--no-context-files",
    "--no-extensions",
    "--no-skills",
    "--no-prompt-templates",
    "--no-themes",
    "--no-approve",
    "--offline",
];

/// Import only reviewed process environment keys. Values remain inside the
/// sandbox command and are never copied into Agent results or evidence.
pub fn pi_rpc_environment_from_process() -> BTreeMap<String, String> {
    pi_environment_from_process()
}

/// A reviewed runtime that creates one isolated Pi process per child Agent.
pub struct PiDelegateBackend {
    config: ResolvedPiConfig,
    rpc_args: Vec<String>,
    sandbox: Arc<dyn SandboxBackend>,
    credential_environment: BTreeMap<String, String>,
    process_controller: Arc<dyn ProcessController>,
    /// Bound exactly once by the daemon composition root. The backend owns
    /// the Pi process/protocol state; the host service only supplies the
    /// already-admitted Agent projection and mailbox bridge.
    host: OnceLock<Weak<dyn AgentHostEffects>>,
    self_ref: OnceLock<Weak<PiDelegateBackend>>,
}

impl PiDelegateBackend {
    pub fn prepare(
        source: &crate::config::CodingRuntimeConfig,
        sandbox: Arc<dyn SandboxBackend>,
        clock: Arc<dyn ::contracts::Clock>,
        credential_environment: BTreeMap<String, String>,
    ) -> Result<Option<Self>, AgentControlError> {
        Self::prepare_with_process_controller(
            source,
            sandbox,
            clock,
            credential_environment,
            Arc::new(LinuxProcessController),
        )
    }

    /// Composition seam used by the daemon to inject the host process
    /// controller. The default `prepare` path remains available for focused
    /// compatibility tests, but production process creation is owned by the
    /// controller rather than this protocol adapter.
    pub fn prepare_with_process_controller(
        source: &crate::config::CodingRuntimeConfig,
        sandbox: Arc<dyn SandboxBackend>,
        _clock: Arc<dyn ::contracts::Clock>,
        credential_environment: BTreeMap<String, String>,
        process_controller: Arc<dyn ProcessController>,
    ) -> Result<Option<Self>, AgentControlError> {
        let Some(validated) = resolve_pi_config(source, sandbox.clone())
            .map_err(|error| runtime_error(format!("validating Pi RPC configuration: {error}")))?
        else {
            return Ok(None);
        };
        Self::from_validated(
            validated,
            sandbox,
            credential_environment,
            process_controller,
        )
        .map(Some)
    }

    fn from_validated(
        config: ResolvedPiConfig,
        sandbox: Arc<dyn SandboxBackend>,
        credential_environment: BTreeMap<String, String>,
        process_controller: Arc<dyn ProcessController>,
    ) -> Result<Self, AgentControlError> {
        validate_sandbox(sandbox.as_ref())?;
        if config.package_version.trim().is_empty() || config.executable_sha256.len() != 64 {
            return Err(runtime_error("Pi RPC build identity is not pinned"));
        }
        let mut rpc_args = config.fixed_args.clone();
        replace_mode_with_rpc(&mut rpc_args)?;
        for required in REQUIRED_ISOLATION_FLAGS {
            if !rpc_args.iter().any(|arg| arg == required) {
                return Err(runtime_error(format!(
                    "Pi RPC isolation flag is missing: {required}"
                )));
            }
        }
        Ok(Self {
            config,
            rpc_args,
            sandbox,
            credential_environment,
            process_controller,
            host: OnceLock::new(),
            self_ref: OnceLock::new(),
        })
    }

    /// Bind the single composition-root host service and this backend's own
    /// Arc after construction. Both cells are write-once so a reload cannot
    /// silently retarget an admitted Pi delegate.
    pub fn bind_host(
        &self,
        host: &Arc<dyn AgentHostEffects>,
        self_ref: &Arc<Self>,
    ) -> Result<(), AgentControlError> {
        self.host
            .set(Arc::downgrade(&host))
            .map_err(|_| runtime_error("Pi delegate service was already bound"))?;
        self.self_ref
            .set(Arc::downgrade(self_ref))
            .map_err(|_| runtime_error("Pi delegate self reference was already bound"))?;
        Ok(())
    }

    fn validate_backend(
        request: &runtime::DelegateSpawnRequest,
    ) -> Result<(), runtime::RuntimeError> {
        if request.backend.0 != PI_CODER_RUNTIME_ID || request.host_request.is_none() {
            return Err(runtime::RuntimeError::UnsupportedRequest);
        }
        Ok(())
    }

    fn observed(&self) -> RuntimeObservedAgentBackend {
        let host = self.host.get().cloned().unwrap_or_else(|| {
            let unavailable: Arc<dyn AgentHostEffects> = Arc::new(UnavailableAgentHostEffects);
            Arc::downgrade(&unavailable)
        });
        RuntimeObservedAgentBackend::from_host(host, None)
    }

    pub fn runtime_id() -> ::contracts::RuntimeId {
        ::contracts::RuntimeId(PI_CODER_RUNTIME_ID.into())
    }

    async fn spawn(
        &self,
        workspace: &WorkspacePolicy,
    ) -> Result<(Child, u32, ChildStdin, BufReader<ChildStdout>, ChildStderr), AgentControlError>
    {
        let policy = pi_sandbox_policy(workspace, self.config.network_enabled)
            .map_err(|error| runtime_error(format!("resolving Pi RPC sandbox policy: {error}")))?;
        let sandbox_config = SandboxConfig {
            workspace: workspace.clone(),
            environment: self.credential_environment.clone(),
            policy: Some(policy),
        };
        let wrapped = self
            .sandbox
            .wrap_argv(&self.config.executable, &self.rpc_args, &sandbox_config)
            .map_err(|error| runtime_error(format!("wrapping Pi RPC sandbox argv: {error}")))?;
        let ManagedProcess {
            mut child,
            process_group,
        } = self
            .process_controller
            .spawn(wrapped, workspace.cwd())
            .await
            .map_err(|error| {
                runtime_error(format!("starting sandboxed Pi RPC process: {}", error))
            })?;
        let stdin = match child.stdin.take() {
            Some(stdin) => stdin,
            None => {
                return Err(self
                    .terminate_spawn_failure(process_group, &mut child, "stdin")
                    .await)
            }
        };
        let stdout = match child.stdout.take() {
            Some(stdout) => stdout,
            None => {
                return Err(self
                    .terminate_spawn_failure(process_group, &mut child, "stdout")
                    .await)
            }
        };
        let stderr = match child.stderr.take() {
            Some(stderr) => stderr,
            None => {
                return Err(self
                    .terminate_spawn_failure(process_group, &mut child, "stderr")
                    .await)
            }
        };
        Ok((child, process_group, stdin, BufReader::new(stdout), stderr))
    }

    async fn terminate_spawn_failure(
        &self,
        process_group: u32,
        child: &mut Child,
        stream: &str,
    ) -> AgentControlError {
        match self
            .process_controller
            .terminate(process_group, child)
            .await
        {
            Ok(()) => runtime_error(format!("Pi RPC {stream} is unavailable")),
            Err(error) => runtime_error(format!(
                "Pi RPC {stream} is unavailable; process cleanup failed: {error}"
            )),
        }
    }
}

struct UnavailableAgentHostEffects;

#[async_trait]
impl AgentHostEffects for UnavailableAgentHostEffects {
    async fn launch_admitted(
        &self,
        _request: ::contracts::AgentSpawnRequest,
        _identity: runtime::DelegateReceipt,
        _launcher: Option<Arc<dyn AgentRuntimeLauncher>>,
    ) -> Result<(), AgentControlError> {
        Err(runtime_error("Pi delegate host effects are not bound"))
    }

    async fn cancel_admitted(
        &self,
        _agent_run: &runtime::AgentRunId,
    ) -> Result<(), runtime::RuntimeError> {
        Err(runtime::RuntimeError::AgentRunNotFound)
    }

    async fn send_admitted(
        &self,
        _agent_run: &runtime::AgentRunId,
        _message: &runtime::DelegateMessage,
    ) -> Result<runtime::DelegateMessageReceipt, runtime::RuntimeError> {
        Err(runtime::RuntimeError::AgentRunNotFound)
    }

    async fn wait_admitted(
        &self,
        _agent_run: &runtime::AgentRunId,
        _timeout: std::time::Duration,
    ) -> Result<runtime::TurnTerminal, runtime::RuntimeError> {
        Err(runtime::RuntimeError::AgentRunNotFound)
    }

    async fn resume_admitted(
        &self,
        _recovery: &runtime::DelegateRecoveryRequest,
        _launcher: Option<Arc<dyn AgentRuntimeLauncher>>,
    ) -> Result<(), runtime::RuntimeError> {
        Err(runtime::RuntimeError::AgentRunNotFound)
    }
}

#[async_trait]
impl runtime::DelegateBackend for PiDelegateBackend {
    async fn spawn(
        &self,
        request: &runtime::DelegateSpawnRequest,
        identity: &runtime::DelegateReceipt,
    ) -> Result<runtime::DelegateReceipt, runtime::RuntimeError> {
        Self::validate_backend(request)?;
        let host = self
            .host
            .get()
            .and_then(Weak::upgrade)
            .ok_or(runtime::RuntimeError::AgentRunNotFound)?;
        let launcher = self
            .self_ref
            .get()
            .and_then(Weak::upgrade)
            .map(|backend| backend as Arc<dyn AgentRuntimeLauncher>)
            .ok_or(runtime::RuntimeError::AgentRunNotFound)?;
        let host_request = request
            .host_request
            .clone()
            .ok_or(runtime::RuntimeError::UnsupportedRequest)?;
        host.launch_admitted(host_request, identity.clone(), Some(launcher))
            .await
            .map_err(|error| {
                tracing::error!(
                    agent_run = %identity.agent_run.0,
                    kind = ?error.kind,
                    message = %error.message,
                    "Pi Delegate admission failed"
                );
                runtime::RuntimeError::Internal
            })?;
        Ok(identity.clone())
    }

    async fn cancel(&self, agent_run: &runtime::AgentRunId) -> Result<(), runtime::RuntimeError> {
        runtime::DelegateBackend::cancel(&self.observed(), agent_run).await
    }

    async fn send(
        &self,
        agent_run: &runtime::AgentRunId,
        message: &runtime::DelegateMessage,
    ) -> Result<(), runtime::RuntimeError> {
        runtime::DelegateBackend::send(&self.observed(), agent_run, message).await
    }

    async fn send_with_receipt(
        &self,
        agent_run: &runtime::AgentRunId,
        message: &runtime::DelegateMessage,
    ) -> Result<runtime::DelegateMessageReceipt, runtime::RuntimeError> {
        runtime::DelegateBackend::send_with_receipt(&self.observed(), agent_run, message).await
    }

    async fn wait(
        &self,
        agent_run: &runtime::AgentRunId,
    ) -> Result<runtime::TurnTerminal, runtime::RuntimeError> {
        runtime::DelegateBackend::wait(&self.observed(), agent_run).await
    }
}

#[async_trait]
impl AgentRuntimeLauncher for PiDelegateBackend {
    fn resource_requirements(&self) -> runtime::RuntimeResourceRequirements {
        pi_manifest().resource_requirements
    }

    async fn launch(
        &self,
        input: AgentRuntimeInput,
        events: Arc<dyn AgentEventSink>,
    ) -> Result<AgentResult, AgentControlError> {
        let workspace = input.workspace.as_ref().ok_or_else(|| {
            runtime_error("Pi RPC spawn lacks host-injected trusted workspace authority")
        })?;
        let configured_roots = configured_roots(workspace.cwd(), &self.config.allowed_paths)?;
        let protected = ::contracts::ProtectedPathPolicy::new(
            workspace
                .protected_paths()
                .credential_paths()
                .iter()
                .cloned()
                .chain(
                    self.config
                        .forbidden_paths
                        .iter()
                        .map(|path| workspace.cwd().join(path)),
                )
                .collect(),
        )
        .map_err(|error| runtime_error(format!("resolving Pi RPC protected paths: {error}")))?;
        let workspace = workspace
            .clone()
            .narrow_writable_roots(configured_roots)
            .map_err(|error| {
                runtime_error(format!(
                    "Pi RPC configured path allowlist exceeds trusted workspace: {error}"
                ))
            })?
            .with_protected_paths(protected);
        let (mut child, process_group, mut stdin, mut stdout, stderr) =
            self.spawn(&workspace).await?;
        let stderr_task = tokio::spawn(read_capped(stderr, 16 * 1024));
        let start_time_ticks = process_start_time_ticks(process_group)
            .map_err(|error| runtime_error(format!("reading Pi RPC process identity: {error}")));
        let runtime_identity = match start_time_ticks {
            Ok(start_time_ticks) => {
                input
                    .runtime_process
                    .register(::contracts::OsProcessId(process_group), start_time_ticks)
                    .await
            }
            Err(error) => Err(error),
        };
        let runtime_identity = match runtime_identity {
            Ok(identity) => identity,
            Err(error) => {
                self.process_controller
                    .terminate(process_group, &mut child)
                    .await
                    .map_err(|cleanup| runtime_error(cleanup.to_string()))?;
                return Err(error);
            }
        };
        let ids = (
            &input.handle.agent_id,
            &input.handle.process_id,
            &input.handle.operation_id,
        );
        events
            .emit(AgentRuntimeEvent::Started {
                agent_id: *ids.0,
                process_id: *ids.1,
                operation_id: *ids.2,
            })
            .await;

        let mut state = RpcState::default();
        let mut next_id = 1_u64;
        let initial = PiRpcCommand::Prompt {
            id: command_id(input.handle.agent_id, next_id),
            message: input.request.task.clone(),
        };
        write_command(&mut stdin, &initial).await?;
        let mut pending = Some(initial);
        let timeout = Duration::from_millis(
            input
                .request
                .budget
                .max_elapsed_ms
                .min(self.config.timeout_ms),
        );
        let deadline = tokio::time::sleep(timeout);
        tokio::pin!(deadline);

        let mut outcome = async {
            loop {
                tokio::select! {
                biased;
                _ = input.cancellation.cancelled() => {
                    next_id += 1;
                    let abort = PiRpcCommand::Abort { id: command_id(input.handle.agent_id, next_id) };
                    let _ = write_command(&mut stdin, &abort).await;
                    break Err(terminal_error("Pi RPC Agent cancelled"));
                }
                _ = &mut deadline => break Err(runtime_error("Pi RPC Agent exceeded its elapsed-time budget")),
                record = read_record(&mut stdout, self.config.max_output_bytes) => {
                    let record = record?;
                    match record {
                        PiRpcRecord::Response { .. } => {
                            let command = pending.take().ok_or_else(|| runtime_error("Pi RPC emitted an unsolicited response"))?;
                            let data = validate_rpc_response(record, &command)
                                .map_err(|error| runtime_error(error.to_string()))?;
                            if matches!(command, PiRpcCommand::GetState { .. }) {
                                let data = data.ok_or_else(|| runtime_error("Pi get_state response lacks data"))?;
                                if data.get("isStreaming").and_then(Value::as_bool) != Some(false) {
                                    break Err(runtime_error("Pi reported streaming after agent_settled"));
                                }
                                break state.finish();
                            }
                            if state.settled {
                                next_id += 1;
                                let get_state = PiRpcCommand::GetState { id: command_id(input.handle.agent_id, next_id) };
                                write_command(&mut stdin, &get_state).await?;
                                pending = Some(get_state);
                            }
                        }
                        PiRpcRecord::Event(event) => {
                            state.apply_event(&event, &input, events.as_ref()).await?;
                            if state.settled && pending.is_none() {
                                next_id += 1;
                                let get_state = PiRpcCommand::GetState { id: command_id(input.handle.agent_id, next_id) };
                                write_command(&mut stdin, &get_state).await?;
                                pending = Some(get_state);
                            }
                        }
                    }
                }
                status = child.wait() => {
                    let status = status.map_err(|error| runtime_error(format!("waiting for Pi RPC process: {error}")))?;
                    break Err(runtime_error(format!("Pi RPC process exited before settlement: {status}")));
                }
                message = input.inbox.recv(), if pending.is_none() && !state.settled => {
                    let Some(message) = message else { continue };
                    if message.kind != AgentMessageKind::Input {
                        break Err(runtime_error("Pi RPC inbox accepts only Agent input messages"));
                    }
                    next_id += 1;
                    let command = if message.start_turn {
                        PiRpcCommand::FollowUp { id: command_id(input.handle.agent_id, next_id), message: message.content }
                    } else {
                        PiRpcCommand::Steer { id: command_id(input.handle.agent_id, next_id), message: message.content }
                    };
                    write_command(&mut stdin, &command).await?;
                    pending = Some(command);
                }
                }
            }
        }
        .await;

        drop(stdin);
        if let Err(cleanup) = self
            .process_controller
            .terminate(process_group, &mut child)
            .await
        {
            if outcome.is_ok() {
                outcome = Err(runtime_error(cleanup.to_string()));
            }
        }
        let stderr = stderr_task.await.unwrap_or_default();
        let stderr = redact_runtime_diagnostic(&stderr, &self.credential_environment);
        let mut outcome = outcome.map_err(|error| {
            if stderr.is_empty() {
                error
            } else {
                AgentControlError {
                    kind: error.kind,
                    message: format!("{}; Pi stderr: {stderr}", error.message),
                }
            }
        });
        if let Ok(result) = outcome.as_mut() {
            if !stderr.is_empty() && result.evidence.len() < 128 {
                result.evidence.push(
                    AttemptEvidence {
                        kind: "pi_rpc_diagnostic".into(),
                        summary: "bounded Pi runtime diagnostic".into(),
                        content: stderr,
                    }
                    .bounded_for_persistence(16 * 1024),
                );
            }
        }
        if let Err(error) = input.runtime_process.clear(runtime_identity).await {
            return Err(runtime_error(format!(
                "clearing Pi RPC process identity: {}",
                error.message
            )));
        }
        let (status, result) = match &outcome {
            Ok(result) => (AgentRunStatus::Succeeded, Some(result.clone())),
            Err(error) if error.kind == AgentControlErrorKind::Terminal => {
                (AgentRunStatus::Cancelled, None)
            }
            Err(_) => (AgentRunStatus::Failed, None),
        };
        events
            .emit(AgentRuntimeEvent::Terminal {
                agent_id: *ids.0,
                process_id: *ids.1,
                operation_id: *ids.2,
                status,
                result,
            })
            .await;
        outcome
    }
}

#[derive(Default)]
struct RpcState {
    started: bool,
    settled: bool,
    final_text: Option<String>,
    usage: AttemptUsage,
    evidence: Vec<AttemptEvidence>,
}

impl RpcState {
    async fn apply_event(
        &mut self,
        event: &Value,
        input: &AgentRuntimeInput,
        events: &dyn AgentEventSink,
    ) -> Result<(), AgentControlError> {
        if self.settled {
            return Err(runtime_error("Pi RPC emitted an event after agent_settled"));
        }
        match event
            .get("type")
            .and_then(Value::as_str)
            .unwrap_or_default()
        {
            "agent_start" => {
                if !self.started {
                    self.started = true;
                }
                // Pi emits another start marker when it restarts generation
                // after a provider retry. The marker carries no identity or
                // authorization state, so treating it as idempotent avoids
                // turning a recoverable provider retry into a failed child.
            }
            "agent_settled" => {
                if !self.started {
                    return Err(runtime_error("Pi agent_settled preceded agent_start"));
                }
                self.settled = true;
            }
            "message_end" => {
                if !self.started {
                    return Err(runtime_error("Pi RPC message preceded agent_start"));
                }
                if let Some(message) = event
                    .get("message")
                    .filter(|m| m.get("role").and_then(Value::as_str) == Some("assistant"))
                {
                    self.final_text = message_text(message).or_else(|| self.final_text.take());
                    accumulate_usage(message.get("usage"), &mut self.usage);
                }
            }
            "agent_end" => {
                if !self.started {
                    return Err(runtime_error("Pi agent_end preceded agent_start"));
                }
                // Pi's retry path can omit the final `message_end` event while
                // still returning the authoritative conversation snapshot on
                // `agent_end`. Recover the last assistant message from that
                // snapshot instead of turning a completed retry into a failed
                // child run.
                if let Some(message) =
                    event
                        .get("messages")
                        .and_then(Value::as_array)
                        .and_then(|messages| {
                            messages.iter().rev().find(|message| {
                                message.get("role").and_then(Value::as_str) == Some("assistant")
                            })
                        })
                {
                    self.final_text = message_text(message).or_else(|| self.final_text.take());
                    accumulate_usage(message.get("usage"), &mut self.usage);
                }
            }
            "tool_execution_end" => {
                if !self.started {
                    return Err(runtime_error("Pi RPC tool event preceded agent_start"));
                }
                if self.evidence.len() >= 128 {
                    return Err(runtime_error("Pi RPC exceeded the tool evidence limit"));
                }
                let name = event
                    .get("toolName")
                    .and_then(Value::as_str)
                    .ok_or_else(|| runtime_error("Pi tool event lacks toolName"))?;
                let is_error = event
                    .get("isError")
                    .and_then(Value::as_bool)
                    .ok_or_else(|| runtime_error("Pi tool event lacks isError"))?;
                events
                    .emit(AgentRuntimeEvent::Tool {
                        agent_id: input.handle.agent_id,
                        process_id: input.handle.process_id,
                        operation_id: input.handle.operation_id,
                        name: name.into(),
                        is_error,
                    })
                    .await;
                self.evidence.push(
                    AttemptEvidence {
                        kind: "pi_rpc_tool".into(),
                        summary: format!(
                            "Pi tool {name} {}",
                            if is_error { "failed" } else { "completed" }
                        ),
                        content: serde_json::to_string(event.get("result").unwrap_or(&Value::Null))
                            .unwrap_or_default(),
                    }
                    .bounded_for_persistence(16 * 1024),
                );
            }
            "auto_retry_start" | "compaction_start" => {
                events
                    .emit(AgentRuntimeEvent::Progress {
                        agent_id: input.handle.agent_id,
                        process_id: input.handle.process_id,
                        operation_id: input.handle.operation_id,
                        summary: format!("Pi RPC {}", event["type"].as_str().unwrap_or_default()),
                    })
                    .await;
            }
            _ => {}
        }
        Ok(())
    }

    fn finish(self) -> Result<AgentResult, AgentControlError> {
        let output = self
            .final_text
            .filter(|text| !text.trim().is_empty())
            .ok_or_else(|| runtime_error("Pi RPC settled without terminal assistant text"))?;
        let result = AgentResult {
            output,
            usage: self.usage,
            evidence: self.evidence,
            artifacts: vec![],
        };
        result.validate()?;
        Ok(result)
    }
}

async fn write_command(
    stdin: &mut ChildStdin,
    command: &PiRpcCommand,
) -> Result<(), AgentControlError> {
    let line = command
        .to_jsonl()
        .map_err(|error| runtime_error(error.to_string()))?;
    stdin
        .write_all(line.as_bytes())
        .await
        .map_err(|error| runtime_error(format!("writing Pi RPC command: {error}")))?;
    stdin
        .flush()
        .await
        .map_err(|error| runtime_error(format!("flushing Pi RPC command: {error}")))
}

async fn read_record(
    stdout: &mut BufReader<ChildStdout>,
    max: usize,
) -> Result<PiRpcRecord, AgentControlError> {
    let mut record = Vec::new();
    let count = stdout
        .read_until(b'\n', &mut record)
        .await
        .map_err(|error| runtime_error(format!("reading Pi RPC record: {error}")))?;
    if count == 0 {
        return Err(runtime_error("Pi RPC stream ended before settlement"));
    }
    if record.len() > max {
        return Err(runtime_error(
            "Pi RPC record exceeds configured output limit",
        ));
    }
    parse_rpc_record(&record).map_err(|error| runtime_error(error.to_string()))
}

async fn read_capped<R>(mut reader: R, max: usize) -> String
where
    R: AsyncRead + Unpin,
{
    let mut bytes = Vec::with_capacity(max.min(4096));
    let mut chunk = [0_u8; 4096];
    let mut truncated = false;
    loop {
        let Ok(read) = reader.read(&mut chunk).await else {
            break;
        };
        if read == 0 {
            break;
        }
        let remaining = max.saturating_sub(bytes.len());
        bytes.extend_from_slice(&chunk[..read.min(remaining)]);
        truncated |= read > remaining;
        // Keep draining after reaching the evidence cap. Otherwise a noisy
        // child can fill the OS pipe and deadlock before terminal settlement.
    }
    if truncated {
        bytes.extend_from_slice(b"\n[stderr truncated]");
    }
    String::from_utf8_lossy(&bytes).into_owned()
}

fn redact_runtime_diagnostic(value: &str, credentials: &BTreeMap<String, String>) -> String {
    let mut redacted = value.replace(['\r', '\n'], " ");
    for secret in credentials.values().filter(|value| value.len() >= 4) {
        redacted = redacted.replace(secret, "[redacted]");
    }
    for marker in [
        "api_key=",
        "api-key:",
        "api key:",
        "apikey=",
        "authorization: bearer ",
        "secret=",
        "token=",
        "token:",
    ] {
        let mut search_from = 0;
        while let Some(offset) = redacted[search_from..].to_ascii_lowercase().find(marker) {
            let start = search_from + offset;
            let value_start = start + marker.len();
            let value_end = redacted[value_start..]
                .find(char::is_whitespace)
                .map(|offset| value_start + offset)
                .unwrap_or(redacted.len());
            redacted.replace_range(value_start..value_end, "[redacted]");
            search_from = value_start + "[redacted]".len();
        }
    }
    redacted.trim().chars().take(4096).collect()
}

fn replace_mode_with_rpc(args: &mut [String]) -> Result<(), AgentControlError> {
    let positions: Vec<_> = args
        .iter()
        .enumerate()
        .filter_map(|(i, arg)| (arg == "--mode").then_some(i))
        .collect();
    if positions.len() != 1 || positions[0] + 1 >= args.len() || args[positions[0] + 1] != "json" {
        return Err(runtime_error(
            "Pi fixed argv must contain exactly '--mode json'",
        ));
    }
    args[positions[0] + 1] = "rpc".into();
    Ok(())
}

fn configured_roots(
    cwd: &std::path::Path,
    configured: &[std::path::PathBuf],
) -> Result<Vec<std::path::PathBuf>, AgentControlError> {
    let canonical_cwd = cwd
        .canonicalize()
        .map_err(|error| runtime_error(format!("resolving trusted Pi RPC cwd: {error}")))?;
    let mut roots = Vec::with_capacity(configured.len());
    for relative in configured {
        let candidate = canonical_cwd.join(relative);
        let path = match candidate.canonicalize() {
            Ok(path) => path,
            // A configured writable path can legitimately be absent in a
            // different repository. Omitting it is fail-restrictive: Pi keeps
            // read-only workspace visibility but receives no write authority
            // for that path.
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => continue,
            Err(error) => {
                return Err(runtime_error(format!(
                    "resolving Pi RPC workspace allowlist: {error}"
                )))
            }
        };
        if !path.starts_with(&canonical_cwd) {
            return Err(runtime_error(
                "Pi RPC workspace allowlist escaped trusted cwd",
            ));
        }
        if !roots.contains(&path) {
            roots.push(path);
        }
    }
    Ok(roots)
}

fn validate_sandbox(sandbox: &dyn SandboxBackend) -> Result<(), AgentControlError> {
    let caps = sandbox.capabilities();
    if !sandbox.is_available()
        || !matches!(
            sandbox.isolation_level(),
            IsolationLevel::Namespace | IsolationLevel::Container
        )
        || !caps.filesystem_isolation
        || !caps.network_isolation
    {
        return Err(runtime_error(
            "Pi RPC requires an available filesystem-and-network namespace sandbox",
        ));
    }
    Ok(())
}

fn command_id(agent: ::contracts::AgentId, sequence: u64) -> String {
    format!("{}-{sequence}", agent.0)
}
fn runtime_error(message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Runtime,
        message: message.into(),
    }
}
fn terminal_error(message: impl Into<String>) -> AgentControlError {
    AgentControlError {
        kind: AgentControlErrorKind::Terminal,
        message: message.into(),
    }
}

fn message_text(message: &Value) -> Option<String> {
    let content = message.get("content")?;
    if let Some(text) = content.as_str() {
        return Some(text.into());
    }
    let text = content
        .as_array()?
        .iter()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<String>();
    (!text.is_empty()).then_some(text)
}

fn accumulate_usage(value: Option<&Value>, usage: &mut AttemptUsage) {
    let Some(value) = value else { return };
    usage.input_tokens = usage.input_tokens.saturating_add(
        value
            .get("inputTokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    );
    usage.output_tokens = usage.output_tokens.saturating_add(
        value
            .get("outputTokens")
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    );
    if let Some(cost) = value
        .get("cost")
        .and_then(|v| v.get("total"))
        .and_then(Value::as_f64)
    {
        usage.cost_usd = Some(usage.cost_usd.unwrap_or_default() + cost);
    }
}

// ── CapabilityRuntime impl (Wave 3) ─────────────────────────────────────
use std::collections::BTreeSet;

static PI_MANIFEST: std::sync::OnceLock<runtime::RuntimeManifest> = std::sync::OnceLock::new();

pub fn pi_manifest() -> &'static runtime::RuntimeManifest {
    PI_MANIFEST.get_or_init(|| runtime::RuntimeManifest {
        id: PI_CODER_RUNTIME_ID.into(),
        aliases: vec![PI_DELEGATE_ALIAS.into(), "pi".into()],
        display_name: "Pi Coding Runtime (RPC)".into(),
        capabilities: BTreeSet::from([
            runtime::RuntimeCapability::CodeRead,
            runtime::RuntimeCapability::CodeSearch,
            runtime::RuntimeCapability::CodeEdit,
            runtime::RuntimeCapability::Shell,
            runtime::RuntimeCapability::Test,
            runtime::RuntimeCapability::MemoryProposal,
        ]),
        interaction_modes: BTreeSet::from([
            runtime::InteractionMode::Resident,
            runtime::InteractionMode::Steering,
            runtime::InteractionMode::FollowUp,
        ]),
        workspace_modes: BTreeSet::from([
            runtime::WorkspaceMode::SharedReadOnly,
            runtime::WorkspaceMode::SharedWritable,
        ]),
        task_encodings: BTreeSet::from([
            runtime::TaskEncoding::NaturalLanguage,
            runtime::TaskEncoding::StructuredJson,
        ]),
        supported_profiles: None,
        tool_governance: runtime::ToolGovernance::Observed,
        priority: 10,
        max_context_tokens: Some(1_000_000),
        resource_requirements: runtime::RuntimeResourceRequirements {
            storage_bytes: 1024 * 1024 * 1024,
            storage_items: 1,
        },
    })
}

#[cfg(test)]
mod tests {
    use super::{configured_roots, read_capped, redact_runtime_diagnostic};
    use std::collections::BTreeMap;
    use tokio::io::AsyncWriteExt;

    #[test]
    fn missing_configured_writable_path_degrades_to_read_only() {
        let workspace = tempfile::tempdir().unwrap();

        let roots =
            configured_roots(workspace.path(), &[std::path::PathBuf::from("missing.rs")]).unwrap();

        assert!(roots.is_empty());
    }

    #[tokio::test]
    async fn stderr_capture_is_bounded_but_drains_to_eof() {
        let (reader, mut writer) = tokio::io::duplex(8);
        let write = tokio::spawn(async move {
            writer.write_all(b"0123456789abcdef").await.unwrap();
        });

        let captured = read_capped(reader, 8).await;
        write.await.unwrap();
        assert_eq!(captured, "01234567\n[stderr truncated]");
    }

    #[test]
    fn stderr_diagnostic_redacts_configured_and_inline_credentials() {
        let environment = BTreeMap::from([("OPENAI_API_KEY".into(), "sk-test-secret".into())]);
        let diagnostic = redact_runtime_diagnostic(
            "provider failed sk-test-secret\nAuthorization: Bearer another-secret",
            &environment,
        );

        assert!(!diagnostic.contains("sk-test-secret"));
        assert!(!diagnostic.contains("another-secret"));
        assert!(diagnostic.contains("[redacted]"));
    }
}
