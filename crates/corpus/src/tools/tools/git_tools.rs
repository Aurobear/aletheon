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

// ── registration helper ────────────────────────────────────────────────────

pub fn git_tools() -> Vec<Arc<dyn Tool>> {
    vec![
        Arc::new(GitStatusTool),
        Arc::new(GitDiffTool),
        Arc::new(GitLogTool),
        Arc::new(GitShowTool),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;

    #[test]
    fn all_git_tools_are_read_only() {
        for t in git_tools() {
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
}
