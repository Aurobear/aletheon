use async_trait::async_trait;
use serde_json::json;
use sha2::{Digest, Sha256};

use super::mutation_path::validate_mutation_path;
use super::scoped_filesystem;
use super::structured_patch::{
    apply_patch_hunks, parse_structured_patch, parse_structured_patch_json, AppliedOperation,
    FailedOperation, FileChangeSummary, PatchOperation, StructuredPatchResult,
};
use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply a unified diff or structured patch to files. Supports creating, modifying, moving, appending, and deleting files."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Unified diff or *** Begin Patch structured patch content"
                },
                "patch_json": {
                    "type": "object",
                    "description": "Structured patch object containing an operations array"
                },
                "base_dir": {
                    "type": "string",
                    "description": "Base directory for applying the patch (default: current dir)"
                }
            },
            "anyOf": [
                {"required": ["patch"]},
                {"required": ["patch_json"]}
            ]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(ApplyPatchTool)
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let patch = input["patch"].as_str().unwrap_or("");
        let patch_json = input.get("patch_json").filter(|value| !value.is_null());
        let base_dir = input["base_dir"].as_str();

        let start = ctx.clock.mono_now();

        if patch.is_empty() && patch_json.is_none() {
            return ToolResult {
                content: "Error: empty patch content".to_string(),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            };
        }

        let workspace = match ctx.effective_workspace_policy() {
            Ok(workspace) => workspace,
            Err(error) => {
                return tool_error(format!("Refused patch workspace: {error}"), start, ctx);
            }
        };
        let requested_base = match base_dir {
            Some(d) => {
                let p = std::path::Path::new(d);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    workspace.cwd().join(d)
                }
            }
            None => workspace.cwd().to_path_buf(),
        };
        let base_path = match validate_mutation_path(
            &workspace,
            workspace.protected_paths(),
            &requested_base,
        ) {
            Ok(path) => path,
            Err(error) => return tool_error(format!("Refused patch base: {error}"), start, ctx),
        };

        let structured = if let Some(value) = patch_json {
            parse_structured_patch_json(&value.to_string()).map(Some)
        } else if patch.trim_start().starts_with("*** Begin Patch") {
            parse_structured_patch(patch).map(Some)
        } else {
            Ok(None)
        };
        let structured = match structured {
            Ok(value) => value,
            Err(error) => {
                return tool_error(format!("Invalid structured patch: {error}"), start, ctx);
            }
        };

        if let Some(structured) = structured {
            for operation in &structured.operations {
                for relative_path in operation_paths(operation) {
                    if let Err(error) = validate_mutation_path(
                        &workspace,
                        workspace.protected_paths(),
                        &base_path.join(relative_path),
                    ) {
                        return tool_error(
                            format!("Refused structured patch target '{relative_path}': {error}"),
                            start,
                            ctx,
                        );
                    }
                }
            }

            let result = apply_structured_scoped(&structured.operations, &base_path, ctx).await;
            let is_error = !result.failed.is_empty();
            return ToolResult {
                content: serde_json::to_string_pretty(&result)
                    .unwrap_or_else(|error| format!("failed to serialize patch result: {error}")),
                is_error,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: Some(patch_delta(&result)),
                },
            };
        }

        for filename in extract_filenames(patch) {
            if let Err(error) = validate_mutation_path(
                &workspace,
                workspace.protected_paths(),
                &base_path.join(&filename),
            ) {
                return tool_error(
                    format!("Refused patch target '{filename}': {error}"),
                    start,
                    ctx,
                );
            }
        }

        let structured = match platform::structured_patch::parse_unified_diff(patch) {
            Ok(patch) => patch,
            Err(error) => return tool_error(format!("Invalid unified patch: {error}"), start, ctx),
        };
        let result = apply_structured_scoped(&structured.operations, &base_path, ctx).await;
        let is_error = !result.failed.is_empty();
        ToolResult {
            content: serde_json::to_string_pretty(&result)
                .unwrap_or_else(|error| format!("failed to serialize patch result: {error}")),
            is_error,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
                patch_delta: Some(patch_delta(&result)),
            },
        }
    }
}

