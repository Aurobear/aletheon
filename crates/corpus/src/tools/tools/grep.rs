//! Grep tool — search for patterns in files using subprocess grep.

use async_trait::async_trait;
use serde_json::json;
use tokio::process::Command;

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct GrepTool;

#[async_trait]
impl Tool for GrepTool {
    fn name(&self) -> &str {
        "grep"
    }

    fn description(&self) -> &str {
        "Search for a pattern in files. Returns matching lines with file paths and line numbers."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Search pattern (regex supported)"
                },
                "path": {
                    "type": "string",
                    "description": "File or directory to search in (default: current working directory)"
                },
                "max_results": {
                    "type": "integer",
                    "description": "Maximum number of matching lines to return (default: 50)"
                }
            },
            "required": ["pattern"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(GrepTool)
    }

    fn concurrency_class(&self) -> ConcurrencyClass {
        ConcurrencyClass::ReadOnly
    }

    async fn execute(&self, input: serde_json::Value, ctx: &ToolContext) -> ToolResult {
        let start = ctx.clock.mono_now();

        let pattern = match input.get("pattern").and_then(|v| v.as_str()) {
            Some(p) => p.to_string(),
            None => {
                return ToolResult {
                    content: "Error: 'pattern' parameter is required".to_string(),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
        };

        let path = input
            .get("path")
            .and_then(|v| v.as_str())
            .unwrap_or(".")
            .to_string();

        let max_results = input
            .get("max_results")
            .and_then(|v| v.as_u64())
            .unwrap_or(50) as usize;

        // Try ripgrep first, fall back to grep
        let result = match try_ripgrep(&pattern, &path, max_results, &ctx.working_dir).await {
            Some(r) => Some(r),
            None => try_grep(&pattern, &path, max_results, &ctx.working_dir).await,
        };

        let elapsed = ctx.clock.mono_now().0.saturating_sub(start.0);

        match result {
            Some(r) => ToolResult {
                content: r.content,
                is_error: r.is_error,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: r.truncated,
                    patch_delta: None,
                },
            },
            None => ToolResult {
                content: "Error: No grep tool available. Install ripgrep (rg) for best performance:\n  - Ubuntu/Debian: sudo apt install ripgrep\n  - macOS: brew install ripgrep\n  - Arch: sudo pacman -S ripgrep".to_string(),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: elapsed,
                    truncated: false,
                    patch_delta: None,
                },
            },
        }
    }
}

struct SubprocessResult {
    content: String,
    is_error: bool,
    truncated: bool,
}

/// Try ripgrep for fast searching.
async fn try_ripgrep(
    pattern: &str,
    path: &str,
    max_results: usize,
    working_dir: &std::path::Path,
) -> Option<SubprocessResult> {
    let output = Command::new("rg")
        .arg("--no-heading")
        .arg("-n")
        .arg("--max-count")
        .arg(max_results.to_string())
        .arg("--color=never")
        .arg(pattern)
        .arg(path)
        .current_dir(working_dir)
        .output()
        .await
        .ok()?;

    let exit_code = output.status.code().unwrap_or(2);
    // rg exit code 1 = no matches, 2+ = error
    if exit_code >= 2 && output.stdout.is_empty() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().collect();
    let truncated = lines.len() >= max_results;

    if lines.is_empty() {
        return Some(SubprocessResult {
            content: format!("No matches found for '{}' in {}", pattern, path),
            is_error: false,
            truncated: false,
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

    Some(SubprocessResult {
        content,
        is_error: false,
        truncated,
    })
}

/// Fallback to grep.
async fn try_grep(
    pattern: &str,
    path: &str,
    max_results: usize,
    working_dir: &std::path::Path,
) -> Option<SubprocessResult> {
    let output = Command::new("grep")
        .arg("-rn")
        .arg("--color=never")
        .arg("--max-count")
        .arg(max_results.to_string())
        .arg(pattern)
        .arg(path)
        .current_dir(working_dir)
        .output()
        .await
        .ok()?;

    if !output.status.success() && output.stdout.is_empty() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let lines: Vec<&str> = stdout.lines().take(max_results).collect();
    let truncated = stdout.lines().count() > max_results;

    if lines.is_empty() {
        return Some(SubprocessResult {
            content: format!("No matches found for '{}' in {}", pattern, path),
            is_error: false,
            truncated: false,
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

    Some(SubprocessResult {
        content,
        is_error: false,
        truncated,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    #[tokio::test]
    async fn test_grep_basic() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("test.rs");
        let mut f = std::fs::File::create(&file_path).unwrap();
        writeln!(f, "fn hello_world() {{").unwrap();
        writeln!(f, "    println!(\"hello\");").unwrap();
        writeln!(f, "}}").unwrap();
        writeln!(f, "fn goodbye() {{").unwrap();
        writeln!(f, "    println!(\"bye\");").unwrap();
        writeln!(f, "}}").unwrap();

        let tool = GrepTool;
        let input = json!({
            "pattern": "hello",
            "path": file_path.to_str().unwrap(),
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
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(!result.is_error, "Expected success: {}", result.content);
        assert!(
            result.content.contains("hello_world"),
            "Expected hello_world match: {}",
            result.content
        );
        assert!(
            result.content.contains("hello"),
            "Expected hello match: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_grep_no_results() {
        let tmp = tempfile::tempdir().unwrap();
        let file_path = tmp.path().join("test.txt");
        std::fs::write(&file_path, "nothing to see here\n").unwrap();

        let tool = GrepTool;
        let input = json!({
            "pattern": "nonexistent_xyz",
            "path": file_path.to_str().unwrap()
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
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(!result.is_error, "Expected no-error: {}", result.content);
        assert!(result.content.contains("No matches"));
    }

    #[tokio::test]
    async fn test_grep_missing_pattern() {
        let tool = GrepTool;
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
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(result.is_error);
        assert!(result.content.contains("required"));
    }

    #[test]
    fn test_tool_metadata() {
        let tool = GrepTool;
        assert_eq!(tool.name(), "grep");
        assert_eq!(tool.permission_level(), PermissionLevel::L0);
        assert_eq!(tool.concurrency_class(), ConcurrencyClass::ReadOnly);
    }
}
