use async_trait::async_trait;
use serde_json::json;

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct ProcessListTool;

#[async_trait]
impl Tool for ProcessListTool {
    fn name(&self) -> &str {
        "process_list"
    }

    fn description(&self) -> &str {
        "List running processes (top 20 by CPU usage)"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {}
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(ProcessListTool)
    }

    async fn execute(&self, _input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let result = tokio::process::Command::new("ps")
            .args(["aux", "--sort=-pcpu"])
            .output()
            .await;

        match result {
            Ok(output) => {
                let stdout = String::from_utf8_lossy(&output.stdout);
                let lines: Vec<&str> = stdout.lines().collect();
                let head = lines.first().copied().unwrap_or("");
                let body: Vec<&str> = lines.iter().skip(1).take(20).copied().collect();
                let mut content = format!("{}\n{}", head, body.join("\n"));
                if lines.len() > 21 {
                    content.push_str(&format!("\n... ({} more processes)", lines.len() - 21));
                }
                ToolResult {
                    content,
                    is_error: false,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: lines.len() > 21,
                    },
                }
            }
            Err(e) => ToolResult {
                content: format!("Failed to list processes: {}", e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }
}
