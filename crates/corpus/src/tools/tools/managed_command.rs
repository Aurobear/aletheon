//! Persistent, steerable command sessions for multi-turn practical work.

use std::collections::HashMap;
use std::process::Stdio;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use serde::Serialize;
use serde_json::json;
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::{ChildStdin, Command};
use tokio::sync::{oneshot, Mutex};

use super::change_transaction::ChangeTransactionRegistry;
use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use crate::security::command_effect::classify_command;

const MAX_RETAINED_OUTPUT_BYTES: usize = 1024 * 1024;
const MAX_ARTIFACT_OUTPUT_BYTES: usize = 16 * 1024 * 1024;
const DEFAULT_YIELD_MS: u64 = 1_000;
const MAX_YIELD_MS: u64 = 30_000;
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 3_600;

#[derive(Clone, Default)]
pub struct ManagedCommandSessions {
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<CommandSession>>>>>,
    change_transactions: Option<ChangeTransactionRegistry>,
}

struct CommandSession {
    owner_session_id: String,
    command: String,
    cwd: String,
    purpose: CommandPurpose,
    stdin: Option<ChildStdin>,
    cancel: Option<oneshot::Sender<()>>,
    output: Vec<u8>,
    artifact_output: Vec<u8>,
    base_cursor: u64,
    delivered_cursor: u64,
    truncated: bool,
    artifact_truncated: bool,
    terminal: Option<CommandTerminal>,
    output_artifact_ref: Option<String>,
    validation_recorded: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "status", rename_all = "snake_case")]
enum CommandTerminal {
    Exited { exit_code: Option<i32> },
    TimedOut,
    Cancelled,
    Failed { error: String },
}

#[derive(Debug, Clone, Serialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum CommandPurpose {
    Shell {
        transaction_id: Option<fabric::change_transaction::ChangeTransactionId>,
        starting_workspace_version: Option<String>,
    },
    Validation {
        validation_kind: ValidationKind,
        transaction_id: fabric::change_transaction::ChangeTransactionId,
        workspace_version: String,
    },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ValidationKind {
    Format,
    Check,
    Test,
    Lint,
    Build,
    Deploy,
}

impl ValidationKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "format" => Some(Self::Format),
            "check" => Some(Self::Check),
            "test" => Some(Self::Test),
            "lint" => Some(Self::Lint),
            "build" => Some(Self::Build),
            "deploy" => Some(Self::Deploy),
            _ => None,
        }
    }
}

fn validation_kind_name(kind: ValidationKind) -> &'static str {
    match kind {
        ValidationKind::Format => "format",
        ValidationKind::Check => "check",
        ValidationKind::Test => "test",
        ValidationKind::Lint => "lint",
        ValidationKind::Build => "build",
        ValidationKind::Deploy => "deploy",
    }
}

fn terminal_status(terminal: &CommandTerminal) -> &'static str {
    match terminal {
        CommandTerminal::Exited { exit_code: Some(0) } => "succeeded",
        CommandTerminal::Exited { .. } | CommandTerminal::Failed { .. } => "failed",
        CommandTerminal::TimedOut => "timed_out",
        CommandTerminal::Cancelled => "cancelled",
    }
}

#[derive(Serialize)]
struct CommandSnapshot {
    session_id: String,
    command: String,
    cwd: String,
    purpose: CommandPurpose,
    cursor_start: u64,
    cursor_end: u64,
    output: String,
    truncated: bool,
    artifact_truncated: bool,
    terminal: Option<CommandTerminal>,
    output_artifact_ref: Option<String>,
    change_transaction: Option<fabric::change_transaction::ChangeTransactionSnapshot>,
}

impl ManagedCommandSessions {
    pub fn with_change_transactions(change_transactions: ChangeTransactionRegistry) -> Self {
        Self {
            sessions: Arc::new(Mutex::new(HashMap::new())),
            change_transactions: Some(change_transactions),
        }
    }

