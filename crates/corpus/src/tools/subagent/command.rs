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
pub struct CommandRequest {
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
pub struct CommandOutput {
    pub exit_code: Option<i32>,
    pub elapsed_ms: u64,
    pub timed_out: bool,
    pub cancelled: bool,
    pub stdout: String,
    pub stderr: String,
    pub stdout_truncated: bool,
    pub stderr_truncated: bool,
}

#[derive(Debug)]
pub enum CommandRunnerError {
    InvalidRequest(String),
    Spawn(std::io::Error),
    Io(std::io::Error),
    Join(String),
}

impl fmt::Display for CommandRunnerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRequest(message) => write!(f, "invalid command request: {message}"),
            Self::Spawn(error) => write!(f, "spawning command: {error}"),
            Self::Io(error) => write!(f, "command I/O: {error}"),
            Self::Join(error) => write!(f, "command output task: {error}"),
        }
    }
}

impl std::error::Error for CommandRunnerError {}

#[derive(Debug, Clone, Default)]
pub struct CommandRunner;

impl CommandRunner {
    pub async fn run(
        &self,
        request: CommandRequest,
        cancel: CancellationToken,
    ) -> Result<CommandOutput, CommandRunnerError> {
        validate(&request)?;
        if cancel.is_cancelled() {
            return Ok(CommandOutput {
                exit_code: None,
                elapsed_ms: 0,
                timed_out: false,
                cancelled: true,
                stdout: String::new(),
                stderr: String::new(),
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
        let mut child = command.spawn().map_err(CommandRunnerError::Spawn)?;
        let process_id = child.id();
        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| CommandRunnerError::InvalidRequest("stdout pipe missing".into()))?;
        let stderr = child
            .stderr
            .take()
            .ok_or_else(|| CommandRunnerError::InvalidRequest("stderr pipe missing".into()))?;
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
            status = child.wait() => Completion::Exited(status.map_err(CommandRunnerError::Io)?),
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
            .map_err(|error| CommandRunnerError::Join(error.to_string()))?
            .map_err(CommandRunnerError::Io)?;
        let stderr = stderr_task
            .await
            .map_err(|error| CommandRunnerError::Join(error.to_string()))?
            .map_err(CommandRunnerError::Io)?;

        Ok(CommandOutput {
            exit_code: status.and_then(|status| status.code()),
            elapsed_ms: started.elapsed().as_millis() as u64,
            timed_out,
            cancelled,
            stdout: String::from_utf8_lossy(&stdout.bytes).into_owned(),
            stderr: String::from_utf8_lossy(&stderr.bytes).into_owned(),
            stdout_truncated: stdout.truncated,
            stderr_truncated: stderr.truncated,
        })
    }
}

fn validate(request: &CommandRequest) -> Result<(), CommandRunnerError> {
    if request.program.as_os_str().is_empty() {
        return Err(CommandRunnerError::InvalidRequest(
            "program must not be empty".into(),
        ));
    }
    if !request.working_dir.is_dir() {
        return Err(CommandRunnerError::InvalidRequest(
            "working directory must exist".into(),
        ));
    }
    if request.timeout.is_zero() || request.stream_cap_bytes == 0 {
        return Err(CommandRunnerError::InvalidRequest(
            "timeout and stream cap must be positive".into(),
        ));
    }
    if request
        .environment
        .keys()
        .any(|key| key.is_empty() || key.contains('=') || key.contains('\0') || key.contains('/'))
    {
        return Err(CommandRunnerError::InvalidRequest(
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

#[cfg(test)]
mod tests {
    use super::*;

    fn request(program: &str, args: &[&str]) -> CommandRequest {
        CommandRequest {
            program: program.into(),
            args: args.iter().map(|value| (*value).into()).collect(),
            working_dir: std::env::temp_dir(),
            environment: BTreeMap::new(),
            stdin: None,
            timeout: Duration::from_secs(2),
            stream_cap_bytes: 8 * 1024,
        }
    }

    #[tokio::test]
    async fn success_and_nonzero_exit_are_structured() {
        let runner = CommandRunner;
        let success = runner
            .run(
                request("/bin/sh", &["-c", "printf ok; printf err >&2"]),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(success.exit_code, Some(0));
        assert_eq!(success.stdout, "ok");
        assert_eq!(success.stderr, "err");
        let failure = runner
            .run(
                request("/bin/sh", &["-c", "exit 17"]),
                CancellationToken::new(),
            )
            .await
            .unwrap();
        assert_eq!(failure.exit_code, Some(17));
    }

    #[tokio::test]
    async fn timeout_and_cancellation_are_distinct() {
        let runner = CommandRunner;
        let mut timed = request("/bin/sh", &["-c", "sleep 10"]);
        timed.timeout = Duration::from_millis(20);
        let output = runner.run(timed, CancellationToken::new()).await.unwrap();
        assert!(output.timed_out);
        assert!(!output.cancelled);

        let cancel = CancellationToken::new();
        let future = runner.run(request("/bin/sh", &["-c", "sleep 10"]), cancel.clone());
        tokio::pin!(future);
        tokio::time::sleep(Duration::from_millis(20)).await;
        cancel.cancel();
        let output = future.await.unwrap();
        assert!(output.cancelled);
        assert!(!output.timed_out);
    }

    #[tokio::test]
    async fn output_is_capped_per_stream_while_pipes_are_drained() {
        let runner = CommandRunner;
        let mut req = request(
            "/bin/sh",
            &["-c", "head -c 10000 /dev/zero; head -c 9000 /dev/zero >&2"],
        );
        req.stream_cap_bytes = 127;
        let output = runner.run(req, CancellationToken::new()).await.unwrap();
        assert_eq!(output.stdout.len(), 127);
        assert_eq!(output.stderr.len(), 127);
        assert!(output.stdout_truncated);
        assert!(output.stderr_truncated);
    }

    #[tokio::test]
    async fn missing_executable_is_a_spawn_error() {
        let error = CommandRunner
            .run(
                request("/definitely/missing/executable", &[]),
                CancellationToken::new(),
            )
            .await
            .unwrap_err();
        assert!(matches!(error, CommandRunnerError::Spawn(_)));
    }

    #[tokio::test]
    async fn environment_is_cleared_then_populated_from_allow_list() {
        let runner = CommandRunner;
        let mut req = request("/usr/bin/env", &[]);
        req.environment.insert("ALLOWED_VALUE".into(), "yes".into());
        std::env::set_var("MUST_NOT_LEAK_TO_COMMAND", "secret");
        let output = runner.run(req, CancellationToken::new()).await.unwrap();
        assert!(output.stdout.contains("ALLOWED_VALUE=yes"));
        assert!(!output.stdout.contains("MUST_NOT_LEAK_TO_COMMAND"));
        assert!(!output.stdout.contains("secret"));
        std::env::remove_var("MUST_NOT_LEAK_TO_COMMAND");
    }

    #[cfg(unix)]
    #[tokio::test]
    async fn timeout_kills_descendant_process_group() {
        let dir = tempfile::tempdir().unwrap();
        let pid_file = dir.path().join("child.pid");
        let script = format!("sleep 30 & echo $! > {}; wait", pid_file.display());
        let mut req = request("/bin/sh", &["-c", &script]);
        req.timeout = Duration::from_millis(100);
        let output = CommandRunner
            .run(req, CancellationToken::new())
            .await
            .unwrap();
        assert!(output.timed_out);
        let pid: i32 = std::fs::read_to_string(pid_file)
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        for _ in 0..20 {
            let alive = unsafe { libc::kill(pid, 0) } == 0;
            if !alive {
                return;
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        panic!("descendant process {pid} survived process-group termination");
    }
}
