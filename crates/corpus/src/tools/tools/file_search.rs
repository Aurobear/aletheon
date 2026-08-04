//! File search tool — search for files and content using ripgrep with fallbacks.

use async_trait::async_trait;
use serde_json::json;

use crate::tools::artifact::ArtifactStore;

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct FileSearchTool;

const MAX_SEARCH_RESULT_BYTES: usize = 24 * 1024;

#[async_trait]
impl Tool for FileSearchTool {
    fn name(&self) -> &str {
        "file_search"
    }

    fn description(&self) -> &str {
        "Search file contents with a targeted regex. Prefer known manifests and precise symbols, then narrow by path/include. Do not use '*' or '**/*' to inventory a repository; use glob with a bounded, specific pattern."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "query": {
                    "type": "string",
                    "description": "Search pattern (regex supported)"
                },
                "path": {
                    "type": "string",
                    "description": "Directory to search in (default: current dir)"
                },
                "include": {
                    "type": "string",
                    "description": "File glob filter (e.g. '*.rs', '*.py')"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Max results to return (default: 50)"
                }
            },
            "required": ["query"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(FileSearchTool)
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let query = match input.get("query").and_then(|v| v.as_str()) {
            Some(q) => q.to_string(),
            None => {
                return ToolResult {
                    content: "Error: 'query' parameter is required".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
        };
        if matches!(query.trim(), "*" | "**" | "**/*" | ".*") {
            return ToolResult {
                content: "Error: broad repository inventory is not a content search. Read known entry files first, or use glob with a specific bounded pattern such as 'crates/*/Cargo.toml' or 'src/**/*.rs'.".to_string(),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            };
        }

        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();

        let include = input
            .get("include")
            .and_then(|v| v.as_str())
            .map(|s| s.to_string());

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        // Strategy 1: Try ripgrep
        if let Some(result) = try_ripgrep(
            &query,
            &path,
            include.as_deref(),
            max_results,
            &*ctx.clock,
            &ctx.working_dir,
        )
        .await
        {
            return result;
        }

        // Strategy 2: Fallback to grep -r
        if let Some(result) = try_grep(
            &query,
            &path,
            include.as_deref(),
            max_results,
            &*ctx.clock,
            &ctx.working_dir,
        )
        .await
        {
            return result;
        }

        // Strategy 3: Fallback to find + grep
        if let Some(result) = try_find_grep(
            &query,
            &path,
            include.as_deref(),
            max_results,
            &*ctx.clock,
            &ctx.working_dir,
        )
        .await
        {
            return result;
        }

        ToolResult {
            content: "Error: No search tool available. Install ripgrep (rg) for best performance:\n  - Ubuntu/Debian: sudo apt install ripgrep\n  - macOS: brew install ripgrep\n  - Arch: sudo pacman -S ripgrep".to_string(),
            is_error: true,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
                patch_delta: None,
            },
        }
    }
}

/// Try ripgrep (rg) for fast searching.
async fn try_ripgrep(
    query: &str,
    path: &str,
    include: Option<&str>,
    max_results: usize,
    clock: &dyn fabric::Clock,
    working_dir: &std::path::Path,
) -> Option<ToolResult> {
    let start = clock.mono_now();
    let mut cmd = tokio::process::Command::new("rg");
    cmd.arg("--no-heading")
        .arg("-n")
        .arg("--max-count")
        .arg(max_results.to_string())
        .arg("--color=never")
        .arg(query)
        .arg(path)
        .current_dir(working_dir);

    if let Some(glob) = include {
        cmd.arg("--glob").arg(glob);
    }

    let output = cmd.output().await.ok()?;
    // rg exit code 1 = no matches (still a valid result), exit code 2+ = error
    let exit_code = output.status.code().unwrap_or(2);
    if exit_code >= 2 && output.stdout.is_empty() {
        return None; // rg error (not installed or invalid args); try next strategy
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let truncated = stdout.lines().count() > max_results;

    Some(structured_search_result(
        query,
        path,
        include,
        max_results,
        "ripgrep",
        &stdout,
        truncated,
        start,
        clock,
    ))
}

fn bound_search_output(content: String) -> (String, bool) {
    if content.len() <= MAX_SEARCH_RESULT_BYTES {
        return (content, false);
    }
    let mut boundary = MAX_SEARCH_RESULT_BYTES;
    while boundary > 0 && !content.is_char_boundary(boundary) {
        boundary -= 1;
    }
    let omitted = content.len() - boundary;
    (
        format!(
            "{}\n... [search output truncated; {} bytes omitted]",
            &content[..boundary],
            omitted
        ),
        true,
    )
}

/// Fallback: grep -r
async fn try_grep(
    query: &str,
    path: &str,
    include: Option<&str>,
    max_results: usize,
    clock: &dyn fabric::Clock,
    working_dir: &std::path::Path,
) -> Option<ToolResult> {
    let start = clock.mono_now();
    let mut cmd = tokio::process::Command::new("grep");
    cmd.arg("-rn").arg("--color=never");

    if let Some(glob) = include {
        cmd.arg("--include").arg(glob);
    }

    cmd.arg(query).arg(path).current_dir(working_dir);

    let output = cmd.output().await.ok()?;
    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let truncated = stdout.lines().count() > max_results;

    Some(structured_search_result(
        query,
        path,
        include,
        max_results,
        "grep",
        &stdout,
        truncated,
        start,
        clock,
    ))
}

/// Fallback: find + grep
async fn try_find_grep(
    query: &str,
    path: &str,
    include: Option<&str>,
    max_results: usize,
    clock: &dyn fabric::Clock,
    working_dir: &std::path::Path,
) -> Option<ToolResult> {
    let start = clock.mono_now();

    // Check if find and grep are available
    let find_check = tokio::process::Command::new("find")
        .arg("--version")
        .output()
        .await;
    if find_check.is_err() {
        return None;
    }

    let mut cmd = tokio::process::Command::new("find");
    cmd.arg(path);

    if let Some(glob) = include {
        cmd.arg("-name").arg(glob);
    }

    cmd.arg("-type").arg("f");
    cmd.arg("-exec")
        .arg("grep")
        .arg("-ln")
        .arg("--color=never")
        .arg(query)
        .arg("{}")
        .arg(";");
    cmd.current_dir(working_dir);

    let output = cmd.output().await.ok()?;
    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let truncated = stdout.lines().count() > max_results;

    Some(structured_search_result(
        query,
        path,
        include,
        max_results,
        "find_grep",
        &stdout,
        truncated,
        start,
        clock,
    ))
}

#[allow(clippy::too_many_arguments)]
fn structured_search_result(
    query: &str,
    path: &str,
    include: Option<&str>,
    max_results: usize,
    engine: &str,
    stdout: &str,
    result_truncated: bool,
    start: fabric::MonoTime,
    clock: &dyn fabric::Clock,
) -> ToolResult {
    let all_lines = stdout.lines().collect::<Vec<_>>();
    let visible = all_lines
        .iter()
        .take(max_results)
        .copied()
        .collect::<Vec<_>>()
        .join("\n");
    let (visible, byte_truncated) = bound_search_output(visible);
    let matches = visible
        .lines()
        .filter(|line| !line.starts_with("... [search output truncated"))
        .map(|line| {
            let mut parts = line.splitn(3, ':');
            let matched_path = parts.next().unwrap_or_default();
            let second = parts.next();
            let third = parts.next();
            let line_number = second.and_then(|value| value.parse::<u64>().ok());
            json!({
                "path": matched_path,
                "line": line_number,
                "text": if line_number.is_some() { third.unwrap_or_default() } else { line },
            })
        })
        .collect::<Vec<_>>();
    let store = ArtifactStore::new(
        super::output::OutputConfig::default()
            .overflow_dir
            .join("artifacts"),
    );
    let artifact_ref = store
        .store(stdout.as_bytes(), "text/plain; charset=utf-8")
        .ok()
        .map(|artifact| artifact.uri());
    let truncated = result_truncated || byte_truncated || all_lines.len() > max_results;
    ToolResult {
        content: json!({
            "kind": "file_search_receipt",
            "query": query,
            "searched_root": path,
            "include": include,
            "engine": engine,
            "max_results": max_results,
            "total_matches_observed": all_lines.len(),
            "returned_matches": matches,
            "truncated": truncated,
            "completeness_boundary": if truncated { "limited_by_result_or_byte_cap" } else { "complete_for_engine_and_scope" },
            "artifact_ref": artifact_ref,
            "message": if all_lines.is_empty() { "No matches found" } else { "Matches found" },
        }).to_string(),
        is_error: false,
        metadata: ToolResultMeta {
            execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
            truncated,
            patch_delta: None,
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_file_search_basic() {
        // Create a temp directory with test files
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("test.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "fn hello_world() {{").unwrap();
        writeln!(f, "    println!(\"hello\");").unwrap();
        writeln!(f, "}}").unwrap();

        let tool = FileSearchTool;
        let input = json!({
            "query": "hello_world",
            "path": tmp.path().to_str().unwrap(),
            "include": "*.rs",
            "max_results": 10
        });

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: tmp.path().to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(
            !result.is_error,
            "Expected success, got: {}",
            result.content
        );
        assert!(
            result.content.contains("hello_world"),
            "Expected match, got: {}",
            result.content
        );
        let receipt: serde_json::Value = serde_json::from_str(&result.content).unwrap();
        assert_eq!(receipt["kind"], "file_search_receipt");
        assert_eq!(receipt["engine"], "ripgrep");
        assert_eq!(
            receipt["completeness_boundary"],
            "complete_for_engine_and_scope"
        );
        assert!(receipt["artifact_ref"]
            .as_str()
            .is_some_and(|reference| reference.starts_with("artifact://sha256/")));
    }

    #[tokio::test]
    async fn test_file_search_no_results() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("empty.rs");
        std::fs::File::create(&file_path).unwrap();

        let tool = FileSearchTool;
        let input = json!({
            "query": "nonexistent_pattern_xyz",
            "path": tmp.path().to_str().unwrap()
        });

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: tmp.path().to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(
            !result.is_error,
            "Expected no-error for empty results: {}",
            result.content
        );
        assert!(result.content.contains("No matches"));
    }

    #[tokio::test]
    async fn test_file_search_missing_query() {
        let tool = FileSearchTool;
        let input = json!({});
        let tmp = tempfile::tempdir().unwrap();

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: tmp.path().to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("required"));
    }

    #[tokio::test]
    async fn rejects_repository_wide_wildcard_as_content_search() {
        let tool = FileSearchTool;
        let tmp = tempfile::tempdir().unwrap();
        let result = tool
            .execute(
                json!({"query": "**/*"}),
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: tmp.path().to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("not a content search"));
    }

    #[test]
    fn bounds_search_output_by_bytes() {
        let input = "结果".repeat(MAX_SEARCH_RESULT_BYTES);
        let (output, truncated) = bound_search_output(input);

        assert!(truncated);
        assert!(output.len() <= MAX_SEARCH_RESULT_BYTES + 80);
        assert!(output.contains("search output truncated"));
    }

    #[tokio::test]
    async fn test_file_search_relative_path_uses_working_dir() {
        // Regression: path="." must resolve against ctx.working_dir, not daemon cwd.
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("marker.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "fn unique_marker_zzz() {{}}").unwrap();

        let tool = FileSearchTool;
        let input = json!({ "query": "unique_marker_zzz", "path": "." });

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: tmp.path().to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(!result.is_error, "got: {}", result.content);
        assert!(
            result.content.contains("unique_marker_zzz"),
            "relative search did not use working_dir: {}",
            result.content
        );
    }

    #[test]
    fn test_tool_metadata() {
        let tool = FileSearchTool;
        assert_eq!(tool.name(), "file_search");
        assert_eq!(tool.permission_level(), PermissionLevel::L0);
        assert_eq!(tool.concurrency_class(), ConcurrencyClass::ReadOnly);
    }
}
