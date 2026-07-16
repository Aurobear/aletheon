//! File search tool — search for files and content using ripgrep with fallbacks.

use async_trait::async_trait;
use serde_json::json;

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct FileSearchTool;

#[async_trait]
impl Tool for FileSearchTool {
    fn name(&self) -> &str {
        "file_search"
    }

    fn description(&self) -> &str {
        "Search for files and content in the filesystem. Supports regex patterns, file type filtering, and path scoping."
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
                    },
                };
            }
        };

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
        if let Some(result) =
            try_ripgrep(&query, &path, include.as_deref(), max_results, &*ctx.clock).await
        {
            return result;
        }

        // Strategy 2: Fallback to grep -r
        if let Some(result) =
            try_grep(&query, &path, include.as_deref(), max_results, &*ctx.clock).await
        {
            return result;
        }

        // Strategy 3: Fallback to find + grep
        if let Some(result) =
            try_find_grep(&query, &path, include.as_deref(), max_results, &*ctx.clock).await
        {
            return result;
        }

        ToolResult {
            content: "Error: No search tool available. Install ripgrep (rg) for best performance:\n  - Ubuntu/Debian: sudo apt install ripgrep\n  - macOS: brew install ripgrep\n  - Arch: sudo pacman -S ripgrep".to_string(),
            is_error: true,
            metadata: ToolResultMeta {
                execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
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
) -> Option<ToolResult> {
    let start = clock.mono_now();
    let mut cmd = tokio::process::Command::new("rg");
    cmd.arg("--no-heading")
        .arg("-n")
        .arg("--max-count")
        .arg(max_results.to_string())
        .arg("--color=never")
        .arg(query)
        .arg(path);

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
    let lines: Vec<&str> = stdout.lines().collect();
    let truncated = lines.len() >= max_results;

    if lines.is_empty() {
        return Some(ToolResult {
            content: format!("No matches found for '{}' in {}", query, path),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
            },
        });
    }

    let content = if truncated {
        format!(
            "{}\n... ({} results shown, more available)",
            lines.join("\n"),
            lines.len()
        )
    } else {
        lines.join("\n")
    };

    Some(ToolResult {
        content,
        is_error: false,
        metadata: ToolResultMeta {
            execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
            truncated,
        },
    })
}

/// Fallback: grep -r
async fn try_grep(
    query: &str,
    path: &str,
    include: Option<&str>,
    max_results: usize,
    clock: &dyn fabric::Clock,
) -> Option<ToolResult> {
    let start = clock.mono_now();
    let mut cmd = tokio::process::Command::new("grep");
    cmd.arg("-rn").arg("--color=never");

    if let Some(glob) = include {
        cmd.arg("--include").arg(glob);
    }

    cmd.arg(query).arg(path);

    let output = cmd.output().await.ok()?;
    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().take(max_results).collect();
    let truncated = stdout.lines().count() > max_results;

    if lines.is_empty() {
        return Some(ToolResult {
            content: format!("No matches found for '{}' in {}", query, path),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
            },
        });
    }

    let content = if truncated {
        format!(
            "{}\n... (showing {} of more results)",
            lines.join("\n"),
            lines.len()
        )
    } else {
        lines.join("\n")
    };

    Some(ToolResult {
        content,
        is_error: false,
        metadata: ToolResultMeta {
            execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
            truncated,
        },
    })
}

/// Fallback: find + grep
async fn try_find_grep(
    query: &str,
    path: &str,
    include: Option<&str>,
    max_results: usize,
    clock: &dyn fabric::Clock,
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

    let output = cmd.output().await.ok()?;
    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().take(max_results).collect();
    let truncated = stdout.lines().count() > max_results;

    if lines.is_empty() {
        return Some(ToolResult {
            content: format!("No files matching '{}' found in {}", query, path),
            is_error: false,
            metadata: ToolResultMeta {
                execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
                truncated: false,
            },
        });
    }

    let content = if truncated {
        format!(
            "{}\n... (showing {} of more results)",
            lines.join("\n"),
            lines.len()
        )
    } else {
        lines.join("\n")
    };

    Some(ToolResult {
        content,
        is_error: false,
        metadata: ToolResultMeta {
            execution_time_ms: clock.mono_now().0.saturating_sub(start.0),
            truncated,
        },
    })
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
                    clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
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
                    clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
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
                    clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("required"));
    }

    #[test]
    fn test_tool_metadata() {
        let tool = FileSearchTool;
        assert_eq!(tool.name(), "file_search");
        assert_eq!(tool.permission_level(), PermissionLevel::L0);
        assert_eq!(tool.concurrency_class(), ConcurrencyClass::ReadOnly);
    }
}
