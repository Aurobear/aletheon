use async_trait::async_trait;
use fabric::Timer;
use serde_json::json;
use tokio::process::Command;

use super::output::{capture_output, process_result, OutputConfig};
use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct BashExecTool;

#[async_trait]
impl Tool for BashExecTool {
    fn name(&self) -> &str {
        "bash_exec"
    }

    fn description(&self) -> &str {
        "Execute a bash command and return stdout/stderr"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "command": {
                    "type": "string",
                    "description": "The bash command to execute"
                },
                "timeout_seconds": {
                    "type": "integer",
                    "description": "Timeout in seconds (default: 10)",
                    "default": 10
                }
            },
            "required": ["command"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(BashExecTool)
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let command = input["command"].as_str().unwrap_or("");
        let timeout_secs = input["timeout_seconds"].as_u64().unwrap_or(10);

        let start = ctx.clock.mono_now();

        let result = aletheon_kernel::chronos::SystemTimer
            .timeout(
                std::time::Duration::from_secs(timeout_secs),
                Command::new("bash")
                    .arg("-c")
                    .arg(command)
                    .current_dir(&ctx.working_dir)
                    .output(),
            )
            .await;

        let elapsed = ctx.clock.mono_now().0.saturating_sub(start.0);

        match result {
            Ok(Ok(output)) => {
                // Layer 1: Capture with byte-level limits (1MB per stream)
                let captured = capture_output(&output.stdout, &output.stderr, &Default::default());

                // Layer 2: Per-result overflow to file with head/tail truncation
                let output_config = OutputConfig::default();
                let processed =
                    process_result("bash_exec", &captured.content, &output_config, &*ctx.clock)
                        .await
                        .unwrap_or_else(|e| {
                            tracing::warn!(error = %e, "Output processing failed, using inline");
                            super::output::ProcessedOutput::Inline {
                                content: captured.content.clone(),
                                original_bytes: captured.content.len(),
                            }
                        });

                let content = processed.to_context_content().to_string();
                let truncated = processed.was_truncated()
                    || captured.stdout_truncated
                    || captured.stderr_truncated;

                ToolResult {
                    content,
                    is_error: !output.status.success(),
                    metadata: ToolResultMeta {
                        execution_time_ms: elapsed,
                        truncated,
                    },
                }
            }
            Ok(Err(e)) => ToolResult {
                content: format!("Failed to execute command: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: false,
                },
            },
            Err(_) => ToolResult {
                content: format!("Command timed out after {} seconds", timeout_secs),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: false,
                },
            },
        }
    }

    async fn execute_streaming(
        &self,
        input: serde_json::Value,
        ctx: &ToolContext,
        sink: &mut fabric::ToolEventSink,
    ) {
        let command_text = input["command"].as_str().unwrap_or("");
        let timeout_secs = input["timeout_seconds"].as_u64().unwrap_or(10);
        let mut command = Command::new("bash");
        command
            .arg("-c")
            .arg(command_text)
            .current_dir(&ctx.working_dir);

        let result = crate::security::sandbox::streaming::execute_command_streaming(
            command,
            std::time::Duration::from_secs(timeout_secs),
            "bash_exec",
            fabric::IsolationLevel::None,
            ctx.clock.clone(),
            sink,
        )
        .await;

        let terminal = match result {
            Ok(output) => {
                let captured = capture_output(
                    output.stdout.as_bytes(),
                    output.stderr.as_bytes(),
                    &Default::default(),
                );
                let processed = process_result(
                    "bash_exec",
                    &captured.content,
                    &OutputConfig::default(),
                    &*ctx.clock,
                )
                .await
                .unwrap_or_else(|error| {
                    tracing::warn!(%error, "streaming output processing failed, using inline");
                    super::output::ProcessedOutput::Inline {
                        content: captured.content.clone(),
                        original_bytes: captured.content.len(),
                    }
                });
                ToolResult {
                    content: processed.to_context_content().to_string(),
                    is_error: output.exit_code != 0,
                    metadata: ToolResultMeta {
                        execution_time_ms: output.elapsed_ms,
                        truncated: processed.was_truncated()
                            || captured.stdout_truncated
                            || captured.stderr_truncated,
                    },
                }
            }
            Err(error) => ToolResult {
                content: format!("Failed to execute command: {error}"),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: 0,
                    truncated: false,
                },
            },
        };
        sink.terminal(Ok(terminal)).await;
    }
}

#[cfg(test)]
mod streaming_tests {
    use super::*;
    use fabric::{tool_event_channel, ToolExecutionEvent, ToolProgress};
    use std::sync::Arc;

    #[tokio::test]
    async fn bash_streams_lines_then_emits_exactly_one_terminal() {
        let (mut sink, mut events) = tool_event_channel();
        let context = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: std::env::temp_dir(),
            session_id: "bash-stream-test".into(),
            clock: Arc::new(aletheon_kernel::chronos::SystemClock::new()),
        };
        BashExecTool
            .execute_streaming(
                serde_json::json!({
                    "command": "printf 'first\\n'; printf 'second\\n'; printf 'warning\\n' >&2"
                }),
                &context,
                &mut sink,
            )
            .await;
        drop(sink);

        let mut progress = Vec::new();
        let mut terminals = Vec::new();
        let mut saw_terminal = false;
        while let Some(event) = events.recv().await {
            match event {
                ToolExecutionEvent::Progress(ToolProgress::Text(line)) => {
                    assert!(!saw_terminal, "progress must not follow terminal");
                    progress.push(line)
                }
                ToolExecutionEvent::Terminal(result) => {
                    assert!(!saw_terminal, "terminal must be unique");
                    saw_terminal = true;
                    terminals.push(result)
                }
                _ => {}
            }
        }
        assert!(progress.contains(&"first".to_string()));
        assert!(progress.contains(&"second".to_string()));
        assert!(progress.contains(&"warning".to_string()));
        assert_eq!(terminals.len(), 1);
        let terminal = terminals.pop().unwrap().unwrap();
        assert!(!terminal.is_error);
        assert!(terminal.content.contains("first"));
        assert!(terminal.content.contains("warning"));
    }
}
