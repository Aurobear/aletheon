//! Bounded, cancellable Tokio command execution with Unix process-group cleanup.

use std::collections::BTreeMap;
use std::fmt;
use std::path::PathBuf;
use std::process::Stdio;
use std::time::{Duration, Instant};

use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt};
use tokio::process::Command;
use tokio_util::sync::CancellationToken;

#[derive(Debug, Clone)]
pub(crate) struct BoundedCommandRequest {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub working_dir: PathBuf,
    /// Complete environment allow-list. The child inherits nothing else.
    pub environment: BTreeMap<String, String>,
    pub stdin: Option<Vec<u8>>,
    pub timeout: Duration,
    pub stream_cap_bytes: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct BoundedCommandOutput {
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout: String,
    pub stderr: String,
    /// Exact captured bytes, retained for binary-safe git diff/artifact handling.
    pub stdout_bytes: Vec<u8>,
    pub stderr_bytes: Vec<u8>,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug)]
pub(crate) enum BoundedCommandError {
    InvalidRequest(String),
    Spawn(std::io::Error),
    Io(std::io::Error),
    Join(String),
}

impl fmt::Display for BoundedCommandError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(message) => write!(f, "invalid command request: {message}"),
            Self::Spawn(error) => write!(f, "spawning command: {error}"),
            Self::Io(error) => write!(f, "command I/O: {error}"),
            Self::Join(error) => write!(f, "command output task: {error}"),
        }
    }
}

impl std::error::Error for BoundedCommandError {}

#[derive(Debug, Clone, Default)]
pub(crate) struct BoundedCommandRunner;

impl BoundedCommandRunner {
    pub async fn run(
        &self,
        request: BoundedCommandRequest,
        cancel: CancellationToken,
    ) -> Result<BoundedCommandOutput, BoundedCommandError> {
        validate(&request)?;
        if cancel.is_cancelled() {
            return Ok(BoundedCommandOutput {
                exit_code: None,
                elapsed_ms: 0,
                timed_out: false,
                cancelled: true,
                stdout: String::new(),
                stderr: String::new(),
                stdout_bytes: vec![],
                stderr_bytes: vec![],
                stdout_truncated: false,
                stderr_truncated: false,
            });
        }

        let mut command = Command::new(&request.program);
        command
            .args(&request.args)
            .current_dir(&request.working_dir)
            .env_clear()
            .envs(&request.environment)
            .stdin(if request.stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .kill_on_drop(true);
        #[cfg(unix)]
        command.process_group(0);

        let started = Instant::now();
        let mut child = command.spawn().map_err(BoundedCommandError::Spawn)?;
        let process_id = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| BoundedCommandError::InvalidRequest("stdout pipe missing".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| BoundedCommandError::InvalidRequest("stderr pipe missing".into()))?;
        let cap = request.stream_cap_bytes;
        let stdout_task = tokio::spawn(read_capped(stdout, cap));
        let stderr_task = tokio::spawn(read_capped(stderr, cap));

        if let (Some(input), Some(mut stdin)) = (request.stdin, child.stdin.take()) {
            tokio::spawn(async move {
                let _ = stdin.write_all(&input).await;
                let _ = stdin.shutdown().await;
            });
        }

        enum Completion {
            Exited(std::process::ExitStatus),
            TimedOut,
            Cancelled,
        }
        let completion = tokio::select! {
            status = child.wait() => Completion::Exited(status.map_err(BoundedCommandError::Io)?),
            _ = tokio::time::sleep(request.timeout) => Completion::TimedOut,
            _ = cancel.cancelled() => Completion::Cancelled,
        };

        let (status, timed_out, cancelled) = match completion {
            Completion::Exited(status) => (Some(status), false, false),
            Completion::TimedOut => {
                terminate_process_group(process_id, &mut child).await;
                (child.wait().await.ok(), true, false)
            }
            Completion::Cancelled => {
                terminate_process_group(process_id, &mut child).await;
                (child.wait().await.ok(), false, true)
            }
        };

        let stdout = stdout_task
            .await
            .map_err(|error| BoundedCommandError::Join(error.to_string()))?
            .map_err(BoundedCommandError::Io)?;
        let stderr = stderr_task
            .await
            .map_err(|error| BoundedCommandError::Join(error.to_string()))?
            .map_err(BoundedCommandError::Io)?;

        let stdout_text = String::from_utf8_lossy(&stdout.bytes).into_owned();
        let stderr_text = String::from_utf8_lossy(&stderr.bytes).into_owned();
        Ok(BoundedCommandOutput {
            exit_code: status.and_then(|status| status.code()),
            elapsed_ms: started.elapsed().as_millis() as u64,
            timed_out,
            cancelled,
            stdout: stdout_text,
            stderr: stderr_text,
            stdout_bytes: stdout.bytes,
            stderr_bytes: stderr.bytes,
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
        })
    }
}

fn validate(request: &BoundedCommandRequest) -> Result<(), BoundedCommandError> {
    if request.program.as_os_str().is_empty() {
        return Err(BoundedCommandError::InvalidRequest(
            "program must not be empty".into(),
        ));
    }
    if !request.working_dir.is_dir() {
        return Err(BoundedCommandError::InvalidRequest(
            "working directory must exist".into(),
        ));
    }
    if request.timeout.is_zero() || request.stream_cap_bytes == 0 {
        return Err(BoundedCommandError::InvalidRequest(
            "timeout and stream cap must be positive".into(),
        ));
    }
    if request
        .environment
        .keys()
        .any(|key| key.is_empty() || key.contains('=') || key.contains('\0') || key.contains('/'))
    {
        return Err(BoundedCommandError::InvalidRequest(
            "invalid environment key".into(),
        ));
    }
    Ok(())
}

struct CappedBytes {
    bytes: Vec<u8>,
    truncated: bool,
}

async fn read_capped(
    mut reader: impl AsyncRead + Unpin,
    cap: usize,
) -> std::io::Result<CappedBytes> {
    let mut stored = Vec::with_capacity(cap.min(8 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    let mut truncated = false;
    loop {
        let read = reader.read(&mut buffer).await?;
        if read == 0 {
            break;
        }
        let remaining = cap.saturating_sub(stored.len());
        let keep = remaining.min(read);
        stored.extend_from_slice(&buffer[..keep]);
        truncated |= keep < read;
    }
    Ok(CappedBytes {
        bytes: stored,
        truncated,
    })
}

#[cfg(unix)]
async fn terminate_process_group(process_id: Option<u32>, child: &mut tokio::process::Child) {
    let Some(process_id) = process_id else {
        let _ = child.kill().await;
        return;
    };
    // Negative pid targets the process group established by process_group(0).
    unsafe {
        libc::kill(-(process_id as i32), libc::SIGTERM);
    }
    tokio::time::sleep(Duration::from_millis(50)).await;
    unsafe {
        libc::kill(-(process_id as i32), libc::SIGKILL);
    }
}

#[cfg(not(unix))]
async fn terminate_process_group(_process_id: Option<u32>, child: &mut tokio::process::Child) {
    let _ = child.kill().await;
}
