//! Structured Git tools — audit P1: dedicated git operations (no shell passthrough).

use std::sync::Arc;

use async_trait::async_trait;
use serde_json::json;
use tokio::process::Command;

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

// ── git_status ─────────────────────────────────────────────────────────────

pub struct GitStatusTool;

#[async_trait]
impl Tool for GitStatusTool {
    fn name(&self) -> &str {
        "git_status"
    }
    fn description(&self) -> &str {
        "Show working tree status (porcelain format)."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"path":{"type":"string","description":"Repo path (default: working dir)"}},"required":[]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(GitStatusTool)
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let output = Command::new("git")
            .args(["-C", path, "status", "--porcelain"])
            .current_dir(&ctx.working_dir)
            .output()
            .await;
        ToolResult {
            content: output
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_else(|e| format!("git_status error: {e}")),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
                patch_delta: None,
            },
        }
    }
}

// ── git_diff ───────────────────────────────────────────────────────────────

pub struct GitDiffTool;

#[async_trait]
impl Tool for GitDiffTool {
    fn name(&self) -> &str {
        "git_diff"
    }
    fn description(&self) -> &str {
        "Show unstaged changes (diff)."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"path":{"type":"string","description":"Repo path"},"staged":{"type":"boolean","description":"Show staged diff instead (default: false)"}},"required":[]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(GitDiffTool)
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let staged = input
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let mut args = vec!["-C", path, "diff"];
        if staged {
            args.push("--staged");
        }
        let output = Command::new("git")
            .args(&args)
            .current_dir(&ctx.working_dir)
            .output()
            .await;
        ToolResult {
            content: output
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_else(|e| format!("git_diff error: {e}")),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
                patch_delta: None,
            },
        }
    }
}

// ── git_log ────────────────────────────────────────────────────────────────

pub struct GitLogTool;

#[async_trait]
impl Tool for GitLogTool {
    fn name(&self) -> &str {
        "git_log"
    }
    fn description(&self) -> &str {
        "Show recent commit history (oneline format)."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"path":{"type":"string","description":"Repo path"},"count":{"type":"integer","description":"Number of commits (default: 10)"}},"required":[]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(GitLogTool)
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let count = input.get("count").and_then(|v| v.as_u64()).unwrap_or(10);
        let output = Command::new("git")
            .args(["-C", path, "log", "--oneline", &format!("-n{count}")])
            .current_dir(&ctx.working_dir)
            .output()
            .await;
        ToolResult {
            content: output
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_else(|e| format!("git_log error: {e}")),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
                patch_delta: None,
            },
        }
    }
}

// ── git_show ───────────────────────────────────────────────────────────────

pub struct GitShowTool;

#[async_trait]
impl Tool for GitShowTool {
    fn name(&self) -> &str {
        "git_show"
    }
    fn description(&self) -> &str {
        "Show a specific commit (diff + metadata)."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{"path":{"type":"string","description":"Repo path"},"commit":{"type":"string","description":"Commit hash or ref"}},"required":["commit"]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(GitShowTool)
    }
    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let commit = input
            .get("commit")
            .and_then(|v| v.as_str())
            .unwrap_or("HEAD");
        let output = Command::new("git")
            .args(["-C", path, "show", "--stat", commit])
            .current_dir(&ctx.working_dir)
            .output()
            .await;
        ToolResult {
            content: output
                .map(|o| String::from_utf8_lossy(&o.stdout).to_string())
                .unwrap_or_else(|e| format!("git_show error: {e}")),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
                patch_delta: None,
            },
        }
    }
}

// ── git_restore ────────────────────────────────────────────────────────────

/// Restores working-tree (and optionally staged) files to HEAD, discarding
/// uncommitted changes for the given pathspecs. Refuses to run against an
/// empty pathspec unless `all:true` is explicitly passed, to avoid silently
/// nuking the entire working tree.
pub struct GitRestoreTool;

