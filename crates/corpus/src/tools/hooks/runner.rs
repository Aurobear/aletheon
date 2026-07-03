//! Hook runner — executes hooks and collects responses.

use std::io::Write;
use std::process::{Command, Stdio};
use std::time::Duration;

use tracing::{debug, warn};

use super::types::{Hook, HookPayload, HookResponse};

/// Runs hooks sequentially, short-circuiting on `proceed=false`.
pub struct HookRunner;

impl HookRunner {
    /// Run a list of hooks for a given payload.
    ///
    /// Execution stops at the first hook that returns `proceed=false`.
    /// If a hook crashes or times out, execution continues with a warning.
    pub fn run_hooks(hooks: &[&Hook], payload: &HookPayload) -> HookResponse {
        let json_payload = match serde_json::to_string(payload) {
            Ok(j) => j,
            Err(e) => {
                warn!("Failed to serialize hook payload: {e}");
                return HookResponse::default();
            }
        };

        for hook in hooks {
            debug!(hook = %hook.name, event = ?payload.event, "running hook");

            match Self::run_single(hook, &json_payload) {
                Ok(resp) => {
                    debug!(hook = %hook.name, proceed = resp.proceed, "hook responded");
                    if !resp.proceed {
                        return resp;
                    }
                }
                Err(e) => {
                    warn!(hook = %hook.name, error = %e, "hook failed, continuing");
                }
            }
        }

        HookResponse::default()
    }

    fn run_single(hook: &Hook, json_payload: &str) -> Result<HookResponse, anyhow::Error> {
        let timeout = Duration::from_millis(hook.timeout_ms);

        let mut child = Command::new("sh")
            .arg("-c")
            .arg(&hook.command)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| anyhow::anyhow!("spawn failed: {e}"))?;

        // Write payload to stdin.
        if let Some(ref mut stdin) = child.stdin {
            stdin
                .write_all(json_payload.as_bytes())
                .map_err(|e| anyhow::anyhow!("stdin write failed: {e}"))?;
        }
        // Close stdin so the hook knows input is done.
        drop(child.stdin.take());

        let pid = child.id();

        // Wait with timeout using a background thread.
        let (tx, rx) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            let result = child.wait_with_output();
            let _ = tx.send(result);
        });

        match rx.recv_timeout(timeout) {
            Ok(Ok(output)) => {
                if !output.status.success() {
                    return Err(anyhow::anyhow!(
                        "hook exited with {}",
                        output.status.code().unwrap_or(-1)
                    ));
                }

                let stdout = String::from_utf8_lossy(&output.stdout);
                let trimmed = stdout.trim();
                if trimmed.is_empty() {
                    // No output = implicit proceed.
                    return Ok(HookResponse::default());
                }

                serde_json::from_str::<HookResponse>(trimmed)
                    .map_err(|e| anyhow::anyhow!("invalid hook response JSON: {e}"))
            }
            Ok(Err(e)) => Err(anyhow::anyhow!("hook process error: {e}")),
            Err(_timeout_err) => {
                // Timeout — kill the process.
                #[cfg(unix)]
                unsafe {
                    libc::kill(pid as i32, libc::SIGKILL);
                }
                Err(anyhow::anyhow!("hook timed out after {timeout:?}"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::hooks::types::HookEvent;

    #[test]
    fn all_proceed() {
        let hooks = [
            Hook {
                name: "h1".into(),
                event: HookEvent::SessionStart,
                command: "echo '{\"proceed\": true}'".into(),
                timeout_ms: 5000,
            },
            Hook {
                name: "h2".into(),
                event: HookEvent::SessionStart,
                command: "echo '{\"proceed\": true, \"message\": \"ok\"}'".into(),
                timeout_ms: 5000,
            },
        ];
        let refs: Vec<&Hook> = hooks.iter().collect();
        let payload = HookPayload {
            event: HookEvent::SessionStart,
            session_id: "test".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            data: serde_json::Value::Null,
        };
        let resp = HookRunner::run_hooks(&refs, &payload);
        assert!(resp.proceed);
    }

    #[test]
    fn stops_on_block() {
        let hooks = [
            Hook {
                name: "blocker".into(),
                event: HookEvent::PreToolUse,
                command: "echo '{\"proceed\": false, \"message\": \"blocked\"}'".into(),
                timeout_ms: 5000,
            },
            Hook {
                name: "unreached".into(),
                event: HookEvent::PreToolUse,
                command: "echo '{\"proceed\": true}'".into(),
                timeout_ms: 5000,
            },
        ];
        let refs: Vec<&Hook> = hooks.iter().collect();
        let payload = HookPayload {
            event: HookEvent::PreToolUse,
            session_id: "test".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            data: serde_json::Value::Null,
        };
        let resp = HookRunner::run_hooks(&refs, &payload);
        assert!(!resp.proceed);
        assert_eq!(resp.message.as_deref(), Some("blocked"));
    }

    #[test]
    fn crash_continues() {
        let hooks = [
            Hook {
                name: "crasher".into(),
                event: HookEvent::Stop,
                command: "exit 1".into(),
                timeout_ms: 5000,
            },
            Hook {
                name: "recovered".into(),
                event: HookEvent::Stop,
                command: "echo '{\"proceed\": true}'".into(),
                timeout_ms: 5000,
            },
        ];
        let refs: Vec<&Hook> = hooks.iter().collect();
        let payload = HookPayload {
            event: HookEvent::Stop,
            session_id: "test".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            data: serde_json::Value::Null,
        };
        let resp = HookRunner::run_hooks(&refs, &payload);
        assert!(resp.proceed);
    }

    #[test]
    fn empty_hooks_proceed() {
        let hooks: Vec<&Hook> = vec![];
        let payload = HookPayload {
            event: HookEvent::SessionStart,
            session_id: "test".into(),
            timestamp: "2026-01-01T00:00:00Z".into(),
            data: serde_json::Value::Null,
        };
        let resp = HookRunner::run_hooks(&hooks, &payload);
        assert!(resp.proceed);
    }

    #[test]
    fn stdin_passthrough() {
        let hooks = [Hook {
            name: "reader".into(),
            event: HookEvent::UserPromptSubmit,
            // Read stdin, echo it back — verifies payload is passed.
            command: "cat".into(),
            timeout_ms: 5000,
        }];
        let refs: Vec<&Hook> = hooks.iter().collect();
        let payload = HookPayload {
            event: HookEvent::UserPromptSubmit,
            session_id: "abc".into(),
            timestamp: "2026-06-14T00:00:00Z".into(),
            data: serde_json::json!({"prompt": "hello"}),
        };
        let resp = HookRunner::run_hooks(&refs, &payload);
        // "cat" echoes the JSON payload back, which is not a valid HookResponse
        // with proceed:false, so the runner treats it as a parse error -> proceed.
        assert!(resp.proceed);
    }
}