fn operation_paths(operation: &PatchOperation) -> Vec<&str> {
    match operation {
        PatchOperation::AddFile { path, .. }
        | PatchOperation::DeleteFile { path }
        | PatchOperation::AppendFile { path, .. } => vec![path],
        PatchOperation::UpdateFile { path, move_to, .. } => {
            let mut paths = vec![path.as_str()];
            if let Some(destination) = move_to {
                paths.push(destination.as_str());
            }
            paths
        }
    }
}

fn tool_error(message: String, start: fabric::MonoTime, ctx: &ToolContext) -> ToolResult {
    ToolResult {
        content: message,
        is_error: true,
        metadata: ToolResultMeta {
            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
            truncated: false,
            patch_delta: None,
        },
    }
}

fn patch_delta(result: &super::structured_patch::StructuredPatchResult) -> fabric::PatchDelta {
    fabric::PatchDelta {
        applied: result
            .applied
            .iter()
            .map(|operation| fabric::PatchDeltaApplied {
                operation: operation.op_type.clone(),
                path: operation.path.clone(),
                hunks_applied: operation.hunks_applied,
                bytes_written: operation.bytes_written,
                moved_to: operation.moved_to.clone(),
            })
            .collect(),
        failed: result
            .failed
            .iter()
            .map(|operation| fabric::PatchDeltaFailed {
                operation: operation.op_type.clone(),
                path: operation.path.clone(),
                error: operation.error.clone(),
                hunks_applied_before_failure: operation.hunks_applied_before_failure,
            })
            .collect(),
        files_changed: result
            .files_changed
            .iter()
            .map(|change| fabric::PatchDeltaFileChange {
                path: change.path.clone(),
                change_type: change.change_type.clone(),
                hunks_applied: change.hunks_applied,
                bytes_before: change.bytes_before,
                bytes_after: change.bytes_after,
            })
            .collect(),
    }
}

async fn apply_structured_scoped(
    operations: &[PatchOperation],
    base_dir: &std::path::Path,
    ctx: &ToolContext,
) -> StructuredPatchResult {
    let mut result = StructuredPatchResult {
        applied: Vec::new(),
        failed: Vec::new(),
        files_changed: Vec::new(),
    };
    emit_patch_progress(ctx, "started", None, None, None, None, None);
    for operation in operations {
        let path = operation_paths(operation)[0].to_owned();
        let operation_name = operation_name(operation);
        match apply_structured_operation(operation, base_dir, ctx).await {
            Ok((applied, change)) => {
                result.applied.push(applied);
                result.files_changed.push(change);
                emit_patch_progress(
                    ctx,
                    "file_changed",
                    Some(&path),
                    Some(operation_name),
                    None,
                    None,
                    None,
                );
            }
            Err((error, hunks_applied)) => {
                result.failed.push(FailedOperation {
                    op_type: operation_name.to_string(),
                    path: path.to_string(),
                    error: error.clone(),
                    hunks_applied_before_failure: hunks_applied,
                });
                emit_patch_progress(
                    ctx,
                    "file_failed",
                    Some(&path),
                    Some(operation_name),
                    Some(error),
                    None,
                    None,
                );
            }
        }
    }
    emit_patch_progress(
        ctx,
        "completed",
        None,
        None,
        None,
        Some(result.applied.len()),
        Some(result.failed.len()),
    );
    result
}

