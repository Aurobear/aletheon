use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use crate::tools::tools::scoped_filesystem;
use async_trait::async_trait;
use serde_json::json;
use sha2::{Digest, Sha256};

use crate::tools::artifact::ArtifactStore;

pub struct FileReadTool;
const MAX_BATCH_FILES: usize = 16;

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
                    "maxItems": 16,
                    "description": "Up to 16 known files to read together. Prefer this for independent repository entry files."
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
        let mut records = Vec::with_capacity(paths.len());
        let mut successful_reads = 0usize;
        let mut any_truncated = false;
        for path in paths {
            let result = read_path(&path, offset, limit, ctx).await;
            if !result.is_error {
                successful_reads += 1;
            }
            any_truncated |= result.metadata.truncated;
            records.push(
                serde_json::from_str::<serde_json::Value>(&result.content).unwrap_or_else(
                    |_| json!({"path": path, "status": "error", "error": result.content}),
                ),
            );
        }
        ToolResult {
            content: json!({
                "kind": "file_read_receipt",
                "complete": successful_reads == records.len() && !any_truncated,
                "files": records,
            })
            .to_string(),
            // A speculative batch is useful when at least one requested file
            // exists. Preserve individual failures in labeled content without
            // turning the whole batch into a failed tool round.
            is_error: successful_reads == 0,
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
                content: json!({
                    "path": path,
                    "status": "error",
                    "error": format!("path is a directory; use glob/grep/file_search to enumerate contents: {path}")
                }).to_string(),
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
        Ok(bytes) => match String::from_utf8(bytes.clone()) {
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
                let selected_content = selected.join("\n");
                let store = ArtifactStore::new(
                    super::output::OutputConfig::default()
                        .overflow_dir
                        .join("artifacts"),
                );
                let artifact = match store.store(&bytes, "text/plain; charset=utf-8") {
                    Ok(artifact) => artifact,
                    Err(error) => {
                        return error_result(
                            ctx,
                            start,
                            format!("Failed to preserve evidence for {path}: {error}"),
                        )
                    }
                };
                let end_line = offset.saturating_add(selected.len()).min(lines.len());

                ToolResult {
                    content: json!({
                        "path": path,
                        "status": "ok",
                        "encoding": "utf-8",
                        "sha256": format!("{:x}", Sha256::digest(&bytes)),
                        "size_bytes": bytes.len(),
                        "artifact_ref": artifact.uri(),
                        "line_range": {"start": offset.saturating_add(1), "end": end_line},
                        "total_lines": lines.len(),
                        "truncated": truncated,
                        "content": selected_content,
                    })
                    .to_string(),
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
            content: json!({
                "path": path,
                "status": "error",
                "error": format!("Failed to read {path}: {error}")
            })
            .to_string(),
            is_error: true,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
                patch_delta: None,
            },
        },
    }
}

fn error_result(ctx: &ToolContext, start: ::contracts::MonoTime, content: String) -> ToolResult {
    ToolResult {
        content: json!({"status": "error", "error": content}).to_string(),
        is_error: true,
        metadata: ToolResultMeta {
            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
            truncated: false,
            patch_delta: None,
        },
    }
}