#[async_trait]
impl Tool for GitRestoreTool {
    fn name(&self) -> &str {
        "git_restore"
    }
    fn description(&self) -> &str {
        "Discard uncommitted changes by restoring working-tree files to HEAD. \
         Destructive: edits to the given paths are lost."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{
            "path":{"type":"string","description":"Repo path (default: working dir)"},
            "paths":{"type":"array","items":{"type":"string"},"description":"Pathspecs to restore (required unless all:true)"},
            "staged":{"type":"boolean","description":"Also restore the index (unstage) before restoring the worktree (default: false)"},
            "all":{"type":"boolean","description":"Explicitly restore the entire working tree; required to omit paths"}
        },"required":[]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(GitRestoreTool)
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let staged = input
            .get("staged")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);
        let all = input.get("all").and_then(|v| v.as_bool()).unwrap_or(false);
        let paths: Vec<String> = input
            .get("paths")
            .and_then(|v| v.as_array())
            .map(|a| {
                a.iter()
                    .filter_map(|p| p.as_str().map(String::from))
                    .collect()
            })
            .unwrap_or_default();

        if paths.is_empty() && !all {
            return refused(
                ctx,
                start,
                "git_restore refused: `paths` is empty. Pass explicit pathspecs, or set \
                 `all:true` to discard changes across the entire working tree.",
            );
        }

        let mut args: Vec<&str> = vec!["-C", path, "restore"];
        if staged {
            args.push("--staged");
        }
        args.push("--");
        if all {
            args.push(".");
        }
        for p in &paths {
            args.push(p.as_str());
        }

        let output = Command::new("git")
            .args(&args)
            .current_dir(&ctx.working_dir)
            .output()
            .await;
        git_command_result(ctx, start, "git_restore", output)
    }
}

// ── git_stash ──────────────────────────────────────────────────────────────

pub struct GitStashTool;

#[async_trait]
impl Tool for GitStashTool {
    fn name(&self) -> &str {
        "git_stash"
    }
    fn description(&self) -> &str {
        "Save, list, restore, or drop stashed working-tree changes (git stash push|pop|list|drop)."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{
            "path":{"type":"string","description":"Repo path (default: working dir)"},
            "action":{"type":"string","enum":["push","pop","list","drop"],"description":"Stash subcommand"},
            "message":{"type":"string","description":"Optional message for `push`"}
        },"required":["action"]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(GitStashTool)
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let action = input.get("action").and_then(|v| v.as_str()).unwrap_or("");
        let message = input.get("message").and_then(|v| v.as_str());

        let mut args: Vec<&str> = vec!["-C", path, "stash"];
        match action {
            "push" => {
                args.push("push");
                if let Some(m) = message {
                    args.push("-m");
                    args.push(m);
                }
            }
            "pop" => args.push("pop"),
            "list" => args.push("list"),
            "drop" => args.push("drop"),
            other => {
                return refused(
                    ctx,
                    start,
                    &format!(
                        "git_stash refused: unknown action `{other}` (expected push|pop|list|drop)"
                    ),
                );
            }
        }

        let output = Command::new("git")
            .args(&args)
            .current_dir(&ctx.working_dir)
            .output()
            .await;
        git_command_result(ctx, start, "git_stash", output)
    }
}

// ── git_reset ──────────────────────────────────────────────────────────────

/// Moves HEAD to a target ref. Only `soft` and `mixed` are permitted by
/// default. `hard` discards uncommitted working-tree changes irreversibly and
/// is refused unless `confirm_hard:true` is explicitly passed.
pub struct GitResetTool;

#[async_trait]
impl Tool for GitResetTool {
    fn name(&self) -> &str {
        "git_reset"
    }
    fn description(&self) -> &str {
        "Move HEAD to a target ref. `soft`/`mixed` leave the working tree untouched; \
         `hard` irreversibly discards uncommitted changes and requires confirm_hard:true."
    }
    fn input_schema(&self) -> serde_json::Value {
        json!({"type":"object","properties":{
            "path":{"type":"string","description":"Repo path (default: working dir)"},
            "mode":{"type":"string","enum":["soft","mixed","hard"],"description":"Reset mode. `hard` requires confirm_hard:true"},
            "target":{"type":"string","description":"Target ref, e.g. HEAD~1"},
            "confirm_hard":{"type":"boolean","description":"Must be true to allow mode:hard (irreversibly discards working-tree changes)"}
        },"required":["mode","target"]})
    }
    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L1
    }
    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(GitResetTool)
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();
        let path = input.get("path").and_then(|v| v.as_str()).unwrap_or(".");
        let mode = input.get("mode").and_then(|v| v.as_str()).unwrap_or("");
        let target = input.get("target").and_then(|v| v.as_str()).unwrap_or("");
        let confirm_hard = input
            .get("confirm_hard")
            .and_then(|v| v.as_bool())
            .unwrap_or(false);

        if target.is_empty() {
            return refused(
                ctx,
                start,
                "git_reset refused: `target` is required (e.g. HEAD~1)",
            );
        }
        if !matches!(mode, "soft" | "mixed" | "hard") {
            return refused(
                ctx,
                start,
                &format!("git_reset refused: unknown mode `{mode}` (expected soft|mixed|hard)"),
            );
        }
        if mode == "hard" && !confirm_hard {
            return refused(
                ctx,
                start,
                "git_reset refused: mode `hard` discards uncommitted working-tree changes \
                 irreversibly; pass confirm_hard:true to proceed",
            );
        }

        let output = Command::new("git")
            .args(["-C", path, "reset", &format!("--{mode}"), target])
            .current_dir(&ctx.working_dir)
            .output()
            .await;
        git_command_result(ctx, start, "git_reset", output)
    }
}