async fn apply_structured_operation(
    operation: &PatchOperation,
    base_dir: &std::path::Path,
    ctx: &ToolContext,
) -> Result<(AppliedOperation, FileChangeSummary), (String, Option<usize>)> {
    let failed = |error: String| (error, None);
    match operation {
        PatchOperation::AddFile { path, content } => {
            let target = base_dir.join(path);
            let filesystem =
                scoped_filesystem::open(ctx, &target, platform::FilesystemAccess::ReadWrite)
                    .map_err(failed)?;
            match filesystem.host.metadata(&filesystem.path).await {
                Ok(_) => {
                    return Err(failed(format!(
                        "cannot add '{path}': target already exists"
                    )))
                }
                Err(platform::HostError {
                    kind: platform::HostErrorKind::NotFound(_),
                    ..
                }) => {}
                Err(error) => return Err(failed(format!("cannot inspect '{path}': {error}"))),
            }
            scoped_write(ctx, &target, content.as_bytes().to_vec(), None)
                .await
                .map_err(failed)?;
            Ok((
                AppliedOperation {
                    op_type: "add".into(),
                    path: path.clone(),
                    hunks_applied: None,
                    bytes_written: Some(content.len() as u64),
                    moved_to: None,
                },
                FileChangeSummary {
                    path: path.clone(),
                    change_type: "created".into(),
                    hunks_applied: 0,
                    bytes_before: 0,
                    bytes_after: content.len() as u64,
                },
            ))
        }
        PatchOperation::DeleteFile { path } => {
            let target = base_dir.join(path);
            let (bytes, expected) = scoped_read_for_update(ctx, &target).await.map_err(failed)?;
            scoped_remove(ctx, &target, Some(expected))
                .await
                .map_err(failed)?;
            Ok((
                AppliedOperation {
                    op_type: "delete".into(),
                    path: path.clone(),
                    hunks_applied: None,
                    bytes_written: None,
                    moved_to: None,
                },
                FileChangeSummary {
                    path: path.clone(),
                    change_type: "deleted".into(),
                    hunks_applied: 0,
                    bytes_before: bytes.len() as u64,
                    bytes_after: 0,
                },
            ))
        }
        PatchOperation::AppendFile { path, content } => {
            let target = base_dir.join(path);
            let (mut bytes, expected) =
                scoped_read_for_update(ctx, &target).await.map_err(failed)?;
            let bytes_before = bytes.len() as u64;
            bytes.extend_from_slice(content.as_bytes());
            scoped_write(ctx, &target, bytes.clone(), Some(expected))
                .await
                .map_err(failed)?;
            Ok((
                AppliedOperation {
                    op_type: "append".into(),
                    path: path.clone(),
                    hunks_applied: None,
                    bytes_written: Some(content.len() as u64),
                    moved_to: None,
                },
                FileChangeSummary {
                    path: path.clone(),
                    change_type: "appended".into(),
                    hunks_applied: 0,
                    bytes_before,
                    bytes_after: bytes.len() as u64,
                },
            ))
        }
        PatchOperation::UpdateFile {
            path,
            move_to,
            hunks,
        } => {
            let source = base_dir.join(path);
            let (bytes, expected) = scoped_read_for_update(ctx, &source).await.map_err(failed)?;
            let bytes_before = bytes.len() as u64;
            let existing = String::from_utf8(bytes).map_err(|error| failed(error.to_string()))?;
            let modified = apply_patch_hunks(&existing, hunks)
                .map_err(|(error, applied)| (error, Some(applied)))?;
            let destination = move_to
                .as_ref()
                .map(|path| base_dir.join(path))
                .unwrap_or_else(|| source.clone());
            let write_precondition = if move_to.is_some() {
                None
            } else {
                Some(expected.clone())
            };
            scoped_write(
                ctx,
                &destination,
                modified.as_bytes().to_vec(),
                write_precondition,
            )
            .await
            .map_err(failed)?;
            if move_to.is_some() {
                scoped_remove(ctx, &source, Some(expected))
                    .await
                    .map_err(failed)?;
            }
            let result_path = move_to.as_ref().unwrap_or(path);
            Ok((
                AppliedOperation {
                    op_type: "update".into(),
                    path: path.clone(),
                    hunks_applied: Some(hunks.len()),
                    bytes_written: None,
                    moved_to: move_to.clone(),
                },
                FileChangeSummary {
                    path: result_path.clone(),
                    change_type: if move_to.is_some() {
                        "moved"
                    } else {
                        "modified"
                    }
                    .into(),
                    hunks_applied: hunks.len(),
                    bytes_before,
                    bytes_after: modified.len() as u64,
                },
            ))
        }
    }
}

async fn scoped_read_for_update(
    ctx: &ToolContext,
    path: &std::path::Path,
) -> Result<(Vec<u8>, String), String> {
    let filesystem = scoped_filesystem::open(ctx, path, platform::FilesystemAccess::ReadWrite)?;
    let bytes = filesystem
        .host
        .read(&filesystem.path)
        .await
        .map_err(|error| error.to_string())?;
    let hash = format!("{:x}", Sha256::digest(&bytes));
    Ok((bytes, hash))
}

