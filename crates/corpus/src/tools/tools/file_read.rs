use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use crate::tools::tools::scoped_filesystem;
use async_trait::async_trait;
use serde_json::json;

pub struct FileReadTool;
const MAX_BATCH_FILES: usize = 8;

#[async_trait]
impl Tool for FileReadTool {
    fn name(&self) -> &str {
        "file_read"
    }

    fn description(&self) -> &str {
        "Read one or more known files. Batch independent reads in one call with `paths` instead of issuing one model round per file. Results for batched reads are labeled by path."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "path": {
                    "type": "string",
                    "description": "Path to the file to read"
                },
                "paths": {
                    "type": "array",
                    "items": {"type": "string"},
                    "maxItems": 8,
                    "description": "Up to 8 known files to read together. Prefer this for independent repository entry files."
                },
                "offset": {
                    "type": "integer",
                    "description": "Line number to start reading from (0-based)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum number of lines to read"
                }
            }
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(FileReadTool)
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let paths = match parse_paths(&input) {
            Ok(paths) => paths,
            Err(message) => {
                return error_result(
                    ctx,
                    ctx.clock.mono_now(),
                    format!("Invalid file_read input: {message}"),
                )
            }
        };
        let offset = input["offset"].as_u64().unwrap_or(0) as usize;
        let limit = input["limit"].as_u64().unwrap_or(2000) as usize;

        let start = ctx.clock.mono_now();
        let batched = paths.len() > 1;
        let mut sections = Vec::with_capacity(paths.len());
        let mut any_error = false;
        let mut any_truncated = false;
        for path in paths {
            let result = read_path(&path, offset, limit, ctx).await;
            any_error |= result.is_error;
            any_truncated |= result.metadata.truncated;
            if batched {
                sections.push(format!("== {path} ==\n{}", result.content));
            } else {
                sections.push(result.content);
            }
        }
        ToolResult {
            content: sections.join("\n\n"),
            is_error: any_error,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: any_truncated,
                patch_delta: None,
            },
        }
    }
}

fn parse_paths(input: &serde_json::Value) -> Result<Vec<String>, String> {
    let mut paths = Vec::new();
    if let Some(path) = input.get("path").and_then(|value| value.as_str()) {
        paths.push(path.to_string());
    }
    if let Some(values) = input.get("paths") {
        let values = values
            .as_array()
            .ok_or_else(|| "'paths' must be an array of strings".to_string())?;
        if values.len() > MAX_BATCH_FILES {
            return Err(format!("'paths' accepts at most {MAX_BATCH_FILES} entries"));
        }
        for value in values {
            paths.push(
                value
                    .as_str()
                    .ok_or_else(|| "'paths' must contain only strings".to_string())?
                    .to_string(),
            );
        }
    }
    if paths.is_empty() {
        return Err("'path' or non-empty 'paths' parameter is required".to_string());
    }
    if paths.len() > MAX_BATCH_FILES {
        return Err(format!(
            "at most {MAX_BATCH_FILES} combined paths are allowed"
        ));
    }
    Ok(paths)
}

async fn read_path(path: &str, offset: usize, limit: usize, ctx: &ToolContext) -> ToolResult {
    let start = ctx.clock.mono_now();
    let filesystem = match scoped_filesystem::open(
        ctx,
        std::path::Path::new(path),
        platform::FilesystemAccess::ReadOnly,
    ) {
        Ok(filesystem) => filesystem,
        Err(error) => return error_result(ctx, start, format!("Refused to read {path}: {error}")),
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
