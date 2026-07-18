use std::sync::Arc;
use std::time::Duration;

use anyhow::Result;
use fabric::{Clock, IsolationLevel, SandboxResult, ToolEventSink, ToolProgress};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::process::Command;

/// Run a configured child while forwarding stdout/stderr lines as bounded G2
/// progress. Dropped progress never affects process completion or capture.
pub async fn execute_command_streaming(
    mut command: Command,
    timeout: Duration,
    backend_used: &str,
    isolation_level: IsolationLevel,
    clock: Arc<dyn Clock>,
    sink: &ToolEventSink,
) -> Result<SandboxResult> {
    command
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .kill_on_drop(true);
    let start = clock.mono_now();
    let mut child = command.spawn()?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| anyhow::anyhow!("streaming child stdout was not piped"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| anyhow::anyhow!("streaming child stderr was not piped"))?;
    let mut stdout = BufReader::new(stdout).lines();
    let mut stderr = BufReader::new(stderr).lines();
    let mut stdout_done = false;
    let mut stderr_done = false;
    let mut status = None;
    let mut captured_stdout = String::new();
    let mut captured_stderr = String::new();

    let execution = async {
        while status.is_none() || !stdout_done || !stderr_done {
            tokio::select! {
                line = stdout.next_line(), if !stdout_done => match line? {
                    Some(line) => {
                        captured_stdout.push_str(&line);
                        captured_stdout.push('\n');
                        let _ = sink.progress(ToolProgress::Text(line));
                    }
                    None => stdout_done = true,
                },
                line = stderr.next_line(), if !stderr_done => match line? {
                    Some(line) => {
                        captured_stderr.push_str(&line);
                        captured_stderr.push('\n');
                        let _ = sink.progress(ToolProgress::Text(line));
                    }
                    None => stderr_done = true,
                },
                child_status = child.wait(), if status.is_none() => {
                    status = Some(child_status?);
                }
            }
        }
        Result::<_, std::io::Error>::Ok(status.expect("child status set after wait"))
    };

    let (exit_code, timed_out) = match tokio::time::timeout(timeout, execution).await {
        Ok(Ok(status)) => (status.code().unwrap_or(-1), false),
        Ok(Err(error)) => return Err(error.into()),
        Err(_) => {
            let _ = child.kill().await;
            (-1, true)
        }
    };
    if timed_out {
        captured_stderr.push_str(&format!(
            "Command timed out after {} seconds",
            timeout.as_secs()
        ));
    }
    let elapsed_ms = clock.mono_now().0.saturating_sub(start.0);
    Ok(SandboxResult {
        stdout: captured_stdout,
        stderr: captured_stderr,
        exit_code,
        backend_used: backend_used.to_owned(),
        isolation_level,
        elapsed_ms,
    })
}
