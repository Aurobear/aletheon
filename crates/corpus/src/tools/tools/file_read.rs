use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use crate::tools::tools::scoped_filesystem;
use async_trait::async_trait;
use serde_json::json;

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

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(FileReadTool)
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let path = input["path"].as_str().unwrap_or("");
        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(2000) as usize;

        let start = ctx.clock.mono_now();
        let filesystem = match scoped_filesystem::open(
            ctx,
            std::path::Path::new(path),
            platform::FilesystemAccess::ReadOnly,
        ) {
            Ok(filesystem) => filesystem,
            Err(error) => {
                return error_result(ctx, start, format!("Refused to read {path}: {error}"))
            }
        };

        match std::fs::metadata(filesystem.path.native()) {
            Ok(metadata) if metadata.is_dir() => {
                return ToolResult {
                    content: format!(
                        "path is a directory; use glob/grep/file_search to enumerate contents: {path}"
                    ),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
            _ => {}
        }

        match filesystem.host.read(&filesystem.path).await {
            Ok(bytes) => match String::from_utf8(bytes) {
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
                            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                            truncated,
                            patch_delta: None,
                        },
                    }
                }
                Err(error) => error_result(ctx, start, format!("File is not UTF-8: {error}")),
            },
            Err(error) => ToolResult {
                content: format!("Failed to read {path}: {error}"),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            },
        }
    }
}

fn error_result(ctx: &ToolContext, start: fabric::MonoTime, content: String) -> ToolResult {
    ToolResult {
        content,
        is_error: true,
        metadata: ToolResultMeta {
            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
            truncated: false,
            patch_delta: None,
        },
    }
}