    async fn start(
        &self,
        session_id: String,
        owner_session_id: String,
        command_text: String,
        cwd: String,
        purpose: CommandPurpose,
        timeout: Duration,
    ) -> Result<String, String> {
        let mut command = Command::new("bash");
        command
            .arg("-c")
            .arg(&command_text)
            .current_dir(&cwd)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        let mut child = command.spawn().map_err(|error| error.to_string())?;
        let stdin = child.stdin.take();
        let stdout = child.stdout.take();
        let stderr = child.stderr.take();
        let (cancel_tx, mut cancel_rx) = oneshot::channel();
        let session = Arc::new(Mutex::new(CommandSession {
            owner_session_id,
            command: command_text,
            cwd,
            purpose,
            stdin,
            cancel: Some(cancel_tx),
            output: Vec::new(),
            artifact_output: Vec::new(),
            base_cursor: 0,
            delivered_cursor: 0,
            truncated: false,
            artifact_truncated: false,
            terminal: None,
            output_artifact_ref: None,
            validation_recorded: false,
        }));
        self.sessions
            .lock()
            .await
            .insert(session_id.clone(), session.clone());

        let stdout_task =
            stdout.map(|stdout| tokio::spawn(capture_stream(stdout, session.clone(), b"")));
        let stderr_task = stderr
            .map(|stderr| tokio::spawn(capture_stream(stderr, session.clone(), b"[stderr] ")));
        tokio::spawn(async move {
            let terminal = tokio::select! {
                status = child.wait() => match status {
                    Ok(status) => CommandTerminal::Exited { exit_code: status.code() },
                    Err(error) => CommandTerminal::Failed { error: error.to_string() },
                },
                _ = tokio::time::sleep(timeout) => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    CommandTerminal::TimedOut
                },
                _ = &mut cancel_rx => {
                    let _ = child.kill().await;
                    let _ = child.wait().await;
                    CommandTerminal::Cancelled
                }
            };
            if let Some(task) = stdout_task {
                let _ = task.await;
            }
            if let Some(task) = stderr_task {
                let _ = task.await;
            }
            let mut state = session.lock().await;
            let store = crate::tools::artifact::ArtifactStore::new(
                super::output::OutputConfig::default()
                    .overflow_dir
                    .join("artifacts"),
            );
            state.output_artifact_ref = store
                .store(&state.artifact_output, "text/plain; charset=utf-8")
                .ok()
                .map(|artifact| artifact.uri());
            state.stdin = None;
            state.cancel = None;
            state.terminal = Some(terminal);
        });
        Ok(session_id)
    }

    async fn snapshot(
        &self,
        session_id: &str,
        owner_session_id: &str,
        wait: Duration,
    ) -> Result<CommandSnapshot, String> {
        let session = self.session(session_id, owner_session_id).await?;
        if !wait.is_zero() && session.lock().await.terminal.is_none() {
            tokio::time::sleep(wait).await;
        }
        let mut state = session.lock().await;
        let cursor_start = state.delivered_cursor.max(state.base_cursor);
        let relative = cursor_start.saturating_sub(state.base_cursor) as usize;
        let output =
            String::from_utf8_lossy(state.output.get(relative..).unwrap_or_default()).into_owned();
        let cursor_end = state.base_cursor + state.output.len() as u64;
        state.delivered_cursor = cursor_end;
        let validation = match (&state.purpose, &state.terminal) {
            (
                CommandPurpose::Validation {
                    validation_kind,
                    transaction_id,
                    workspace_version,
                },
                Some(terminal),
            ) if !state.validation_recorded => Some((
                *validation_kind,
                *transaction_id,
                workspace_version.clone(),
                state.command.clone(),
                state.cwd.clone(),
                if state.artifact_truncated {
                    "truncated".to_string()
                } else {
                    terminal_status(terminal).to_string()
                },
                state.output_artifact_ref.clone(),
            )),
            _ => None,
        };
        let shell_change = match (&state.purpose, &state.terminal) {
            (
                CommandPurpose::Shell {
                    transaction_id: Some(transaction_id),
                    starting_workspace_version: Some(starting_workspace_version),
                },
                Some(_),
            ) if !state.validation_recorded => Some((
                *transaction_id,
                starting_workspace_version.clone(),
                state.cwd.clone(),
                state
                    .terminal
                    .as_ref()
                    .map(terminal_status)
                    .unwrap_or("failed")
                    .to_string(),
            )),
            _ => None,
        };
        if validation.is_some() || shell_change.is_some() {
            state.validation_recorded = true;
        }
        let purpose = state.purpose.clone();
        let mut snapshot = CommandSnapshot {
            session_id: session_id.to_string(),
            command: state.command.clone(),
            cwd: state.cwd.clone(),
            purpose: purpose.clone(),
            cursor_start,
            cursor_end,
            output,
            truncated: state.truncated,
            artifact_truncated: state.artifact_truncated,
            terminal: state.terminal.clone(),
            output_artifact_ref: state.output_artifact_ref.clone(),
            change_transaction: None,
        };
        drop(state);
        if let Some((kind, transaction_id, version, command, cwd, status, output_ref)) = validation
        {
            if let Some(registry) = &self.change_transactions {
                snapshot.change_transaction =
                    match super::workspace_version::capture(std::path::Path::new(&cwd)) {
                        Ok(observed) => registry
                            .record_validation(
                                transaction_id,
                                validation_kind_name(kind).into(),
                                command,
                                version,
                                observed,
                                status,
                                output_ref,
                                session_id,
                            )
                            .await
                            .ok(),
                        Err(_) => registry
                            .release_command(transaction_id, session_id)
                            .await
                            .ok(),
                    };
            }
        } else if let Some((transaction_id, starting_version, cwd, terminal_status)) = shell_change
        {
            if let Some(registry) = &self.change_transactions {
                snapshot.change_transaction =
                    match super::workspace_version::capture(std::path::Path::new(&cwd)) {
                        Ok(current) if current.digest != starting_version => registry
                            .record_shell_terminal(
                                transaction_id,
                                session_id,
                                current,
                                &terminal_status,
                            )
                            .await
                            .ok(),
                        Ok(_) | Err(_) => registry
                            .release_command(transaction_id, session_id)
                            .await
                            .ok(),
                    };
            }
        } else if let CommandPurpose::Validation { transaction_id, .. } = purpose {
            if let Some(registry) = &self.change_transactions {
                snapshot.change_transaction = registry.snapshot(transaction_id).await;
            }
        } else if let CommandPurpose::Shell {
            transaction_id: Some(transaction_id),
            ..
        } = purpose
        {
            if let Some(registry) = &self.change_transactions {
                snapshot.change_transaction = registry.snapshot(transaction_id).await;
            }
        }
        Ok(snapshot)
    }

    async fn write(
        &self,
        session_id: &str,
        owner_session_id: &str,
        chars: &[u8],
    ) -> Result<(), String> {
        let session = self.session(session_id, owner_session_id).await?;
        let mut stdin = {
            let mut state = session.lock().await;
            if state.terminal.is_some() {
                return Err("command session is already terminal".into());
            }
            state
                .stdin
                .take()
                .ok_or_else(|| "command session stdin is unavailable".to_string())?
        };
        let result = stdin
            .write_all(chars)
            .await
            .map_err(|error| error.to_string());
        session.lock().await.stdin = Some(stdin);
        result
    }

    async fn cancel(&self, session_id: &str, owner_session_id: &str) -> Result<(), String> {
        let session = self.session(session_id, owner_session_id).await?;
        let sender = session.lock().await.cancel.take();
        match sender {
            Some(sender) => sender
                .send(())
                .map_err(|_| "command cancellation receiver is unavailable".to_string()),
            None => Ok(()),
        }
    }

    async fn reap(&self, session_id: &str, owner_session_id: &str) -> Result<(), String> {
        let session = self.session(session_id, owner_session_id).await?;
        if session.lock().await.terminal.is_none() {
            return Err("cannot reap a running command session".into());
        }
        self.sessions.lock().await.remove(session_id);
        Ok(())
    }

    async fn session(
        &self,
        session_id: &str,
        owner_session_id: &str,
    ) -> Result<Arc<Mutex<CommandSession>>, String> {
        let session = self
            .sessions
            .lock()
            .await
            .get(session_id)
            .cloned()
            .ok_or_else(|| format!("unknown command session: {session_id}"))?;
        if session.lock().await.owner_session_id != owner_session_id {
            return Err("command session belongs to a different authenticated session".into());
        }
        Ok(session)
    }
}