// ── shared helpers ───────────────────────────────────────────────────────────

fn refused(ctx: &ToolContext, start: fabric::MonoTime, message: &str) -> ToolResult {
    ToolResult {
        content: message.to_string(),
        is_error: true,
        metadata: ToolResultMeta {
            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
            truncated: false,
            patch_delta: None,
        },
    }
}

/// Renders a completed `Command::output()` into a `ToolResult`, reporting
/// stdout on success and stderr (with the tool name prefixed) on failure or
/// spawn error.
fn git_command_result(
    ctx: &ToolContext,
    start: fabric::MonoTime,
    tool_name: &str,
    output: std::io::Result<std::process::Output>,
) -> ToolResult {
    let elapsed = || ctx.clock.mono_now().0.saturating_sub(start.0);
    match output {
        Ok(o) if o.status.success() => ToolResult {
            content: String::from_utf8_lossy(&o.stdout).to_string(),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: elapsed(),
                truncated: false,
                patch_delta: None,
            },
        },
        Ok(o) => ToolResult {
            content: format!("{tool_name} error: {}", String::from_utf8_lossy(&o.stderr)),
            is_error: true,
            metadata: ToolResultMeta {
                execution_time_ms: elapsed(),
                truncated: false,
                patch_delta: None,
            },
        },
        Err(e) => ToolResult {
            content: format!("{tool_name} error: {e}"),
            is_error: true,
            metadata: ToolResultMeta {
                execution_time_ms: elapsed(),
                truncated: false,
                patch_delta: None,
            },
        },
    }
}

// ── registration helper ────────────────────────────────────────────────────

