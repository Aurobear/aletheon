use async_trait::async_trait;
use serde_json::json;
use tokio::fs;

use super::mutation_path::validate_mutation_path;
use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct FileWriteTool;

#[async_trait]
impl Tool for FileWriteTool {
    fn name(&self) -> &str {
        "file_write"
    }

    fn description(&self) -> &str {
        "Write content to a file (creates or overwrites)"
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to write"
                },
                "content": {
                    "type": "string",
                    "description": "Content to write"
                }
            },
            "required": ["path", "content"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(FileWriteTool)
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = input["path"].as_str().unwrap_or("");
        let content = input["content"].as_str().unwrap_or("");
        let start = ctx.clock.mono_now();

        let full_path = match validate_mutation_path(&ctx.working_dir, std::path::Path::new(path)) {
            Ok(path) => path,
            Err(error) => {
                return ToolResult {
                    content: format!("Refused to write {path}: {error}"),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        };

        // Create parent directories if needed
        if let Some(parent) = full_path.parent() {
            if let Err(e) = fs::create_dir_all(parent).await {
                return ToolResult {
                    content: format!("Failed to create directory: {}", e),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                };
            }
        }

        match fs::write(&full_path, content).await {
            Ok(_) => ToolResult {
                content: format!("Wrote {} bytes to {}", content.len(), full_path.display()),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
            Err(e) => ToolResult {
                content: format!("Failed to write {}: {}", full_path.display(), e),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            },
        }
    }
}