async fn capture_stream(
    mut reader: impl AsyncRead + Unpin,
    session: Arc<Mutex<CommandSession>>,
    prefix: &'static [u8],
) {
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        let read = match reader.read(&mut buffer).await {
            Ok(0) | Err(_) => break,
            Ok(read) => read,
        };
        let mut state = session.lock().await;
        if !state.artifact_truncated {
            let incoming = prefix.len().saturating_add(read);
            let remaining = MAX_ARTIFACT_OUTPUT_BYTES.saturating_sub(state.artifact_output.len());
            if incoming > remaining {
                if remaining > 0 {
                    let mut chunk = Vec::with_capacity(incoming);
                    chunk.extend_from_slice(prefix);
                    chunk.extend_from_slice(&buffer[..read]);
                    state.artifact_output.extend_from_slice(&chunk[..remaining]);
                }
                state.artifact_truncated = true;
            } else {
                state.artifact_output.extend_from_slice(prefix);
                state.artifact_output.extend_from_slice(&buffer[..read]);
            }
        }
        if !prefix.is_empty() {
            state.output.extend_from_slice(prefix);
        }
        state.output.extend_from_slice(&buffer[..read]);
        if state.output.len() > MAX_RETAINED_OUTPUT_BYTES {
            let remove = state.output.len() - MAX_RETAINED_OUTPUT_BYTES;
            state.output.drain(..remove);
            state.base_cursor = state.base_cursor.saturating_add(remove as u64);
            state.truncated = true;
        }
    }
}

#[derive(Clone)]
pub struct ExecCommandTool {
    sessions: ManagedCommandSessions,
}

impl ExecCommandTool {
    pub fn new(sessions: ManagedCommandSessions) -> Self {
        Self { sessions }
    }
}

#[async_trait]
impl Tool for ExecCommandTool {
    fn name(&self) -> &str {
        "exec_command"
    }

