use async_trait::async_trait;
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
                    "description": "Timeout in seconds (default: 30)",
                    "default": 30
                }
            },
            "required": ["command"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> { Box::new(BashExecTool) }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let command = input["command"].as_str().unwrap_or("");
        let _timeout = input["timeout_seconds"].as_u64().unwrap_or(30);

        let start = std::time::Instant::now();

        let result = Command::new("bash")
            .arg("-c")
            .arg(command)
            .current_dir(&ctx.working_dir)
            .output()
            .await;

        let elapsed = start.elapsed().as_millis() as u64;

        match result {
            Ok(output) => {
                // Layer 1: Capture with byte-level limits (1MB per stream)
                let captured = capture_output(&output.stdout, &output.stderr, &Default::default());

                // Layer 2: Per-result overflow to file with head/tail truncation
                let output_config = OutputConfig::default();
                let processed = process_result("bash_exec", &captured.content, &output_config)
                    .await
                    .unwrap_or_else(|e| {
                        tracing::warn!(error = %e, "Output processing failed, using inline");
                        super::output::ProcessedOutput::Inline {
                            content: captured.content.clone(),
                            original_bytes: captured.content.len(),
                        }
                    });

                let content = processed.to_context_content().to_string();
                let truncated = processed.was_truncated() || captured.stdout_truncated || captured.stderr_truncated;

                ToolResult {
                    content,
                    is_error: !output.status.success(),
                    metadata: ToolResultMeta {
                        execution_time_ms: elapsed,
                        truncated,
                    },
                }
            }
            Err(e) => ToolResult {
                content: format!("Failed to execute command: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: false,
                },
            },
        }
    }
}