async fn scoped_write(
    ctx: &ToolContext,
    path: &std::path::Path,
    content: Vec<u8>,
    expected_sha256: Option<String>,
) -> Result<(), String> {
    let filesystem = scoped_filesystem::open(ctx, path, platform::FilesystemAccess::ReadWrite)?;
    if let Some(parent) = filesystem.path.native().parent() {
        filesystem
            .host
            .create_dir_all(&platform::HostPath::new(parent.to_path_buf()))
            .await
            .map_err(|error| error.to_string())?;
    }
    filesystem
        .host
        .atomic_write(platform::AtomicWrite {
            path: filesystem.path,
            content,
            expected_sha256,
            mode: None,
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

async fn scoped_remove(
    ctx: &ToolContext,
    path: &std::path::Path,
    expected_sha256: Option<String>,
) -> Result<(), String> {
    let filesystem = scoped_filesystem::open(ctx, path, platform::FilesystemAccess::ReadWrite)?;
    filesystem
        .host
        .remove_file(platform::RemoveFile {
            path: filesystem.path,
            expected_sha256,
        })
        .await
        .map_err(|error| error.to_string())?;
    Ok(())
}

fn operation_name(operation: &PatchOperation) -> &'static str {
    match operation {
        PatchOperation::AddFile { .. } => "add",
        PatchOperation::DeleteFile { .. } => "delete",
        PatchOperation::UpdateFile { .. } => "update",
        PatchOperation::AppendFile { .. } => "append",
    }
}

#[allow(clippy::too_many_arguments)]
fn emit_patch_progress(
    ctx: &ToolContext,
    status: &str,
    path: Option<&str>,
    operation: Option<&str>,
    error: Option<String>,
    applied_count: Option<usize>,
    failed_count: Option<usize>,
) {
    let Some(sender) = &ctx.turn_event_sender else {
        return;
    };
    if let Err(delivery_error) = sender.send(&fabric::ipc::TurnEventV1::PatchProgress {
        status: status.into(),
        path: path.map(str::to_owned),
        operation: operation.map(str::to_owned),
        error,
        applied_count,
        failed_count,
    }) {
        tracing::warn!(
            ?delivery_error,
            "structured patch progress event was not delivered"
        );
    }
}

/// Extract filenames from unified diff headers.
fn extract_filenames(patch: &str) -> Vec<String> {
    let mut files = Vec::new();
    for line in patch.lines() {
        if let Some(rest) = line.strip_prefix("+++ ") {
            // +++ b/path/to/file or +++ /dev/null (deleted)
            let path = rest
                .split('\t')
                .next()
                .unwrap_or("")
                .trim_start_matches("b/");
            if path != "/dev/null" && !path.is_empty() {
                files.push(path.to_string());
            }
        }
    }
    files.sort();
    files.dedup();
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    use std::sync::Arc;
    use tempfile::TempDir;
    use tokio::fs;

    fn governed_context(root: &std::path::Path, allowed_paths: Vec<String>) -> ToolContext {
        let workspace =
            fabric::WorkspacePolicy::from_resolved_roots(root.to_path_buf(), vec![]).unwrap();
        ToolContext {
            approval_authority: Some(fabric::ToolApprovalAuthority {
                principal_id: fabric::PrincipalId("test".into()),
                connection_id: fabric::ConnectionId::new(),
                thread_id: fabric::ThreadId("test".into()),
                turn_id: fabric::TurnId::new(),
                call_id: "apply-patch".into(),
                workspace,
                granted_scope: fabric::CapabilityScope {
                    allowed_paths,
                    ..Default::default()
                },
            }),
            agent: None,
            working_dir: root.to_path_buf(),
            session_id: "test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        }
    }

    #[tokio::test]
    async fn governed_apply_patch_fails_closed_on_empty_permit_scope() {
        let tmp = TempDir::new().unwrap();
        let context = governed_context(tmp.path(), vec![]);
        let patch = "*** Begin Patch\nAdd File: denied.txt\n>>>\nno\n>>>\n*** End Patch";

        let result = ApplyPatchTool
            .execute(json!({"patch": patch}), &context)
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("empty path scope"));
        assert!(!tmp.path().join("denied.txt").exists());
    }

    #[tokio::test]
    async fn governed_apply_patch_writes_only_inside_the_admitted_workspace() {
        let workspace = TempDir::new().unwrap();
        let external = TempDir::new().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let context = governed_context(&root, vec![root.to_string_lossy().into_owned()]);
        let patch = "*** Begin Patch\nAdd File: admitted.txt\n>>>\nyes\n>>>\n*** End Patch";

        let allowed = ApplyPatchTool
            .execute(json!({"patch": patch}), &context)
            .await;
        let denied = ApplyPatchTool
            .execute(
                json!({"patch": patch, "base_dir": external.path()}),
                &context,
            )
            .await;

        assert!(!allowed.is_error, "{}", allowed.content);
        assert_eq!(
            std::fs::read_to_string(root.join("admitted.txt")).unwrap(),
            "yes"
        );
        assert!(denied.is_error);
        assert!(!external.path().join("admitted.txt").exists());
    }

    #[tokio::test]
    async fn governed_apply_patch_preserves_protected_path_denials() {
        let workspace = TempDir::new().unwrap();
        let root = workspace.path().canonicalize().unwrap();
        let context = governed_context(&root, vec![root.to_string_lossy().into_owned()]);
        let patch = "*** Begin Patch\nAdd File: .env\n>>>\nTOKEN=secret\n>>>\n*** End Patch";

        let result = ApplyPatchTool
            .execute(json!({"patch": patch}), &context)
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("protected") || result.content.contains("sensitive"));
        assert!(!root.join(".env").exists());
    }

    #[tokio::test]
    async fn test_apply_patch_create_file() {
        let tmp = TempDir::new().unwrap();
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: tmp.path().to_path_buf(),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };

        let patch = "--- /dev/null\n+++ b/new_file.txt\n@@ -0,0 +1,3 @@\n+line one\n+line two\n+line three\n";

        let tool = ApplyPatchTool;
        let input = json!({ "patch": patch });
        let result = tool.execute(input, &ctx).await;

        assert!(!result.is_error, "Expected success: {}", result.content);
        let created = fs::read_to_string(tmp.path().join("new_file.txt"))
            .await
            .unwrap();
        assert_eq!(created, "line one\nline two\nline three\n");
    }

    #[tokio::test]
    async fn test_apply_patch_modify_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("existing.txt");
        fs::write(&file_path, "line one\nline two\nline three\n")
            .await
            .unwrap();

        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: tmp.path().to_path_buf(),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };

        let patch = "--- a/existing.txt\n+++ b/existing.txt\n@@ -1,3 +1,3 @@\n line one\n-line two\n+line TWO\n line three\n";

        let tool = ApplyPatchTool;
        let input = json!({ "patch": patch });
        let result = tool.execute(input, &ctx).await;

        assert!(!result.is_error, "Expected success: {}", result.content);
        let modified = fs::read_to_string(&file_path).await.unwrap();
        assert_eq!(modified, "line one\nline TWO\nline three\n");
    }

    #[tokio::test]
    async fn test_apply_patch_delete_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("doomed.txt");
        fs::write(&file_path, "delete me\n").await.unwrap();

        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: tmp.path().to_path_buf(),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };

        let patch = "--- a/doomed.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-delete me\n";

        let tool = ApplyPatchTool;
        let input = json!({ "patch": patch });
        let result = tool.execute(input, &ctx).await;

        assert!(!result.is_error, "Expected success: {}", result.content);
        assert!(!file_path.exists(), "File should have been deleted");
    }

    #[tokio::test]
    async fn structured_apply_returns_typed_patch_delta_metadata() {
        let tmp = TempDir::new().unwrap();
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: tmp.path().to_path_buf(),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let result = ApplyPatchTool
            .execute(
                json!({
                    "patch_json": {
                        "operations": [{
                            "type": "add",
                            "path": "delta.txt",
                            "content": "tracked"
                        }]
                    }
                }),
                &ctx,
            )
            .await;

        assert!(!result.is_error, "{}", result.content);
        let delta = result.metadata.patch_delta.expect("structured delta");
        assert_eq!(delta.applied.len(), 1);
        assert_eq!(delta.applied[0].operation, "add");
        assert_eq!(delta.applied[0].path, "delta.txt");
        assert_eq!(delta.files_changed[0].change_type, "created");
    }

    #[test]
    fn test_extract_filenames() {
        let patch = "--- a/foo.rs\n+++ b/foo.rs\n@@ -1,1 +1,1 @@\n-old\n+new\n--- a/bar.rs\n+++ b/bar.rs\n@@ -1,1 +1,1 @@\n-x\n+y\n";
        let files = extract_filenames(patch);
        assert_eq!(files, vec!["bar.rs", "foo.rs"]);
    }

    #[test]
    fn test_parse_create_patch() {
        let patch = "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1,2 @@\n+hello\n+world\n";
        let parsed = platform::structured_patch::parse_unified_diff(patch).unwrap();
        assert_eq!(parsed.operations.len(), 1);
        assert!(matches!(
            &parsed.operations[0],
            PatchOperation::AddFile { path, content }
                if path == "new.txt" && content == "hello\nworld\n"
        ));
    }

    #[test]
    fn test_parse_empty_patch() {
        let result = platform::structured_patch::parse_unified_diff("");
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_empty_patch_input() {
        let tmp = TempDir::new().unwrap();
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: tmp.path().to_path_buf(),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let tool = ApplyPatchTool;
        let input = json!({ "patch": "" });
        let result = tool.execute(input, &ctx).await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn structured_text_patch_is_applied_without_unified_diff_fallback() {
        let tmp = TempDir::new().unwrap();
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: tmp.path().to_path_buf(),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let patch = "*** Begin Patch\nAdd File: nested/new.txt\n>>>\nhello\n>>>\n*** End Patch";

        let result = ApplyPatchTool
            .execute(json!({ "patch": patch }), &ctx)
            .await;

        assert!(!result.is_error, "Expected success: {}", result.content);
        assert_eq!(
            fs::read_to_string(tmp.path().join("nested/new.txt"))
                .await
                .unwrap(),
            "hello"
        );
    }

    #[tokio::test]
    async fn structured_json_patch_is_applied() {
        let tmp = TempDir::new().unwrap();
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: tmp.path().to_path_buf(),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };

        let result = ApplyPatchTool
            .execute(
                json!({
                    "patch_json": {
                        "operations": [{
                            "type": "add",
                            "path": "json.txt",
                            "content": "from json"
                        }]
                    }
                }),
                &ctx,
            )
            .await;

        assert!(!result.is_error, "Expected success: {}", result.content);
        assert_eq!(
            fs::read_to_string(tmp.path().join("json.txt"))
                .await
                .unwrap(),
            "from json"
        );
    }

    #[test]
    fn test_apply_hunks_basic() {
        let original = "line one\nline two\nline three\n";
        let hunks = vec![platform::structured_patch::PatchHunk {
            old_start: 1,
            old_count: 3,
            new_start: 1,
            new_count: 3,
            content: " line one\n-line two\n+line 2\n line three".into(),
        }];
        let result = platform::structured_patch::apply_patch_hunks(original, &hunks).unwrap();
        assert_eq!(result, "line one\nline 2\nline three\n");
    }

    #[test]
    fn test_apply_hunks_add_lines() {
        let original = "line one\nline three\n";
        let hunks = vec![platform::structured_patch::PatchHunk {
            old_start: 1,
            old_count: 2,
            new_start: 1,
            new_count: 3,
            content: " line one\n+line two\n line three".into(),
        }];
        let result = platform::structured_patch::apply_patch_hunks(original, &hunks).unwrap();
        assert_eq!(result, "line one\nline two\nline three\n");
    }

    #[tokio::test]
    async fn tool_path_uses_streaming_applier_when_sender_is_trusted() {
        let tmp = TempDir::new().unwrap();
        let (mut events, sender) =
            fabric::ipc::TurnEventStream::new(fabric::ipc::StreamConfig::turn_events(8));
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: tmp.path().to_path_buf(),
            session_id: "streaming-tool".into(),
            clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: Some(sender),
        };

        let result = ApplyPatchTool
            .execute(
                json!({
                    "patch_json": {"operations": [{
                        "type": "add", "path": "streamed.txt", "content": "done"
                    }]}
                }),
                &ctx,
            )
            .await;

        assert!(!result.is_error);
        assert!(result.metadata.patch_delta.is_some());
        assert!(matches!(
            events.recv().await.unwrap(),
            fabric::ipc::TurnEventV1::PatchProgress { status, .. } if status == "started"
        ));
        assert!(matches!(
            events.recv().await.unwrap(),
            fabric::ipc::TurnEventV1::PatchProgress { status, path: Some(path), .. }
                if status == "file_changed" && path == "streamed.txt"
        ));
        assert!(matches!(
            events.recv().await.unwrap(),
            fabric::ipc::TurnEventV1::PatchProgress { status, .. } if status == "completed"
        ));
    }
}