    fn description(&self) -> &str {
        "Start a managed shell command only when no dedicated tool can perform the operation. For repository overview, prefer repo_inspect, batched file_read, scoped glob, git_status, and git_log; do not build, test, or count an inventory merely to infer maturity. Returns an incremental output cursor and a session_id when the command remains active; use write_stdin to poll, steer, cancel, or reap it."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string","description":"Shell command. System package, service, and privilege changes are denied by the production host; report that boundary instead of retrying or requesting repo_inspect."},
                "transaction_id": {"type":"string","description":"Required only for commands the host classifies as mutating; host-minted by repo_inspect"},
                "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_SECS},
                "yield_time_ms": {"type": "integer", "minimum": 0, "maximum": MAX_YIELD_MS}
            },
            "required": ["command"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let Some(command) = input.get("command").and_then(|value| value.as_str()) else {
            return tool_error("exec_command requires a non-empty command");
        };
        if command.trim().is_empty() {
            return tool_error("exec_command requires a non-empty command");
        }
        let timeout = input
            .get("timeout_seconds")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS);
        let yield_ms = input
            .get("yield_time_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_YIELD_MS)
            .min(MAX_YIELD_MS);
        let cwd = ctx.working_dir.to_string_lossy().into_owned();
        let command_session_id = uuid::Uuid::new_v4().to_string();
        let effect = classify_command(command);
        let unrestricted = ctx
            .approval_authority
            .as_ref()
            .map(|authority| authority.permission_mode.is_full())
            .unwrap_or(false);
        let transaction = if let Some(registry) = &self.sessions.change_transactions {
            if effect.requires_transaction() && !unrestricted {
                let Some(transaction_id) = input
                    .get("transaction_id")
                    .and_then(|value| value.as_str())
                    .and_then(|value| uuid::Uuid::parse_str(value).ok())
                    .map(fabric::change_transaction::ChangeTransactionId)
                else {
                    return tool_error(
                        "mutating exec_command requires a valid transaction_id from repo_inspect in the production runtime",
                    );
                };
                match registry
                    .reserve_command(
                        transaction_id,
                        &ctx.session_id,
                        ctx.agent.clone(),
                        &ctx.working_dir,
                        command_session_id.clone(),
                        "shell".into(),
                    )
                    .await
                {
                    Ok(snapshot) => Some((transaction_id, snapshot.current.digest)),
                    Err(failure) => {
                        return json_result(
                            json!({"kind":"change_transaction_error", "recovery":failure.recovery(), "failure":failure}),
                            true,
                            false,
                        )
                    }
                }
            } else {
                None
            }
        } else {
            None
        };
        match self
            .sessions
            .start(
                command_session_id.clone(),
                ctx.session_id.clone(),
                command.into(),
                cwd,
                CommandPurpose::Shell {
                    transaction_id: transaction.as_ref().map(|value| value.0),
                    starting_workspace_version: transaction.as_ref().map(|value| value.1.clone()),
                },
                Duration::from_secs(timeout),
            )
            .await
        {
            Ok(session_id) => match self
                .sessions
                .snapshot(
                    &session_id,
                    &ctx.session_id,
                    Duration::from_millis(yield_ms),
                )
                .await
            {
                Ok(snapshot) => snapshot_result(snapshot),
                Err(error) => tool_error(error),
            },
            Err(error) => {
                if let (Some(registry), Some((transaction_id, _))) =
                    (&self.sessions.change_transactions, transaction)
                {
                    let _ = registry
                        .release_command(transaction_id, &command_session_id)
                        .await;
                }
                tool_error(format!("failed to start command: {error}"))
            }
        }
    }
}

#[derive(Clone)]
pub struct ValidationRunTool {
    sessions: ManagedCommandSessions,
}

impl ValidationRunTool {
    pub fn new(sessions: ManagedCommandSessions) -> Self {
        Self { sessions }
    }
}

#[async_trait]
impl Tool for ValidationRunTool {
    fn name(&self) -> &str {
        "validation_run"
    }

