use async_trait::async_trait;
use serde_json::json;
use tokio::fs;
use tokio::process::Command;

use super::mutation_path::validate_mutation_path;
use super::{PermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta};

pub struct ApplyPatchTool;

#[async_trait]
impl Tool for ApplyPatchTool {
    fn name(&self) -> &str {
        "apply_patch"
    }

    fn description(&self) -> &str {
        "Apply a unified diff patch to files. Supports creating new files, modifying existing files, and deleting files."
    }

    fn input_schema(&self) -> serde_json::Value {
        json!({
            "type": "object",
            "properties": {
                "patch": {
                    "type": "string",
                    "description": "Unified diff patch content (standard diff format)"
                },
                "base_dir": {
                    "type": "string",
                    "description": "Base directory for applying the patch (default: current dir)"
                }
            },
            "required": ["patch"]
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
        let base_dir = input["base_dir"].as_str();

        let start = ctx.clock.mono_now();

        if patch.is_empty() {
            return ToolResult {
                content: "Error: empty patch content".to_string(),
                is_error: true,
                metadata: ToolResultMeta {
                    execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                    truncated: false,
                },
            };
        }

        let requested_base = match base_dir {
            Some(d) => {
                let p = std::path::Path::new(d);
                if p.is_absolute() {
                    p.to_path_buf()
                } else {
                    ctx.working_dir.join(d)
                }
            }
            None => ctx.working_dir.clone(),
        };
        let base_path = match validate_mutation_path(&ctx.working_dir, &requested_base) {
            Ok(path) => path,
            Err(error) => return tool_error(format!("Refused patch base: {error}"), start, ctx),
        };
        for filename in extract_filenames(patch) {
            if let Err(error) = validate_mutation_path(&ctx.working_dir, &base_path.join(&filename))
            {
                return tool_error(
                    format!("Refused patch target '{filename}': {error}"),
                    start,
                    ctx,
                );
            }
        }

        // Try system `patch` command first for robustness
        match apply_via_patch_command(patch, &base_path).await {
            Ok(output) => {
                let summary = summarize_patch_result(patch, &output);
                ToolResult {
                    content: summary,
                    is_error: false,
                    metadata: ToolResultMeta {
                        execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                        truncated: false,
                    },
                }
            }
            Err(patch_err) => {
                // Fallback: try to parse and apply manually for simple cases
                match apply_patch_native(patch, &base_path).await {
                    Ok(report) => ToolResult {
                        content: report,
                        is_error: false,
                        metadata: ToolResultMeta {
                            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    },
                    Err(native_err) => ToolResult {
                        content: format!(
                            "Failed to apply patch.\nSystem patch: {}\nNative apply: {}",
                            patch_err, native_err
                        ),
                        is_error: true,
                        metadata: ToolResultMeta {
                            execution_time_ms: ctx.clock.mono_now().0.saturating_sub(start.0),
                            truncated: false,
                        },
                    },
                }
            }
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
        },
    }
}

/// Apply patch using the system `patch` command.
async fn apply_via_patch_command(
    patch: &str,
    base_dir: &std::path::Path,
) -> Result<String, String> {
    // Ensure base_dir exists
    fs::create_dir_all(base_dir)
        .await
        .map_err(|e| format!("Failed to create base dir: {}", e))?;

    // Write patch to a temp file since piping stdin with tokio::process is tricky
    let tmp_patch = base_dir.join(".aletheon_patch_tmp");
    fs::write(&tmp_patch, patch)
        .await
        .map_err(|e| format!("Failed to write temp patch file: {}", e))?;

    let result = Command::new("patch")
        .arg("-p1")
        .arg("--directory")
        .arg(base_dir)
        .arg("--input")
        .arg(&tmp_patch)
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::piped())
        .output()
        .await;

    // Clean up temp file
    let _ = fs::remove_file(&tmp_patch).await;

    let output = result.map_err(|e| format!("Failed to run patch: {}", e))?;

    let stderr = String::from_utf8_lossy(&output.stderr).to_string();
    let stdout = String::from_utf8_lossy(&output.stdout).to_string();

    if output.status.success() {
        Ok(format!("{}\n{}", stdout, stderr).trim().to_string())
    } else {
        // Retry with --force (handles creation of new files)
        let tmp_patch2 = base_dir.join(".aletheon_patch_tmp");
        fs::write(&tmp_patch2, patch)
            .await
            .map_err(|e| format!("Failed to write temp patch file (retry): {}", e))?;

        let retry = Command::new("patch")
            .arg("-p1")
            .arg("--force")
            .arg("--directory")
            .arg(base_dir)
            .arg("--input")
            .arg(&tmp_patch2)
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .output()
            .await;

        let _ = fs::remove_file(&tmp_patch2).await;
        let retry = retry.map_err(|e| format!("Failed to run patch retry: {}", e))?;

        if retry.status.success() {
            let out = format!(
                "{}\n{}",
                String::from_utf8_lossy(&retry.stdout),
                String::from_utf8_lossy(&retry.stderr)
            );
            Ok(out.trim().to_string())
        } else {
            Err(format!(
                "patch exited with {}: {}",
                output.status,
                stderr.trim()
            ))
        }
    }
}

/// Summarize patch command output into a human-readable report.
fn summarize_patch_result(patch: &str, output: &str) -> String {
    let files: Vec<String> = extract_filenames(patch);
    let file_list = if files.is_empty() {
        "unknown files".to_string()
    } else {
        files.join(", ")
    };

    format!(
        "Patch applied successfully.\nFiles: {}\nDetails: {}",
        file_list,
        output.trim()
    )
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

/// Native patch application for when system `patch` is unavailable.
/// Handles simple cases: new files, deletions, and basic hunks.
async fn apply_patch_native(patch: &str, base_dir: &std::path::Path) -> Result<String, String> {
    let patches = parse_unified_diff(patch)?;
    let mut reports = Vec::new();

    for fp in patches {
        let full_path = base_dir.join(&fp.filename);

        match fp.patch_type {
            PatchOp::Create => {
                let content = fp.added_lines.join("\n");
                if let Some(parent) = full_path.parent() {
                    fs::create_dir_all(parent)
                        .await
                        .map_err(|e| format!("Create dir failed for {}: {}", fp.filename, e))?;
                }
                fs::write(&full_path, &content)
                    .await
                    .map_err(|e| format!("Write failed for {}: {}", fp.filename, e))?;
                reports.push(format!(
                    "Created: {} ({} lines)",
                    fp.filename,
                    fp.added_lines.len()
                ));
            }
            PatchOp::Delete => {
                if full_path.exists() {
                    fs::remove_file(&full_path)
                        .await
                        .map_err(|e| format!("Delete failed for {}: {}", fp.filename, e))?;
                    reports.push(format!("Deleted: {}", fp.filename));
                } else {
                    return Err(format!("Cannot delete {}: file not found", fp.filename));
                }
            }
            PatchOp::Modify => {
                if !full_path.exists() {
                    return Err(format!("Cannot modify {}: file not found", fp.filename));
                }
                let existing = fs::read_to_string(&full_path)
                    .await
                    .map_err(|e| format!("Read failed for {}: {}", fp.filename, e))?;
                let result = apply_hunks(&existing, &fp.hunks)?;
                fs::write(&full_path, &result)
                    .await
                    .map_err(|e| format!("Write failed for {}: {}", fp.filename, e))?;
                reports.push(format!(
                    "Modified: {} ({} hunks applied)",
                    fp.filename,
                    fp.hunks.len()
                ));
            }
        }
    }

    Ok(reports.join("\n"))
}

enum PatchOp {
    Create,
    Delete,
    Modify,
}

struct FilePatch {
    filename: String,
    patch_type: PatchOp,
    added_lines: Vec<String>,
    hunks: Vec<Hunk>,
}

struct Hunk {
    _old_start: usize,
    _old_count: usize,
    _new_start: usize,
    _new_count: usize,
    lines: Vec<HunkLine>,
}

enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

/// Parse unified diff into per-file patches.
fn parse_unified_diff(patch: &str) -> Result<Vec<FilePatch>, String> {
    let lines: Vec<&str> = patch.lines().collect();
    let mut patches = Vec::new();
    let mut i = 0;

    while i < lines.len() {
        // Look for --- header
        if let Some(old_name) = lines[i].strip_prefix("--- ") {
            i += 1;
            if i >= lines.len() {
                return Err("Unexpected end of patch: missing +++ line".into());
            }
            let new_name = lines[i]
                .strip_prefix("+++ ")
                .ok_or_else(|| format!("Expected +++ but got: {}", lines[i]))?;
            i += 1;

            let old_path = old_name
                .split('\t')
                .next()
                .unwrap_or("")
                .trim_start_matches("a/");
            let new_path = new_name
                .split('\t')
                .next()
                .unwrap_or("")
                .trim_start_matches("b/");

            if old_path == "/dev/null" && new_path != "/dev/null" {
                // Create new file - skip @@ hunk headers, collect + lines
                let mut added_lines = Vec::new();
                while i < lines.len()
                    && !lines[i].starts_with("--- ")
                    && !lines[i].starts_with("diff ")
                {
                    if lines[i].starts_with("@@ ") {
                        // Skip hunk header
                        i += 1;
                        continue;
                    }
                    if let Some(line) = lines[i].strip_prefix('+') {
                        added_lines.push(line.to_string());
                    } else if lines[i].starts_with(' ') {
                        added_lines.push(lines[i][1..].to_string());
                    }
                    // Skip - lines and other non-content lines
                    i += 1;
                }
                patches.push(FilePatch {
                    filename: new_path.to_string(),
                    patch_type: PatchOp::Create,
                    added_lines,
                    hunks: Vec::new(),
                });
            } else if old_path != "/dev/null" && new_path == "/dev/null" {
                // Delete file
                patches.push(FilePatch {
                    filename: old_path.to_string(),
                    patch_type: PatchOp::Delete,
                    added_lines: Vec::new(),
                    hunks: Vec::new(),
                });
                // Skip any hunks for the deleted file
                while i < lines.len()
                    && !lines[i].starts_with("--- ")
                    && !lines[i].starts_with("diff ")
                {
                    i += 1;
                }
            } else if old_path != "/dev/null" && new_path != "/dev/null" {
                // Modify existing file
                let mut hunks = Vec::new();
                while i < lines.len()
                    && !lines[i].starts_with("--- ")
                    && !lines[i].starts_with("diff ")
                {
                    if let Some(hunk_header) = lines[i].strip_prefix("@@ ") {
                        let hunk = parse_hunk_header(hunk_header, &lines, &mut i)?;
                        hunks.push(hunk);
                    } else {
                        i += 1;
                    }
                }
                patches.push(FilePatch {
                    filename: new_path.to_string(),
                    patch_type: PatchOp::Modify,
                    added_lines: Vec::new(),
                    hunks,
                });
            }
        } else {
            i += 1;
        }
    }

    if patches.is_empty() {
        return Err("No valid patch hunks found in patch content".into());
    }

    Ok(patches)
}

fn parse_hunk_header(header: &str, lines: &[&str], idx: &mut usize) -> Result<Hunk, String> {
    // Parse @@ -old_start,old_count +new_start,new_count @@
    let parts: Vec<&str> = header.split(" @@").collect();
    let range = parts[0];
    let ranges: Vec<&str> = range.split_whitespace().collect();
    if ranges.len() < 2 {
        return Err(format!("Invalid hunk header: @@ {} @@", header));
    }

    let parse_range = |s: &str| -> Result<(usize, usize), String> {
        let s = s.trim_start_matches('-').trim_start_matches('+');
        let parts: Vec<&str> = s.split(',').collect();
        let start: usize = parts[0]
            .parse()
            .map_err(|_| format!("Invalid line number: {}", parts[0]))?;
        let count: usize = if parts.len() > 1 {
            parts[1]
                .parse()
                .map_err(|_| format!("Invalid count: {}", parts[1]))?
        } else {
            1
        };
        Ok((start, count))
    };

    let (old_start, old_count) = parse_range(ranges[0])?;
    let (new_start, new_count) = parse_range(ranges[1])?;

    *idx += 1;

    let mut hunk_lines = Vec::new();
    let expected = old_count + new_count;
    let mut consumed = 0;

    while *idx < lines.len() && consumed < expected {
        let line = lines[*idx];
        if line.starts_with("@@ ") || line.starts_with("--- ") || line.starts_with("diff ") {
            break;
        }
        if let Some(rest) = line.strip_prefix('+') {
            hunk_lines.push(HunkLine::Add(rest.to_string()));
            consumed += 1;
        } else if let Some(rest) = line.strip_prefix('-') {
            hunk_lines.push(HunkLine::Remove(rest.to_string()));
            consumed += 1;
        } else if let Some(rest) = line.strip_prefix(' ') {
            hunk_lines.push(HunkLine::Context(rest.to_string()));
            consumed += 1;
        } else if line.is_empty() {
            // Empty lines in patch are context
            hunk_lines.push(HunkLine::Context(String::new()));
            consumed += 1;
        } else {
            break;
        }
        *idx += 1;
    }

    Ok(Hunk {
        _old_start: old_start,
        _old_count: old_count,
        _new_start: new_start,
        _new_count: new_count,
        lines: hunk_lines,
    })
}

/// Apply parsed hunks to existing file content.
fn apply_hunks(content: &str, hunks: &[Hunk]) -> Result<String, String> {
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut offset: isize = 0;

    for hunk in hunks {
        // Find the matching region using context lines
        let match_start = find_hunk_position(&lines, hunk, offset)?;

        // Apply the hunk: remove lines marked with -, add lines marked with +
        let mut new_lines = Vec::new();
        let mut pos = match_start;

        for hunk_line in &hunk.lines {
            match hunk_line {
                HunkLine::Context(expected) => {
                    if pos >= lines.len() || lines[pos] != *expected {
                        return Err(format!(
                            "Context mismatch at line {}: expected {:?}, got {:?}",
                            pos + 1,
                            expected,
                            lines.get(pos)
                        ));
                    }
                    new_lines.push(lines[pos].clone());
                    pos += 1;
                }
                HunkLine::Remove(expected) => {
                    if pos >= lines.len() || lines[pos] != *expected {
                        return Err(format!(
                            "Remove mismatch at line {}: expected {:?}, got {:?}",
                            pos + 1,
                            expected,
                            lines.get(pos)
                        ));
                    }
                    pos += 1;
                }
                HunkLine::Add(line) => {
                    new_lines.push(line.clone());
                }
            }
        }

        let old_count = hunk
            .lines
            .iter()
            .filter(|l| matches!(l, HunkLine::Context(_) | HunkLine::Remove(_)))
            .count();
        let new_count = new_lines.len();

        // Replace the region
        let end = pos;
        lines.splice(match_start..end, new_lines);

        offset = (match_start as isize) + (new_count as isize) - (old_count as isize);
    }

    let mut result = lines.join("\n");
    if content.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

/// Find the position in the file where a hunk should be applied.
fn find_hunk_position(lines: &[String], hunk: &Hunk, offset: isize) -> Result<usize, String> {
    let target = ((hunk._old_start as isize) - 1 + offset).max(0) as usize;

    // Try exact position first
    if matches_hunk_at(lines, hunk, target) {
        return Ok(target);
    }

    // Search nearby (fuzzy matching within +/- 3 lines)
    for delta in 1..=3 {
        if target + delta < lines.len() && matches_hunk_at(lines, hunk, target + delta) {
            return Ok(target + delta);
        }
        if target >= delta && matches_hunk_at(lines, hunk, target - delta) {
            return Ok(target - delta);
        }
    }

    Err(format!(
        "Could not find matching location for hunk starting at line {}",
        hunk._old_start
    ))
}

/// Check if a hunk's context/remove lines match at a given position.
fn matches_hunk_at(lines: &[String], hunk: &Hunk, start: usize) -> bool {
    let mut pos = start;
    for hunk_line in &hunk.lines {
        match hunk_line {
            HunkLine::Context(expected) | HunkLine::Remove(expected) => {
                if pos >= lines.len() || lines[pos] != *expected {
                    return false;
                }
                pos += 1;
            }
            HunkLine::Add(_) => {
                // Add lines don't need to match
            }
        }
    }
    true
}

#[cfg(test)]
mod tests {
    use super::*;

    use tempfile::TempDir;

    #[tokio::test]
    async fn test_apply_patch_create_file() {
        let tmp = TempDir::new().unwrap();
        let ctx = ToolContext {
            approval_authority: None,
            agent: None,
            working_dir: tmp.path().to_path_buf(),
            session_id: "test".to_string(),
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
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
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
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
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        };

        let patch = "--- a/doomed.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-delete me\n";

        let tool = ApplyPatchTool;
        let input = json!({ "patch": patch });
        let result = tool.execute(input, &ctx).await;

        assert!(!result.is_error, "Expected success: {}", result.content);
        assert!(!file_path.exists(), "File should have been deleted");
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
        let patches = parse_unified_diff(patch).unwrap();
        assert_eq!(patches.len(), 1);
        assert_eq!(patches[0].filename, "new.txt");
        assert!(matches!(patches[0].patch_type, PatchOp::Create));
        assert_eq!(patches[0].added_lines, vec!["hello", "world"]);
    }

    #[test]
    fn test_parse_empty_patch() {
        let result = parse_unified_diff("");
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
            clock: std::sync::Arc::new(aletheon_kernel::chronos::TestClock::default()),
        };
        let tool = ApplyPatchTool;
        let input = json!({ "patch": "" });
        let result = tool.execute(input, &ctx).await;
        assert!(result.is_error);
    }

    #[test]
    fn test_apply_hunks_basic() {
        let original = "line one\nline two\nline three\n";
        let hunks = vec![Hunk {
            _old_start: 1,
            _old_count: 3,
            _new_start: 1,
            _new_count: 3,
            lines: vec![
                HunkLine::Context("line one".into()),
                HunkLine::Remove("line two".into()),
                HunkLine::Add("line 2".into()),
                HunkLine::Context("line three".into()),
            ],
        }];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "line one\nline 2\nline three\n");
    }

    #[test]
    fn test_apply_hunks_add_lines() {
        let original = "line one\nline three\n";
        let hunks = vec![Hunk {
            _old_start: 1,
            _old_count: 2,
            _new_start: 1,
            _new_count: 3,
            lines: vec![
                HunkLine::Context("line one".into()),
                HunkLine::Add("line two".into()),
                HunkLine::Context("line three".into()),
            ],
        }];
        let result = apply_hunks(original, &hunks).unwrap();
        assert_eq!(result, "line one\nline two\nline three\n");
    }
}
