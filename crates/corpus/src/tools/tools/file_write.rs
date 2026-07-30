use super::scoped_filesystem;
use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};
use async_trait::async_trait;
use serde_json::json;

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
                },
                "expected_sha256": {
                    "type": "string",
                    "description": "Optional: expected hash of current file. Write refused if mismatch (stale workspace view)."
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

        let filesystem = match scoped_filesystem::open(
            ctx,
            std::path::Path::new(path),
            platform::FilesystemAccess::ReadWrite,
        ) {
            Ok(filesystem) => filesystem,
            Err(error) => {
                return ToolResult {
                    content: format!("Refused to write {path}: {error}"),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
        };

        // Create parent directories if needed
        if let Some(parent) = filesystem.path.native().parent() {
            if let Err(error) = filesystem
                .host
                .create_dir_all(&platform::HostPath::new(parent.to_path_buf()))
                .await
            {
                return ToolResult {
                    content: format!("Failed to create directory: {error}"),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
        }

        let before_metadata = filesystem.host.metadata(&filesystem.path).await.ok();
        let existed = before_metadata
            .as_ref()
            .is_some_and(|metadata| metadata.is_file);
        let bytes_before = before_metadata.map_or(0, |metadata| metadata.size_bytes);
        let change_type = if existed { "modified" } else { "added" };
        let evidence_path = if std::path::Path::new(path).is_absolute() {
            ctx.effective_workspace_policy()
                .ok()
                .and_then(|workspace| {
                    std::path::Path::new(path)
                        .strip_prefix(workspace.cwd())
                        .ok()
                        .map(|relative| relative.to_string_lossy().into_owned())
                })
                .unwrap_or_else(|| path.to_owned())
        } else {
            path.to_owned()
        };

        match filesystem
            .host
            .atomic_write(platform::AtomicWrite {
                path: filesystem.path,
                content: content.as_bytes().to_vec(),
                expected_sha256: input
                    .get("expected_sha256")
                    .and_then(|value| value.as_str())
                    .map(str::to_owned),
                mode: None,
            })
            .await
        {
            Ok(receipt) => ToolResult {
                content: format!(
                    "Wrote {} bytes (sha256 {})",
                    receipt.bytes_written, receipt.sha256
                ),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: Some(fabric::PatchDelta {
                        applied: vec![fabric::PatchDeltaApplied {
                            operation: "write".into(),
                            path: evidence_path.clone(),
                            hunks_applied: Some(1),
                            bytes_written: Some(receipt.bytes_written),
                            moved_to: None,
                        }],
                        failed: vec![],
                        files_changed: vec![fabric::PatchDeltaFileChange {
                            path: evidence_path,
                            change_type: change_type.into(),
                            hunks_applied: 1,
                            bytes_before,
                            bytes_after: receipt.bytes_written,
                            is_binary: false,
                        }],
                        ..Default::default()
                    }),
                },
            },
            Err(error) => ToolResult {
                content: format!("Failed to write {path}: {error}"),
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