    fn description(&self) -> &str {
        "Run an exact repository validation command through the governed command runtime. Classifies check/test/lint/build/deploy and preserves cwd, output cursors, and terminal status."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string", "description": "Exact command selected from applicable repository instructions or validation policy"},
                "validation_kind": {"type": "string", "enum": ["format", "check", "test", "lint", "build", "deploy"]},
                "transaction_id": {"type": "string", "description": "Host-minted transaction_id whose reviewed workspace version is being validated"},
                "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_SECS},
                "yield_time_ms": {"type": "integer", "minimum": 0, "maximum": MAX_YIELD_MS}
            },
            "required": ["command", "validation_kind", "transaction_id"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let Some(command) = input.get("command").and_then(|value| value.as_str()) else {
            return tool_error("validation_run requires a non-empty command");
        };
        if command.trim().is_empty() {
            return tool_error("validation_run requires a non-empty command");
        }
        let Some(kind) = input
            .get("validation_kind")
            .and_then(|value| value.as_str())
            .and_then(ValidationKind::parse)
        else {
            return tool_error(
                "validation_kind must be format, check, test, lint, build, or deploy",
            );
        };
        let Some(transaction_id) = input
            .get("transaction_id")
            .and_then(|value| value.as_str())
            .and_then(|value| uuid::Uuid::parse_str(value).ok())
            .map(fabric::change_transaction::ChangeTransactionId)
        else {
            return tool_error("validation_run requires a valid transaction_id from repo_inspect");
        };
        let Some(registry) = &self.sessions.change_transactions else {
            return tool_error("validation_run transaction registry is unavailable");
        };
        let transaction = match registry
            .verify_current(
                transaction_id,
                &ctx.session_id,
                ctx.agent.clone(),
                &ctx.working_dir,
            )
            .await
        {
            Ok(snapshot)
                if snapshot.phase
                    == fabric::change_transaction::ChangeTransactionPhase::DiffReviewed =>
            {
                snapshot
            }
            Ok(_) => return tool_error("validation_run requires a reviewed transaction diff"),
            Err(failure) => {
                return json_result(
                    json!({"kind":"change_transaction_error", "recovery":failure.recovery(), "failure":failure}),
                    true,
                    false,
                )
            }
        };
        let Some(planned_step) = transaction.validation_plan.iter().find(|step| {
            step.command == command && step.validation_kind == validation_kind_name(kind)
        }) else {
            return json_result(
                json!({
                    "kind":"change_transaction_error",
                    "recovery":"correct_arguments",
                    "failure": {
                        "class":"invalid_tool_request",
                        "summary":"command and validation_kind must match a host-derived validation plan step",
                        "retryable":true
                    },
                    "validation_plan": transaction.validation_plan,
                }),
                true,
                false,
            );
        };
        if transaction.validation_receipts.iter().any(|receipt| {
            receipt.command == planned_step.command
                && receipt.validation_kind == planned_step.validation_kind
                && receipt.workspace_version == transaction.current.digest
                && receipt.terminal_status == "succeeded"
        }) {
            return tool_error(
                "validation plan step already has a successful receipt for this version",
            );
        }
        let timeout = input
            .get("timeout_seconds")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_TIMEOUT_SECS)
            .clamp(1, MAX_TIMEOUT_SECS);
        let yield_ms = input
            .get("yield_time_ms")
            .and_then(|value| value.as_u64())
            .unwrap_or(DEFAULT_YIELD_MS)
            .min(MAX_YIELD_MS);
        let cwd = ctx.working_dir.to_string_lossy().into_owned();
        let command_session_id = uuid::Uuid::new_v4().to_string();
        let transaction = match registry
            .reserve_command(
                transaction_id,
                &ctx.session_id,
                ctx.agent.clone(),
                &ctx.working_dir,
                command_session_id.clone(),
                "validation".into(),
            )
            .await
        {
            Ok(snapshot) => snapshot,
            Err(failure) => {
                return json_result(
                    json!({"kind":"change_transaction_error", "recovery":failure.recovery(), "failure":failure}),
                    true,
                    false,
                )
            }
        };
        match self
            .sessions
            .start(
                command_session_id.clone(),
                ctx.session_id.clone(),
                command.into(),
                cwd,
                CommandPurpose::Validation {
                    validation_kind: kind,
                    transaction_id,
                    workspace_version: transaction.current.digest,
                },
                Duration::from_secs(timeout),
            )
            .await
        {
            Ok(session_id) => match self
                .sessions
                .snapshot(
                    &session_id,
                    &ctx.session_id,
                    Duration::from_millis(yield_ms),
                )
                .await
            {
                Ok(snapshot) => snapshot_result(snapshot),
                Err(error) => tool_error(error),
            },
            Err(error) => {
                let _ = registry
                    .release_command(transaction_id, &command_session_id)
                    .await;
                tool_error(format!("failed to start validation: {error}"))
            }
        }
    }
}

#[derive(Clone)]
pub struct WriteStdinTool {
    sessions: ManagedCommandSessions,
}

impl WriteStdinTool {
    pub fn new(sessions: ManagedCommandSessions) -> Self {
        Self { sessions }
    }
}

#[async_trait]
impl Tool for WriteStdinTool {
    fn name(&self) -> &str {
        "write_stdin"
    }
    fn description(&self) -> &str {
        "Poll or steer an existing exec_command session. Supports writing characters, cancellation, terminal observation, and explicit reaping."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "session_id": {"type": "string"},
                "chars": {"type": "string"},
                "yield_time_ms": {"type": "integer", "minimum": 0, "maximum": MAX_YIELD_MS},
                "cancel": {"type": "boolean"},
                "reap": {"type": "boolean"}
            },
            "required": ["session_id"]
        })
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(self.clone())
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let Some(session_id) = input.get("session_id").and_then(|v| v.as_str()) else {
            return tool_error("write_stdin requires session_id");
        };
        if input.get("reap").and_then(|v| v.as_bool()).unwrap_or(false) {
            return match self.sessions.reap(session_id, &ctx.session_id).await {
                Ok(()) => json_result(
                    json!({"session_id": session_id, "reaped": true}),
                    false,
                    false,
                ),
                Err(error) => tool_error(error),
            };
        }
        if let Some(chars) = input.get("chars").and_then(|v| v.as_str()) {
            if let Err(error) = self
                .sessions
                .write(session_id, &ctx.session_id, chars.as_bytes())
                .await
            {
                return tool_error(error);
            }
        }
        if input
            .get("cancel")
            .and_then(|v| v.as_bool())
            .unwrap_or(false)
        {
            if let Err(error) = self.sessions.cancel(session_id, &ctx.session_id).await {
                return tool_error(error);
            }
        }
        let yield_ms = input
            .get("yield_time_ms")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_YIELD_MS)
            .min(MAX_YIELD_MS);
        match self
            .sessions
            .snapshot(session_id, &ctx.session_id, Duration::from_millis(yield_ms))
            .await
        {
            Ok(snapshot) => snapshot_result(snapshot),
            Err(error) => tool_error(error),
        }
    }
}

