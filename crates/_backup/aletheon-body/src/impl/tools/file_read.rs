use async_trait::async_trait;
use serde_json::json;
use tokio::fs;

use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct FileReadTool;

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read a file and return its contents"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (0-based)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read"
                }
            },
            "required": ["path"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> { Box::new(FileReadTool) }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = input["path"].as_str().unwrap_or("");
        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(2000) as usize;

        let full_path = if std::path::Path::new(path).is_absolute() {
            std::path::PathBuf::from(path)
        } else {
            ctx.working_dir.join(path)
        };

        let start = std::time::Instant::now();

        match fs::read_to_string(&full_path).await {
            Ok(content) => {
                let lines: Vec<&str> = content.lines().collect();
                let selected: Vec<String> = lines
                    .iter()
                    .skip(offset)
                    .take(limit)
                    .enumerate()
                    .map(|(i, line)| format!("{:>5}\t{}", offset + i + 1, line))
                    .collect();

                let truncated = lines.len() > offset + limit;
                let result = selected.join("\n");

                ToolResult {
                    content: result,
                    is_error: false,
                    metadata: ToolResultMeta {
                        execution_time_ms: start.elapsed().as_millis() as u64,
                        truncated,
                    },
                }
            }
            Err(e) => ToolResult {
                content: format!("Failed to read {}: {}", full_path.display(), e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: start.elapsed().as_millis() as u64,
                    truncated: false,
                },
            },
        }
    }
}
