use async_trait::async_trait;
use serde_json::json;

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct SystemStatusTool;

#[async_trait]
impl Tool for SystemStatusTool {
    fn name(&self) -> &str {
        "system_status"
    }

    fn description(&self) -> &str {
        "Get system status: CPU, memory, disk usage"
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

    fn boxed_clone(&self) -> Box<dyn Tool> { Box::new(SystemStatusTool) }

    async fn execute(&self, _input: serde_json::Value, _ctx: &ToolContext) -> ToolResult {
        let start = std::time::Instant::now();

        let mut parts = Vec::new();

        // Memory info from /proc/meminfo
        if let Ok(meminfo) = tokio::fs::read_to_string("/proc/meminfo").await {
            for line in meminfo.lines().take(5) {
                parts.push(line.to_string());
            }
        }

        // Load average
        if let Ok(loadavg) = tokio::fs::read_to_string("/proc/loadavg").await {
            parts.push(format!("Load: {}", loadavg.trim()));
        }

        // Disk usage
        if let Ok(output) = tokio::process::Command::new("df")
            .args(["-h", "/"])
            .output()
            .await
        {
            let df = String::from_utf8_lossy(&output.stdout);
            for line in df.lines().take(2) {
                parts.push(line.to_string());
            }
        }

        ToolResult {
            content: parts.join("\n"),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: start.elapsed().as_millis() as u64,
                truncated: false,
            },
        }
    }
}