fn snapshot_result(snapshot: CommandSnapshot) -> ToolResult {
    let is_error = matches!(
        &snapshot.terminal,
        Some(CommandTerminal::Exited { exit_code }) if exit_code.is_none_or(|code| code != 0)
    ) || matches!(
        &snapshot.terminal,
        Some(
            CommandTerminal::TimedOut | CommandTerminal::Cancelled | CommandTerminal::Failed { .. }
        )
    ) || snapshot
        .change_transaction
        .as_ref()
        .is_some_and(|transaction| {
            matches!(
                transaction.phase,
                fabric::change_transaction::ChangeTransactionPhase::Repair
                    | fabric::change_transaction::ChangeTransactionPhase::Conflicted
            )
        });
    let truncated = snapshot.truncated;
    json_result(
        serde_json::to_value(snapshot).expect("command snapshot serializes"),
        is_error,
        truncated,
    )
}

fn json_result(value: serde_json::Value, is_error: bool, truncated: bool) -> ToolResult {
    ToolResult {
        content: serde_json::to_string_pretty(&value).expect("JSON value serializes"),
        is_error,
        metadata: ToolResultMeta {
            execution_time_ms: 0,
            truncated,
            patch_delta: None,
        },
    }
}

fn tool_error(message: impl Into<String>) -> ToolResult {
    json_result(json!({"error": message.into()}), true, false)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    fn context(owner: &str) -> ToolContext {
        context_at(owner, PathBuf::from("/tmp"))
    }

    fn context_at(owner: &str, working_dir: PathBuf) -> ToolContext {
        ToolContext {
            agent: None,
            approval_authority: None,
            working_dir,
            session_id: owner.into(),
            clock: Arc::new(kernel::chronos::SystemClock::new()),
            turn_event_sender: None,
        }
    }

    async fn poll_terminal(tool: &WriteStdinTool, id: &str, owner: &str) -> serde_json::Value {
        for _ in 0..20 {
            let result = tool
                .execute(
                    json!({"session_id": id, "yield_time_ms": 20}),
                    &context(owner),
                )
                .await;
            let value: serde_json::Value = serde_json::from_str(&result.content).unwrap();
            if !value["terminal"].is_null() {
                return value;
            }
        }
        panic!("command did not reach a terminal snapshot");
    }

    #[tokio::test]
    async fn command_can_be_polled_to_authoritative_terminal_snapshot() {
        let sessions = ManagedCommandSessions::default();
        let start = ExecCommandTool::new(sessions.clone())
            .execute(
                json!({"command": "printf first; sleep 0.05; printf second", "yield_time_ms": 0}),
                &context("owner"),
            )
            .await;
        let start: serde_json::Value = serde_json::from_str(&start.content).unwrap();
        let id = start["session_id"].as_str().unwrap();
        let terminal = poll_terminal(&WriteStdinTool::new(sessions), id, "owner").await;
        assert_eq!(terminal["terminal"]["status"], "exited");
        assert_eq!(terminal["terminal"]["exit_code"], 0);
        assert!(terminal["output"].as_str().unwrap().contains("second"));
        assert!(
            terminal["cursor_end"].as_u64().unwrap() >= terminal["cursor_start"].as_u64().unwrap()
        );
    }

    #[tokio::test]
    async fn command_session_is_owner_scoped_and_reapable() {
        let sessions = ManagedCommandSessions::default();
        let start = ExecCommandTool::new(sessions.clone())
            .execute(
                json!({"command": "true", "yield_time_ms": 50}),
                &context("owner"),
            )
            .await;
        let start: serde_json::Value = serde_json::from_str(&start.content).unwrap();
        let id = start["session_id"].as_str().unwrap();
        let denied = WriteStdinTool::new(sessions.clone())
            .execute(json!({"session_id": id}), &context("other"))
            .await;
        assert!(denied.is_error);
        let tool = WriteStdinTool::new(sessions);
        let _ = poll_terminal(&tool, id, "owner").await;
        let reaped = tool
            .execute(json!({"session_id": id, "reap": true}), &context("owner"))
            .await;
        assert!(!reaped.is_error, "{}", reaped.content);
    }

    #[tokio::test]
    async fn running_command_can_be_cancelled_and_observed_terminally() {
        let sessions = ManagedCommandSessions::default();
        let start = ExecCommandTool::new(sessions.clone())
            .execute(
                json!({"command": "sleep 30", "yield_time_ms": 0}),
                &context("owner"),
            )
            .await;
        let start: serde_json::Value = serde_json::from_str(&start.content).unwrap();
        let id = start["session_id"].as_str().unwrap();
        let tool = WriteStdinTool::new(sessions);
        let cancel = tool
            .execute(
                json!({"session_id": id, "cancel": true, "yield_time_ms": 20}),
                &context("owner"),
            )
            .await;
        assert!(!cancel.content.is_empty());
        let terminal = poll_terminal(&tool, id, "owner").await;
        assert_eq!(terminal["terminal"]["status"], "cancelled");
    }

    #[tokio::test]
    async fn running_command_accepts_incremental_stdin() {
        let sessions = ManagedCommandSessions::default();
        let start = ExecCommandTool::new(sessions.clone())
            .execute(
                json!({"command": "read line; printf 'received:%s' \"$line\"", "yield_time_ms": 0}),
                &context("owner"),
            )
            .await;
        let start: serde_json::Value = serde_json::from_str(&start.content).unwrap();
        let id = start["session_id"].as_str().unwrap();
        let tool = WriteStdinTool::new(sessions);
        let write = tool
            .execute(
                json!({"session_id": id, "chars": "hello\n", "yield_time_ms": 20}),
                &context("owner"),
            )
            .await;
        assert!(!write.is_error, "{}", write.content);
        let write: serde_json::Value = serde_json::from_str(&write.content).unwrap();
        let terminal = poll_terminal(&tool, id, "owner").await;
        assert_eq!(terminal["terminal"]["status"], "exited");
        let observed = format!(
            "{}{}",
            write["output"].as_str().unwrap_or_default(),
            terminal["output"].as_str().unwrap_or_default()
        );
        assert!(observed.contains("received:hello"));
    }

    #[tokio::test]
    async fn validation_preserves_kind_command_and_terminal_result() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("input.txt"), "stable").unwrap();
        let registry = ChangeTransactionRegistry::default();
        let transaction = registry.begin("owner", temp.path()).await.unwrap();
        registry
            .record_apply(transaction.transaction_id, transaction.current.clone())
            .await
            .unwrap();
        registry
            .record_diff_review(
                transaction.transaction_id,
                "artifact://sha256/test-diff".into(),
                Vec::new(),
            )
            .await
            .unwrap();
        registry
            .set_validation_plan(
                transaction.transaction_id,
                vec![fabric::change_transaction::ValidationPlanStep {
                    id: "focused-test".into(),
                    validation_kind: "test".into(),
                    command: "printf validated".into(),
                    reason: "test fixture".into(),
                    source: "test".into(),
                    required: true,
                }],
            )
            .await;
        let sessions = ManagedCommandSessions::with_change_transactions(registry.clone());
        let result = ValidationRunTool::new(sessions)
            .execute(
                json!({
                    "command": "printf validated",
                    "validation_kind": "test",
                    "transaction_id": transaction.transaction_id.0,
                    "yield_time_ms": 20
                }),
                &context_at("owner", temp.path().to_path_buf()),
            )
            .await;
        let value: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["command"], "printf validated");
        assert_eq!(value["cwd"], temp.path().display().to_string());
        assert_eq!(value["purpose"]["kind"], "validation");
        assert_eq!(value["purpose"]["validation_kind"], "test");
        assert_eq!(value["terminal"]["status"], "exited");
        assert_eq!(value["terminal"]["exit_code"], 0);
        assert_eq!(value["change_transaction"]["phase"], "validated");
        assert_eq!(
            value["change_transaction"]["validation_receipts"][0]["workspace_version"],
            transaction.current.digest
        );
        assert!(value["output_artifact_ref"]
            .as_str()
            .is_some_and(|value| value.starts_with("artifact://sha256/")));
    }

    #[tokio::test]
    async fn production_exec_command_requires_transaction_and_records_workspace_change() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("input.txt"), "baseline").unwrap();
        let registry = ChangeTransactionRegistry::default();
        let transaction = registry.begin("owner", temp.path()).await.unwrap();
        let sessions = ManagedCommandSessions::with_change_transactions(registry.clone());
        let tool = ExecCommandTool::new(sessions);
        let ctx = context_at("owner", temp.path().to_path_buf());

        let missing = tool
            .execute(json!({"command":"true", "yield_time_ms":20}), &ctx)
            .await;
        assert!(missing.is_error);

        let result = tool
            .execute(
                json!({
                    "command":"printf generated > generated.txt",
                    "transaction_id": transaction.transaction_id.0,
                    "yield_time_ms":20
                }),
                &ctx,
            )
            .await;
        assert!(!result.is_error, "{}", result.content);
        let value: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["terminal"]["exit_code"], 0);
        assert_eq!(value["change_transaction"]["phase"], "applied");
        assert_ne!(
            value["change_transaction"]["current"]["digest"],
            transaction.baseline.digest
        );
    }

    #[tokio::test]
    async fn production_read_only_exec_command_bypasses_change_transaction() {
        let temp = tempfile::tempdir().unwrap();
        let registry = ChangeTransactionRegistry::default();
        let tool = ExecCommandTool::new(ManagedCommandSessions::with_change_transactions(registry));
        let result = tool
            .execute(
                json!({"command":"which sh && sh --version", "yield_time_ms":1000}),
                &context_at("observed-session", temp.path().to_path_buf()),
            )
            .await;

        let value: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_ne!(
            value.get("error").and_then(serde_json::Value::as_str),
            Some(
                "mutating exec_command requires a valid transaction_id from repo_inspect in the production runtime"
            )
        );
        assert!(value["change_transaction"].is_null());
    }

    #[tokio::test]
    async fn transaction_lease_rejects_overlapping_managed_commands_until_terminal_poll() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("input.txt"), "baseline").unwrap();
        let registry = ChangeTransactionRegistry::default();
        let transaction = registry.begin("owner", temp.path()).await.unwrap();
        let sessions = ManagedCommandSessions::with_change_transactions(registry);
        let exec = ExecCommandTool::new(sessions.clone());
        let ctx = context_at("owner", temp.path().to_path_buf());
        let first = exec
            .execute(
                json!({
                    "command":"sleep 0.15",
                    "transaction_id":transaction.transaction_id.0,
                    "yield_time_ms":0
                }),
                &ctx,
            )
            .await;
        assert!(!first.is_error, "{}", first.content);
        let first: serde_json::Value = serde_json::from_str(&first.content).unwrap();
        assert!(first["terminal"].is_null());

        let overlap = exec
            .execute(
                json!({
                    "command":"true",
                    "transaction_id":transaction.transaction_id.0,
                    "yield_time_ms":0
                }),
                &ctx,
            )
            .await;
        assert!(overlap.is_error);
        let overlap: serde_json::Value = serde_json::from_str(&overlap.content).unwrap();
        assert_eq!(overlap["failure"]["class"], "concurrent_modification");

        tokio::time::sleep(Duration::from_millis(200)).await;
        let terminal = WriteStdinTool::new(sessions)
            .execute(
                json!({"session_id":first["session_id"], "yield_time_ms":20}),
                &ctx,
            )
            .await;
        assert!(!terminal.is_error, "{}", terminal.content);
        let terminal: serde_json::Value = serde_json::from_str(&terminal.content).unwrap();
        assert_eq!(terminal["terminal"]["exit_code"], 0);
        assert!(terminal["change_transaction"]["active_command"].is_null());
    }

    #[tokio::test]
    async fn validation_rejects_command_outside_derived_plan() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("input.txt"), "stable").unwrap();
        let registry = ChangeTransactionRegistry::default();
        let transaction = registry.begin("owner", temp.path()).await.unwrap();
        registry
            .record_apply(transaction.transaction_id, transaction.current.clone())
            .await
            .unwrap();
        registry
            .record_diff_review(
                transaction.transaction_id,
                "artifact://sha256/test-diff".into(),
                Vec::new(),
            )
            .await
            .unwrap();
        registry
            .set_validation_plan(
                transaction.transaction_id,
                vec![fabric::change_transaction::ValidationPlanStep {
                    id: "planned".into(),
                    validation_kind: "test".into(),
                    command: "true".into(),
                    reason: "fixture".into(),
                    source: "test".into(),
                    required: true,
                }],
            )
            .await;
        let sessions = ManagedCommandSessions::with_change_transactions(registry);
        let result = ValidationRunTool::new(sessions)
            .execute(
                json!({
                    "command":"false",
                    "validation_kind":"test",
                    "transaction_id":transaction.transaction_id.0
                }),
                &context_at("owner", temp.path().to_path_buf()),
            )
            .await;
        assert!(result.is_error);
        let value: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["failure"]["class"], "invalid_tool_request");
        assert_eq!(value["validation_plan"][0]["command"], "true");
    }

    #[tokio::test]
    async fn validation_that_mutates_reviewed_workspace_is_conflicted_not_accepted() {
        let temp = tempfile::tempdir().unwrap();
        std::fs::write(temp.path().join("input.txt"), "reviewed").unwrap();
        let registry = ChangeTransactionRegistry::default();
        let transaction = registry.begin("owner", temp.path()).await.unwrap();
        registry
            .record_apply(transaction.transaction_id, transaction.current.clone())
            .await
            .unwrap();
        registry
            .record_diff_review(
                transaction.transaction_id,
                "artifact://sha256/test-diff".into(),
                Vec::new(),
            )
            .await
            .unwrap();
        let command = "printf mutated > input.txt";
        registry
            .set_validation_plan(
                transaction.transaction_id,
                vec![fabric::change_transaction::ValidationPlanStep {
                    id: "must-not-mutate".into(),
                    validation_kind: "test".into(),
                    command: command.into(),
                    reason: "fixture".into(),
                    source: "test".into(),
                    required: true,
                }],
            )
            .await;
        let result = ValidationRunTool::new(ManagedCommandSessions::with_change_transactions(
            registry.clone(),
        ))
        .execute(
            json!({
                "command":command,
                "validation_kind":"test",
                "transaction_id":transaction.transaction_id.0,
                "yield_time_ms":30
            }),
            &context_at("owner", temp.path().to_path_buf()),
        )
        .await;
        assert!(result.is_error, "{}", result.content);
        let value: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["change_transaction"]["phase"], "conflicted");
        assert_eq!(
            value["change_transaction"]["failure"]["class"],
            "concurrent_modification"
        );
    }

    #[tokio::test]
    async fn terminal_artifact_preserves_output_beyond_incremental_preview_limit() {
        let sessions = ManagedCommandSessions::default();
        let result = ExecCommandTool::new(sessions)
            .execute(
                json!({
                    "command":"python3 -c 'import sys; sys.stdout.write(\"x\" * 1100000)'",
                    "yield_time_ms":1000,
                    "timeout_seconds":10
                }),
                &context("owner"),
            )
            .await;
        assert!(!result.is_error, "{}", result.content);
        let value: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["terminal"]["exit_code"], 0);
        assert_eq!(value["truncated"], true);
        assert_eq!(value["artifact_truncated"], false);
        assert!(value["output_artifact_ref"]
            .as_str()
            .is_some_and(|value| value.starts_with("artifact://sha256/")));
    }
}
