//! Glob tool — list files matching a glob pattern.

use async_trait::async_trait;
use serde_json::json;

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct GlobTool;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "List files matching a glob pattern. Returns relative paths from the root directory."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "Glob pattern to match (e.g. '**/*.rs', '*.txt', 'src/**/*.py')"
                },
                "root": {
                    "type": "string",
                    "description": "Root directory to search from (default: current working directory)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results (default: 200). Values above 2000 are capped."
                }
            },
            "required": ["pattern"]
        })
    }

    fn permission_level(&self) -> PermissionLevel {
        PermissionLevel::L0
    }

    fn boxed_clone(&self) -> Box<dyn Tool> {
        Box::new(GlobTool)
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

        let root = input
            .get("root")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let max_results = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(200)
            .min(2000) as usize;

        // Use walkdir + manual glob matching
        let glob_pattern = match GlobPattern::new(&pattern) {
            Ok(p) => p,
            Err(e) => {
                return ToolResult {
                    content: format!("Error: invalid glob pattern '{}': {}", pattern, e),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
        };

        let mut matches = Vec::new();
        let walker = walkdir::WalkDir::new(&root).follow_links(false);

        for entry in walker {
            if matches.len() >= max_results {
                break;
            }
            let entry = match entry {
                Ok(e) => e,
                Err(_) => continue,
            };
            if !entry.file_type().is_file() {
                continue;
            }
            // Get path relative to root
            let relative = match entry.path().strip_prefix(&root) {
                Ok(r) => r,
                Err(_) => continue,
            };
            let relative_str = relative.to_string_lossy();
            if glob_pattern.matches(&relative_str) {
                matches.push(relative_str.to_string());
            }
        }

        let truncated = matches.len() >= max_results;

        if matches.is_empty() {
            ToolResult {
                content: format!(
                    "No files matching '{}' found in {}",
                    pattern,
                    root.display()
                ),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            }
        } else {
            let content = matches.join("\n");
            ToolResult {
                content,
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated,
                    patch_delta: None,
                },
            }
        }
    }
}

/// Simple glob pattern matcher supporting `*`, `**`, and `?`.
struct GlobPattern {
    pattern: String,
}

impl GlobPattern {
    fn new(pattern: &str) -> Result<Self, String> {
        if pattern.is_empty() {
            return Err("pattern is empty".to_string());
        }
        Ok(Self {
            pattern: pattern.to_string(),
        })
    }

    fn matches(&self, path: &str) -> bool {
        glob_match(&self.pattern, path)
    }
}

/// Match a path against a glob pattern.
/// Supports: `*` (single segment wildcard), `**` (recursive), `?` (single char).
fn glob_match(pattern: &str, path: &str) -> bool {
    let pat: Vec<char> = pattern.chars().collect();
    let path: Vec<char> = path.chars().collect();
    glob_match_rec(&pat, 0, &path, 0)
}

fn glob_match_rec(pat: &[char], pi: usize, path: &[char], pj: usize) -> bool {
    // Base case: both exhausted
    if pi >= pat.len() && pj >= path.len() {
        return true;
    }
    // Pattern exhausted but path remains
    if pi >= pat.len() {
        return false;
    }

    // Handle **
    if pi + 1 < pat.len() && pat[pi] == '*' && pat[pi + 1] == '*' {
        // Skip ** and optional trailing /
        let mut next_pi = pi + 2;
        if next_pi < pat.len() && pat[next_pi] == '/' {
            next_pi += 1;
        }
        // ** can match zero or more path segments
        // Try matching remaining pattern at every remaining position
        for k in pj..=path.len() {
            if glob_match_rec(pat, next_pi, path, k) {
                return true;
            }
        }
        return false;
    }

    // Handle single *
    if pat[pi] == '*' {
        // * matches within one segment (no /)
        let next_pi = pi + 1;
        // Try matching zero or more non-/ characters
        for k in pj..=path.len() {
            if k > pj && path[k - 1] == '/' {
                break; // * doesn't cross /
            }
            if glob_match_rec(pat, next_pi, path, k) {
                return true;
            }
        }
        return false;
    }

    // Handle ?
    if pat[pi] == '?' {
        if pj < path.len() && path[pj] != '/' {
            return glob_match_rec(pat, pi + 1, path, pj + 1);
        }
        return false;
    }

    // Handle literal character
    if pj < path.len() && pat[pi] == path[pj] {
        return glob_match_rec(pat, pi + 1, path, pj + 1);
    }

    false
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[tokio::test]
    async fn test_glob_rs_files() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        // Create test files
        fs::write(root.join("a.rs"), "fn a() {}").unwrap();
        fs::write(root.join("b.txt"), "hello").unwrap();
        fs::create_dir_all(root.join("sub")).unwrap();
        fs::write(root.join("sub/c.rs"), "fn c() {}").unwrap();

        let tool = GlobTool;
        let input = json!({
            "pattern": "**/*.rs",
            "root": root.to_str().unwrap()
        });

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: root.to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(!result.is_error, "Expected success: {}", result.content);
        assert!(
            result.content.contains("a.rs"),
            "Expected a.rs in results: {}",
            result.content
        );
        assert!(
            result.content.contains("sub/c.rs"),
            "Expected sub/c.rs in results: {}",
            result.content
        );
        assert!(
            !result.content.contains("b.txt"),
            "Expected no b.txt: {}",
            result.content
        );
    }

    #[tokio::test]
    async fn test_glob_single_star() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();

        fs::write(root.join("a.rs"), "").unwrap();
        fs::write(root.join("b.rs"), "").unwrap();
        fs::write(root.join("c.txt"), "").unwrap();

        let tool = GlobTool;
        let input = json!({
            "pattern": "*.rs",
            "root": root.to_str().unwrap()
        });

        let result = tool
            .execute(
                input,
                &ToolContext {
                    approval_authority: None,
                    agent: None,
                    working_dir: root.to_path_buf(),
                    session_id: "test".to_string(),
                    clock: std::sync::Arc::new(kernel::chronos::TestClock::default()),
                    turn_event_sender: None,
                },
            )
            .await;

        assert!(!result.is_error);
        assert!(result.content.contains("a.rs"));
        assert!(result.content.contains("b.rs"));
        assert!(!result.content.contains("c.txt"));
    }

    #[tokio::test]
    async fn test_glob_no_matches() {
        let tmp = tempfile::tempdir().unwrap();
        fs::write(tmp.path().join("a.txt"), "").unwrap();

        let tool = GlobTool;
        let input = json!({
            "pattern": "*.rs",
            "root": tmp.path().to_str().unwrap()
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

        assert!(!result.is_error);
        assert!(result.content.contains("No files matching"));
    }

    #[tokio::test]
    async fn test_glob_missing_pattern() {
        let tool = GlobTool;
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

    #[test]
    fn test_glob_pattern_match() {
        assert!(glob_match("*.rs", "foo.rs"));
        assert!(!glob_match("*.rs", "foo.txt"));
        assert!(glob_match("**/*.rs", "src/main.rs"));
        assert!(glob_match("**/*.rs", "a/b/c.rs"));
        assert!(glob_match("*.rs", "bar.rs"));
        assert!(!glob_match("*.rs", "dir/bar.rs"));
        assert!(glob_match("?.rs", "a.rs"));
        assert!(!glob_match("?.rs", "ab.rs"));
    }

    #[test]
    fn test_tool_metadata() {
        let tool = GlobTool;
        assert_eq!(tool.name(), "glob");
        assert_eq!(tool.permission_level(), PermissionLevel::L0);
        assert_eq!(tool.concurrency_class(), ConcurrencyClass::ReadOnly);
    }
}