pub fn git_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(GitStatusTool),
        Arc::new(GitDiffTool),
        Arc::new(GitLogTool),
        Arc::new(GitShowTool),
        Arc::new(GitRestoreTool),
        Arc::new(GitStashTool),
        Arc::new(GitResetTool),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn read_only_git_tools_are_read_only() {
        let read_only: Vec<Box<dyn Tool>> = vec![
            Box::new(GitStatusTool),
            Box::new(GitDiffTool),
            Box::new(GitLogTool),
            Box::new(GitShowTool),
        ];
        for t in read_only {
            assert_eq!(t.permission_level(), PermissionLevel::L0, "{}", t.name());
            assert_eq!(
                t.concurrency_class(),
                ConcurrencyClass::ReadOnly,
                "{}",
                t.name()
            );
        }
    }

    #[test]
    fn write_git_tools_are_l1_side_effect() {
        let writes: Vec<Box<dyn Tool>> = vec![
            Box::new(GitRestoreTool),
            Box::new(GitStashTool),
            Box::new(GitResetTool),
        ];
        for t in writes {
            assert_eq!(t.permission_level(), PermissionLevel::L1, "{}", t.name());
            assert_eq!(
                t.concurrency_class(),
                ConcurrencyClass::SideEffect,
                "{}",
                t.name()
            );
        }
    }

    #[test]
    fn git_tool_names_unique() {
        let names: Vec<_> = git_tools().iter().map(|t| t.name().to_string()).collect();
        let mut deduped = names.clone();
        deduped.sort();
        deduped.dedup();
        assert_eq!(names.len(), deduped.len());
    }

    #[tokio::test]
    async fn git_status_runs() {
        let tool = GitStatusTool;
        let tmp = tempfile::tempdir().unwrap();
        // init a real git repo so git status doesn't error
        tokio::process::Command::new("git")
            .args(["init"])
            .current_dir(tmp.path())
            .output()
            .await
            .unwrap();
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: tmp.path().to_path_buf(),
            session_id: "test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        };
        let result = tool
            .execute(json!({"path": tmp.path().to_str().unwrap()}), &ctx)
            .await;
        assert!(!result.is_error, "{}", result.content);
    }

    async fn init_repo_with_commit(tmp: &std::path::Path) {
        for args in [
            vec!["init"],
            vec!["config", "user.email", "test@example.com"],
            vec!["config", "user.name", "test"],
        ] {
            tokio::process::Command::new("git")
                .args(&args)
                .current_dir(tmp)
                .output()
                .await
                .unwrap();
        }
        tokio::fs::write(tmp.join("a.txt"), "hello\n")
            .await
            .unwrap();
        for args in [vec!["add", "."], vec!["commit", "-m", "init"]] {
            tokio::process::Command::new("git")
                .args(&args)
                .current_dir(tmp)
                .output()
                .await
                .unwrap();
        }
    }

    fn test_ctx(working_dir: std::path::PathBuf) -> ToolContext {
        ToolContext {
            approval_authority: None,
            agent: None,
            working_dir,
            session_id: "test".into(),
            clock: Arc::new(kernel::chronos::TestClock::default()),
            turn_event_sender: None,
        }
    }

    #[tokio::test]
    async fn git_restore_refuses_empty_paths() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_with_commit(tmp.path()).await;
        let ctx = test_ctx(tmp.path().to_path_buf());
        let result = GitRestoreTool
            .execute(json!({"path": tmp.path().to_str().unwrap()}), &ctx)
            .await;
        assert!(result.is_error, "{}", result.content);
    }

    #[tokio::test]
    async fn git_restore_discards_uncommitted_changes() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_with_commit(tmp.path()).await;
        tokio::fs::write(tmp.path().join("a.txt"), "modified\n")
            .await
            .unwrap();
        let ctx = test_ctx(tmp.path().to_path_buf());
        let result = GitRestoreTool
            .execute(
                json!({"path": tmp.path().to_str().unwrap(), "paths": ["a.txt"]}),
                &ctx,
            )
            .await;
        assert!(!result.is_error, "{}", result.content);
        let contents = tokio::fs::read_to_string(tmp.path().join("a.txt"))
            .await
            .unwrap();
        assert_eq!(contents, "hello\n");
    }

    #[tokio::test]
    async fn git_stash_push_and_list() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_with_commit(tmp.path()).await;
        tokio::fs::write(tmp.path().join("a.txt"), "modified\n")
            .await
            .unwrap();
        let ctx = test_ctx(tmp.path().to_path_buf());
        let push = GitStashTool
            .execute(
                json!({"path": tmp.path().to_str().unwrap(), "action": "push"}),
                &ctx,
            )
            .await;
        assert!(!push.is_error, "{}", push.content);
        let list = GitStashTool
            .execute(
                json!({"path": tmp.path().to_str().unwrap(), "action": "list"}),
                &ctx,
            )
            .await;
        assert!(!list.is_error, "{}", list.content);
        assert!(!list.content.trim().is_empty());
    }

    #[tokio::test]
    async fn git_stash_refuses_unknown_action() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_with_commit(tmp.path()).await;
        let ctx = test_ctx(tmp.path().to_path_buf());
        let result = GitStashTool
            .execute(
                json!({"path": tmp.path().to_str().unwrap(), "action": "nuke"}),
                &ctx,
            )
            .await;
        assert!(result.is_error);
    }

    #[tokio::test]
    async fn git_reset_hard_requires_confirmation() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_with_commit(tmp.path()).await;
        let ctx = test_ctx(tmp.path().to_path_buf());
        let result = GitResetTool
            .execute(
                json!({"path": tmp.path().to_str().unwrap(), "mode": "hard", "target": "HEAD"}),
                &ctx,
            )
            .await;
        assert!(result.is_error, "{}", result.content);
    }

    #[tokio::test]
    async fn git_reset_mixed_runs() {
        let tmp = tempfile::tempdir().unwrap();
        init_repo_with_commit(tmp.path()).await;
        let ctx = test_ctx(tmp.path().to_path_buf());
        let result = GitResetTool
            .execute(
                json!({"path": tmp.path().to_str().unwrap(), "mode": "mixed", "target": "HEAD"}),
                &ctx,
            )
            .await;
        assert!(!result.is_error, "{}", result.content);
    }
}
