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

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

const MAX_RETAINED_OUTPUT_BYTES: usize = 1024 * 1024;
const DEFAULT_YIELD_MS: u64 = 1_000;
const MAX_YIELD_MS: u64 = 30_000;
const DEFAULT_TIMEOUT_SECS: u64 = 30;
const MAX_TIMEOUT_SECS: u64 = 3_600;

#[derive(Clone, Default)]
pub struct ManagedCommandSessions {
    sessions: Arc<Mutex<HashMap<String, Arc<Mutex<CommandSession>>>>>,
}

struct CommandSession {
    owner_session_id: String,
    command: String,
    cwd: String,
    purpose: CommandPurpose,
    stdin: Option<ChildStdin>,
    cancel: Option<oneshot::Sender<()>>,
    output: Vec<u8>,
    base_cursor: u64,
    delivered_cursor: u64,
    truncated: bool,
    terminal: Option<CommandTerminal>,
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
    Shell,
    Validation { validation_kind: ValidationKind },
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "snake_case")]
enum ValidationKind {
    Check,
    Test,
    Lint,
    Build,
    Deploy,
}

impl ValidationKind {
    fn parse(value: &str) -> Option<Self> {
        match value {
            "check" => Some(Self::Check),
            "test" => Some(Self::Test),
            "lint" => Some(Self::Lint),
            "build" => Some(Self::Build),
            "deploy" => Some(Self::Deploy),
            _ => None,
        }
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
    terminal: Option<CommandTerminal>,
}

impl ManagedCommandSessions {
    async fn start(
        &self,
        owner_session_id: String,
        command_text: String,
        cwd: String,
        purpose: CommandPurpose,
        timeout: Duration,
    ) -> Result<String, String> {
        let session_id = uuid::Uuid::new_v4().to_string();
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
            base_cursor: 0,
            delivered_cursor: 0,
            truncated: false,
            terminal: None,
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
        Ok(CommandSnapshot {
            session_id: session_id.to_string(),
            command: state.command.clone(),
            cwd: state.cwd.clone(),
            purpose: state.purpose.clone(),
            cursor_start,
            cursor_end,
            output,
            truncated: state.truncated,
            terminal: state.terminal.clone(),
        })
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
        "Start a managed shell command. Returns an incremental output cursor and a session_id when the command remains active; use write_stdin to poll, steer, cancel, or reap it."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {"type": "string"},
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
        match self
            .sessions
            .start(
                ctx.session_id.clone(),
                command.into(),
                cwd,
                CommandPurpose::Shell,
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
            Err(error) => tool_error(format!("failed to start command: {error}")),
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
                "validation_kind": {"type": "string", "enum": ["check", "test", "lint", "build", "deploy"]},
                "timeout_seconds": {"type": "integer", "minimum": 1, "maximum": MAX_TIMEOUT_SECS},
                "yield_time_ms": {"type": "integer", "minimum": 0, "maximum": MAX_YIELD_MS}
            },
            "required": ["command", "validation_kind"]
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
            return tool_error("validation_kind must be check, test, lint, build, or deploy");
        };
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
        match self
            .sessions
            .start(
                ctx.session_id.clone(),
                command.into(),
                cwd,
                CommandPurpose::Validation {
                    validation_kind: kind,
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
            Err(error) => tool_error(format!("failed to start validation: {error}")),
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
    );
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
        ToolContext {
            agent: None,
            approval_authority: None,
            working_dir: PathBuf::from("/tmp"),
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
        let sessions = ManagedCommandSessions::default();
        let result = ValidationRunTool::new(sessions)
            .execute(
                json!({
                    "command": "printf validated",
                    "validation_kind": "test",
                    "yield_time_ms": 20
                }),
                &context("owner"),
            )
            .await;
        let value: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(value["command"], "printf validated");
        assert_eq!(value["cwd"], "/tmp");
        assert_eq!(value["purpose"]["kind"], "validation");
        assert_eq!(value["purpose"]["validation_kind"], "test");
        assert_eq!(value["terminal"]["status"], "exited");
        assert_eq!(value["terminal"]["exit_code"], 0);
    }
}
