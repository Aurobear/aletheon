//! Supervised resident Pi RPC adapter for one live Agent.

use std::collections::BTreeMap;
use std::sync::Arc;
use std::time::Duration;

use super::pi::{pi_environment_from_process, pi_sandbox_policy, PiRuntime, ResolvedPiConfig};
use super::pi_protocol::{parse_rpc_record, validate_rpc_response, PiRpcCommand, PiRpcRecord};
use crate::service::agent_control::{
    AgentEventSink, AgentRuntimeEvent, AgentRuntimeInput, AgentRuntimeLauncher,
};
use async_trait::async_trait;
use fabric::sandbox::{IsolationLevel, SandboxBackend, SandboxConfig};
use fabric::{
    AgentControlError, AgentControlErrorKind, AgentMessageKind, AgentResult, AgentRunStatus,
    AttemptEvidence, AttemptUsage, WorkspacePolicy,
};
use serde_json::Value;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::process::Command;
use tokio::process::{Child, ChildStdin, ChildStdout};

pub const PI_RPC_RUNTIME_ID: &str = "pi-rpc";
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
pub struct PiRpcRuntime {
    config: ResolvedPiConfig,
    rpc_args: Vec<String>,
    sandbox: Arc<dyn SandboxBackend>,
    credential_environment: BTreeMap<String, String>,
}

impl PiRpcRuntime {
    pub fn prepare(
        source: &cognit::config::PiRuntimeConfig,
        sandbox: Arc<dyn SandboxBackend>,
        clock: Arc<dyn fabric::Clock>,
        credential_environment: BTreeMap<String, String>,
    ) -> Result<Option<Self>, AgentControlError> {
        let Some(validated) = PiRuntime::prepare(source, sandbox.clone(), clock)
            .map_err(|error| runtime_error(format!("validating Pi RPC configuration: {error}")))?
        else {
            return Ok(None);
        };
        Self::from_validated(validated.config().clone(), sandbox, credential_environment).map(Some)
    }

    fn from_validated(
        config: ResolvedPiConfig,
        sandbox: Arc<dyn SandboxBackend>,
        credential_environment: BTreeMap<String, String>,
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
        })
    }

    pub fn runtime_id() -> fabric::RuntimeId {
        fabric::RuntimeId(PI_RPC_RUNTIME_ID.into())
    }

    async fn spawn(
        &self,
        workspace: &WorkspacePolicy,
    ) -> Result<(Child, u32, ChildStdin, BufReader<ChildStdout>), AgentControlError> {
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
        let mut command = Command::new(&wrapped.program);
        command
            .args(&wrapped.args)
            .env_clear()
            .envs(&wrapped.environment)
            .current_dir(workspace.cwd())
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::null())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);
        let mut child = command.spawn().map_err(|error| {
            runtime_error(format!("starting sandboxed Pi RPC process: {error}"))
        })?;
        let process_group = child
            .id()
            .ok_or_else(|| runtime_error("Pi RPC process lacks an operating-system id"))?;
        let stdin = child
            .stdin
            .take()
            .ok_or_else(|| runtime_error("Pi RPC stdin is unavailable"))?;
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| runtime_error("Pi RPC stdout is unavailable"))?;
        Ok((child, process_group, stdin, BufReader::new(stdout)))
    }
}

#[async_trait]
impl AgentRuntimeLauncher for PiRpcRuntime {
    async fn launch(
        &self,
        input: AgentRuntimeInput,
        events: Arc<dyn AgentEventSink>,
    ) -> Result<AgentResult, AgentControlError> {
        let workspace = input.workspace.as_ref().ok_or_else(|| {
            runtime_error("Pi RPC spawn lacks host-injected trusted workspace authority")
        })?;
        let configured_roots = configured_roots(workspace.cwd(), &self.config.allowed_paths)?;
        let protected = fabric::ProtectedPathPolicy::new(
            self.config
                .forbidden_paths
                .iter()
                .map(|path| workspace.cwd().join(path))
                .collect(),
        )
        .map_err(|error| runtime_error(format!("resolving Pi RPC protected paths: {error}")))?;
        if workspace.protected_paths() != &protected {
            return Err(runtime_error(
                "Pi RPC trusted workspace differs from configured protected paths",
            ));
        }
        let workspace = workspace
            .clone()
            .narrow_writable_roots(configured_roots)
            .map_err(|error| {
                runtime_error(format!(
                    "Pi RPC configured path allowlist exceeds trusted workspace: {error}"
                ))
            })?;
        let (mut child, process_group, mut stdin, mut stdout) = self.spawn(&workspace).await?;
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

        let outcome = loop {
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
        };

        drop(stdin);
        terminate_process_tree(process_group, &mut child).await;
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
                if self.started {
                    return Err(runtime_error("Pi RPC emitted duplicate agent_start"));
                }
                self.started = true;
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
        let path = canonical_cwd
            .join(relative)
            .canonicalize()
            .map_err(|error| {
                runtime_error(format!("resolving Pi RPC workspace allowlist: {error}"))
            })?;
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

#[cfg(unix)]
async fn terminate_process_tree(process_group: u32, child: &mut Child) {
    // Command::process_group(0) makes the direct child group leader. Keep the
    // group id separately because Tokio clears Child::id after wait/reap.
    unsafe {
        libc::kill(-(process_group as i32), libc::SIGKILL);
    }
    let _ = child.wait().await;
}

#[cfg(not(unix))]
async fn terminate_process_tree(_process_group: u32, child: &mut Child) {
    let _ = child.kill().await;
    let _ = child.wait().await;
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

fn command_id(agent: fabric::AgentId, sequence: u64) -> String {
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
        id: PI_RPC_RUNTIME_ID.into(),
        aliases: vec!["pi".into()],
        display_name: "Pi Coding Runtime (RPC)".into(),
        capabilities: BTreeSet::from([
            runtime::RuntimeCapability::CodeRead,
            runtime::RuntimeCapability::CodeSearch,
            runtime::RuntimeCapability::CodeEdit,
            runtime::RuntimeCapability::Shell,
            runtime::RuntimeCapability::Test,
        ]),
        interaction_modes: BTreeSet::from([
            runtime::InteractionMode::Resident,
            runtime::InteractionMode::Steering,
            runtime::InteractionMode::FollowUp,
        ]),
        workspace_mode: runtime::WorkspaceMode::Shared,
        tool_governance: runtime::ToolGovernance::Observed,
    })
}
