//! Glob tool — list files matching a glob pattern.

use async_trait::async_trait;
use serde_json::json;
use std::collections::BTreeSet;

use super::{ConcurrencyClass, PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct GlobTool;

const DEFAULT_GLOB_RESULTS: usize = 100;
const MAX_GLOB_RESULTS: usize = 200;
const MAX_GLOB_RESULT_BYTES: usize = 24 * 1024;
const MAX_GLOB_PATTERNS: usize = 20;

#[async_trait]
impl Tool for GlobTool {
    fn name(&self) -> &str {
        "glob"
    }

    fn description(&self) -> &str {
        "Discover an unknown path with bounded, specific globs after known files have been read. NEVER submit '**' or '**/*': those policy-rejected patterns do not execute. Do not use glob to begin a repository overview, and do not inventory language or file extensions. In a repository overview, recursive documentation inventories such as 'docs/**/*.md', wildcard-scope inventories such as 'crates/*/tests/**/*.rs', and batches larger than 6 patterns are broad and forbidden; read exact architecture/status paths returned by repo_inspect instead. Use `patterns` only to batch a small set of specific missing paths. Returns deduplicated relative paths from the root directory."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "pattern": {
                    "type": "string",
                    "description": "A bounded, scoped glob such as 'crates/*/Cargo.toml' or 'src/**/*.py'. '**' and '**/*' are forbidden and will be rejected."
                },
                "patterns": {
                    "type": "array",
                    "items": {"type": "string"},
                    "maxItems": 20,
                    "description": "Up to 20 specific missing-path patterns to evaluate together; not for extension inventories."
                },
                "root": {
                    "type": "string",
                    "description": "Root directory to search from (default: current working directory)"
                },
                "limit": {
                    "type": "integer",
                    "description": "Maximum results (default: 100, hard cap: 200)."
                }
            }
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

        let patterns = match parse_patterns(&input) {
            Ok(patterns) => patterns,
            Err(message) => {
                return ToolResult {
                    content: format!("Error: {message}"),
                    is_error: true,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                        patch_delta: None,
                    },
                };
            }
        };
        if let Some(pattern) = patterns
            .iter()
            .find(|pattern| matches!(pattern.trim(), "**" | "**/*"))
        {
            return ToolResult {
                content: format!("Policy guidance: unqualified recursive inventory '{pattern}' was not executed. Read known entry files first, then use scoped patterns such as 'crates/*/Cargo.toml' or 'crates/executive/src/**/*.rs'."),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            };
        }
        let redirected = patterns
            .iter()
            .filter(|pattern| super::overview_guard::broad_pattern(pattern))
            .cloned()
            .collect::<Vec<_>>();
        if super::overview_guard::active(ctx) && !redirected.is_empty() {
            return ToolResult {
                content: json!({
                    "overview_policy_redirect": true,
                    "not_executed": redirected,
                    "guidance": "Read exact paths returned by repo_inspect or identified in returned content. Label evidence not inspected in this turn as unverified."
                })
                .to_string(),
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                    patch_delta: None,
                },
            };
        }

        let root = input
            .get("root")
            .and_then(|v| v.as_str())
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| ctx.working_dir.clone());

        let max_results = input
            .get("limit")
            .and_then(|v| v.as_u64())
            .unwrap_or(DEFAULT_GLOB_RESULTS as u64)
            .min(MAX_GLOB_RESULTS as u64) as usize;

        // Use walkdir + manual glob matching
        let mut glob_patterns = Vec::with_capacity(patterns.len());
        for pattern in &patterns {
            match GlobPattern::new(pattern) {
                Ok(compiled) => glob_patterns.push(compiled),
                Err(e) => {
                    return ToolResult {
                        content: format!("Error: invalid glob pattern '{pattern}': {e}"),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                            patch_delta: None,
                        },
                    }
                }
            }
        }

        let mut matches = BTreeSet::new();
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
            if glob_patterns
                .iter()
                .any(|pattern| pattern.matches(&relative_str))
            {
                matches.insert(relative_str.to_string());
            }
        }

        let truncated = matches.len() >= max_results;

        if matches.is_empty() {
            ToolResult {
                content: format!(
                    "No files matching '{}' found in {}",
                    patterns.join("', '"),
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
            let mut content = matches.into_iter().collect::<Vec<_>>().join("\n");
            let mut byte_truncated = false;
            if content.len() > MAX_GLOB_RESULT_BYTES {
                let mut boundary = MAX_GLOB_RESULT_BYTES;
                while boundary > 0 && !content.is_char_boundary(boundary) {
                    boundary -= 1;
                }
                let omitted = content.len() - boundary;
                content.truncate(boundary);
                content.push_str(&format!(
                    "\n... [glob output truncated; {omitted} bytes omitted]"
                ));
                byte_truncated = true;
            }
            ToolResult {
                content,
                is_error: false,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: truncated || byte_truncated,
                    patch_delta: None,
                },
            }
        }
    }
}

fn parse_patterns(input: &serde_json::Value) -> Result<Vec<String>, String> {
    let mut patterns = Vec::new();
    if let Some(pattern) = input.get("pattern").and_then(|value| value.as_str()) {
        patterns.push(pattern.to_string());
    }
    if let Some(values) = input.get("patterns") {
        let values = values
            .as_array()
            .ok_or_else(|| "'patterns' must be an array of strings".to_string())?;
        if values.len() > MAX_GLOB_PATTERNS {
            return Err(format!(
                "'patterns' accepts at most {MAX_GLOB_PATTERNS} entries"
            ));
        }
        for value in values {
            let pattern = value
                .as_str()
                .ok_or_else(|| "'patterns' must contain only strings".to_string())?;
            patterns.push(pattern.to_string());
        }
    }
    if patterns.is_empty() {
        return Err("'pattern' or non-empty 'patterns' parameter is required".to_string());
    }
    if patterns.len() > MAX_GLOB_PATTERNS {
        return Err(format!(
            "at most {MAX_GLOB_PATTERNS} combined patterns are allowed"
        ));
    }
    Ok(patterns)
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
    async fn batches_multiple_patterns_and_deduplicates_matches() {
        let tmp = tempfile::tempdir().unwrap();
        let root = tmp.path();
        fs::write(root.join("Cargo.toml"), "").unwrap();
        fs::write(root.join("README.md"), "").unwrap();
        fs::write(root.join("ignored.txt"), "").unwrap();

        let result = GlobTool
            .execute(
                json!({
                    "patterns": ["*.toml", "*.md", "README.*"],
                    "root": root
                }),
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

        assert!(!result.is_error, "{}", result.content);
        assert_eq!(result.content, "Cargo.toml\nREADME.md");
    }

    #[tokio::test]
    async fn rejects_unqualified_recursive_inventory() {
        let tmp = tempfile::tempdir().unwrap();
        let tool = GlobTool;
        let result = tool
            .execute(
                json!({"pattern": "**/*", "root": tmp.path()}),
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
        assert!(result.content.starts_with("Policy guidance:"));
        assert!(result.content.contains("was not executed"));
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
