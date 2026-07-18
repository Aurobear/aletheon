use serde::{Deserialize, Serialize};
use std::fs;
use std::path::{Component, Path, PathBuf};

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// A single operation within a structured patch.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum PatchOperation {
    AddFile {
        path: String,
        content: String,
    },
    DeleteFile {
        path: String,
    },
    UpdateFile {
        path: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        move_to: Option<String>,
        #[serde(default, skip_serializing_if = "Vec::is_empty")]
        hunks: Vec<PatchHunk>,
    },
    AppendFile {
        path: String,
        content: String,
    },
}

/// A single hunk within an UpdateFile operation.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub struct PatchHunk {
    pub old_start: u64,
    pub old_count: u64,
    pub new_start: u64,
    pub new_count: u64,
    /// The combined hunk text: lines prefixed with ' ', '-', '+'.
    pub content: String,
}

/// A structured patch containing one or more file-level operations.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredPatch {
    pub operations: Vec<PatchOperation>,
}

/// Result of applying a structured patch, tracking partial progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredPatchResult {
    /// Operations that fully succeeded.
    pub applied: Vec<AppliedOperation>,
    /// Operations that failed.
    pub failed: Vec<FailedOperation>,
    /// Complete list of files that were changed.
    pub files_changed: Vec<FileChangeSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedOperation {
    pub op_type: String, // "add", "update", "delete", "append"
    pub path: String,
    pub hunks_applied: Option<usize>,
    pub bytes_written: Option<u64>,
    pub moved_to: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FailedOperation {
    pub op_type: String,
    pub path: String,
    pub error: String,
    pub hunks_applied_before_failure: Option<usize>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChangeSummary {
    pub path: String,
    pub change_type: String, // "created", "modified", "deleted", "moved", "appended"
    pub hunks_applied: usize,
    pub bytes_before: u64,
    pub bytes_after: u64,
}

// ---------------------------------------------------------------------------
// P1.3: Path validation
// ---------------------------------------------------------------------------

/// Validate a relative path for safety.
///
/// Rejects:
/// - Absolute paths (starting with `/`)
/// - Path traversal (`../` or `..\` segments)
/// - Empty paths
pub fn validate_path(path_str: &str) -> Result<(), String> {
    if path_str.is_empty() {
        return Err("path is empty".to_string());
    }
    if path_str.starts_with('/') {
        return Err(format!(
            "absolute path not allowed: '{}' (use a relative path)",
            path_str
        ));
    }

    let path = Path::new(path_str);
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(format!(
                    "path traversal not allowed: '{}' (contains '..')",
                    path_str
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "absolute or prefixed path not allowed: '{}'",
                    path_str
                ));
            }
            _ => {}
        }
    }
    Ok(())
}

// ---------------------------------------------------------------------------
// P1.1: Text format parser
// ---------------------------------------------------------------------------

const BEGIN_MARKER: &str = "*** Begin Patch";
const END_MARKER: &str = "*** End Patch";
const FENCE: &str = ">>>";

/// Parse a structured patch from the text format (Codex-compatible).
pub fn parse_structured_patch(input: &str) -> Result<StructuredPatch, String> {
    let mut operations: Vec<PatchOperation> = Vec::new();
    let input = input.trim();

    if input.is_empty() {
        return Err("empty patch content".to_string());
    }

    let mut rest = input;

    while !rest.is_empty() {
        // Find the next patch block
        let begin_pos = rest.find(BEGIN_MARKER).ok_or_else(|| {
            if operations.is_empty() {
                "no '*** Begin Patch' marker found".to_string()
            } else {
                format!(
                    "expected '{}' marker after operation index {}",
                    BEGIN_MARKER,
                    operations.len()
                )
            }
        })?;

        let block_start = begin_pos + BEGIN_MARKER.len();

        let end_pos = rest[block_start..].find(END_MARKER).ok_or_else(|| {
            format!(
                "missing '{}' marker for operation index {}",
                END_MARKER,
                operations.len()
            )
        })?;

        let block_end = block_start + end_pos;
        let block_content = &rest[block_start..block_end].trim();

        let operation = parse_patch_block(block_content)?;
        operations.push(operation);

        rest = &rest[block_end + END_MARKER.len()..];
        rest = rest.trim();
    }

    Ok(StructuredPatch { operations })
}

/// Parse a single patch block (text between markers, without the markers).
fn parse_patch_block(block: &str) -> Result<PatchOperation, String> {
    let lines: Vec<&str> = block.lines().collect();
    if lines.is_empty() {
        return Err("empty patch block".to_string());
    }

    let first_line = lines[0].trim();

    if let Some(rest) = first_line.strip_prefix("Delete File:") {
        let path = rest.trim().to_string();
        validate_path(&path)?;
        Ok(PatchOperation::DeleteFile { path })
    } else if let Some(rest) = first_line.strip_prefix("Add File:") {
        let path = rest.trim().to_string();
        validate_path(&path)?;
        let content = extract_fenced_content(block, "Add File")?;
        Ok(PatchOperation::AddFile { path, content })
    } else if let Some(rest) = first_line.strip_prefix("Update File:") {
        let path = rest.trim().to_string();
        validate_path(&path)?;
        let (move_to, hunks) = parse_update_block(block)?;
        Ok(PatchOperation::UpdateFile {
            path,
            move_to,
            hunks,
        })
    } else if let Some(rest) = first_line.strip_prefix("Append File:") {
        let path = rest.trim().to_string();
        validate_path(&path)?;
        let content = extract_fenced_content(block, "Append File")?;
        Ok(PatchOperation::AppendFile { path, content })
    } else {
        Err(format!(
            "unknown operation header: '{}'. Expected one of: Add File:, Delete File:, Update File:, Append File:",
            first_line
        ))
    }
}

/// Extract content between fence markers (`>>>`) from a patch block.
fn extract_fenced_content(block: &str, op_name: &str) -> Result<String, String> {
    let fence_positions: Vec<usize> = block.match_indices(FENCE).map(|(i, _)| i).collect();

    if fence_positions.len() < 2 {
        return Err(format!(
            "{} operation missing content fences ('>>>'): expected at least two fence markers",
            op_name
        ));
    }

    let start = fence_positions[0] + FENCE.len();
    let end = fence_positions[fence_positions.len() - 1];

    let content = block[start..end]
        .trim_start_matches('\n')
        .trim_end_matches('\n');

    // Strip the trailing \n>>> part by trimming trailing whitespace and fence
    // We already have the content between first and last fence.
    // The last fence position gives us the end; we just need to trim to it.
    Ok(content.to_string())
}

/// Parse an UpdateFile block to extract move_to and hunks.
fn parse_update_block(block: &str) -> Result<(Option<String>, Vec<PatchHunk>), String> {
    let lines: Vec<&str> = block.lines().collect();

    // Check for Move to: line (line index 1 or later)
    let mut move_to: Option<String> = None;

    // Search for "Move to:" line after the first line, before "Hunk:" or fences
    for (_idx, line) in lines.iter().enumerate().skip(1) {
        let trimmed = line.trim();
        if let Some(rest) = trimmed.strip_prefix("Move to:") {
            let mv = rest.trim().to_string();
            validate_path(&mv)?;
            move_to = Some(mv);
        }
        // Stop searching when we find the Hunk: line
        if trimmed == "Hunk:" {
            break;
        }
    }

    // Find fence markers for hunk content
    let fence_positions: Vec<usize> = block.match_indices(FENCE).map(|(i, _)| i).collect();

    if fence_positions.len() < 2 {
        return Err(
            "UpdateFile operation missing hunk content fences ('>>>'): expected fence markers around hunk content"
                .to_string(),
        );
    }

    let start = fence_positions[0] + FENCE.len();
    let end = fence_positions[fence_positions.len() - 1];

    let hunk_text = block[start..end]
        .trim_start_matches('\n')
        .trim_end_matches('\n');

    let hunks = parse_hunks(hunk_text)?;

    Ok((move_to, hunks))
}

/// Parse hunk text into a vector of PatchHunk structs.
fn parse_hunks(hunk_text: &str) -> Result<Vec<PatchHunk>, String> {
    let mut hunks: Vec<PatchHunk> = Vec::new();
    let mut current_hunk: Option<(u64, u64, u64, u64, Vec<String>)> = None;

    for line in hunk_text.lines() {
        if line.starts_with("@@ ") {
            // Flush previous hunk
            if let Some((old_start, old_count, new_start, new_count, lines)) = current_hunk.take() {
                hunks.push(PatchHunk {
                    old_start,
                    old_count,
                    new_start,
                    new_count,
                    content: lines.join("\n"),
                });
            }
            // Parse the hunk header: @@ -old_start,old_count +new_start,new_count @@
            current_hunk = Some(parse_hunk_header(line)?);
        } else if let Some((_, _, _, _, ref mut content_lines)) = current_hunk {
            content_lines.push(line.to_string());
        }
    }

    // Flush the last hunk
    if let Some((old_start, old_count, new_start, new_count, lines)) = current_hunk {
        hunks.push(PatchHunk {
            old_start,
            old_count,
            new_start,
            new_count,
            content: lines.join("\n"),
        });
    }

    if hunks.is_empty() {
        return Err("no hunk headers found in UpdateFile block".to_string());
    }

    Ok(hunks)
}

/// Parse a hunk header line: "@@ -old_start,old_count +new_start,new_count @@"
fn parse_hunk_header(line: &str) -> Result<(u64, u64, u64, u64, Vec<String>), String> {
    // Strip the leading "@@ " and trailing " @@"
    let inner = line
        .strip_prefix("@@ ")
        .and_then(|s| s.strip_suffix(" @@"))
        .ok_or_else(|| format!("invalid hunk header: '{}'", line))?;

    let parts: Vec<&str> = inner.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(format!("invalid hunk header: '{}'", line));
    }

    let (old_start, old_count) = parse_hunk_range(parts[0])?;
    let (new_start, new_count) = parse_hunk_range(parts[1])?;

    Ok((old_start, old_count, new_start, new_count, Vec::new()))
}

/// Parse a hunk range like "-10,7" or "+10,8".
fn parse_hunk_range(s: &str) -> Result<(u64, u64), String> {
    // Strip leading '-' or '+'
    let s = s.trim_start_matches(['-', '+']);
    if let Some((start_str, count_str)) = s.split_once(',') {
        let start: u64 = start_str
            .parse()
            .map_err(|_| format!("invalid hunk line number: '{}'", start_str))?;
        let count: u64 = count_str
            .parse()
            .map_err(|_| format!("invalid hunk count: '{}'", count_str))?;
        Ok((start, count))
    } else {
        let start: u64 = s
            .parse()
            .map_err(|_| format!("invalid hunk line number: '{}'", s))?;
        Ok((start, 1))
    }
}

// ---------------------------------------------------------------------------
// P1.2: JSON format parser
// ---------------------------------------------------------------------------

/// Parse a structured patch from JSON format.
pub fn parse_structured_patch_json(input: &str) -> Result<StructuredPatch, String> {
    let raw: serde_json::Value =
        serde_json::from_str(input).map_err(|e| format!("invalid JSON: {}", e))?;

    let ops_array = raw
        .get("operations")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "JSON must contain an 'operations' array".to_string())?;

    let mut operations: Vec<PatchOperation> = Vec::new();

    for (idx, op) in ops_array.iter().enumerate() {
        let op_type = op
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("operation at index {} missing 'type' field", idx))?;

        let path = op
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("operation at index {} missing 'path' field", idx))?;

        validate_path(path)?;

        let operation = match op_type {
            "add" => {
                let content = op.get("content").and_then(|v| v.as_str()).ok_or_else(|| {
                    format!("add operation at index {} missing 'content' field", idx)
                })?;
                PatchOperation::AddFile {
                    path: path.to_string(),
                    content: content.to_string(),
                }
            }
            "delete" => PatchOperation::DeleteFile {
                path: path.to_string(),
            },
            "update" => {
                let move_to = op
                    .get("move_to")
                    .and_then(|v| v.as_str())
                    .map(|s| {
                        validate_path(s)?;
                        Ok::<String, String>(s.to_string())
                    })
                    .transpose()?;
                let hunks = parse_json_hunks(op, idx)?;
                PatchOperation::UpdateFile {
                    path: path.to_string(),
                    move_to,
                    hunks,
                }
            }
            "append" => {
                let content = op.get("content").and_then(|v| v.as_str()).ok_or_else(|| {
                    format!("append operation at index {} missing 'content' field", idx)
                })?;
                PatchOperation::AppendFile {
                    path: path.to_string(),
                    content: content.to_string(),
                }
            }
            other => {
                return Err(format!(
                    "unknown operation type '{}' at index {}. Expected one of: add, update, delete, append",
                    other, idx
                ));
            }
        };

        operations.push(operation);
    }

    Ok(StructuredPatch { operations })
}

/// Parse hunks from a JSON operation object.
fn parse_json_hunks(op: &serde_json::Value, idx: usize) -> Result<Vec<PatchHunk>, String> {
    let hunks_array = op
        .get("hunks")
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("update operation at index {} missing 'hunks' array", idx))?;

    let mut hunks: Vec<PatchHunk> = Vec::new();
    for (h_idx, h) in hunks_array.iter().enumerate() {
        let old_start = h
            .get("old_start")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("hunk at index {}.{} missing 'old_start' field", idx, h_idx))?;
        let old_count = h
            .get("old_count")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("hunk at index {}.{} missing 'old_count' field", idx, h_idx))?;
        let new_start = h
            .get("new_start")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("hunk at index {}.{} missing 'new_start' field", idx, h_idx))?;
        let new_count = h
            .get("new_count")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("hunk at index {}.{} missing 'new_count' field", idx, h_idx))?;
        let content = h
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("hunk at index {}.{} missing 'content' field", idx, h_idx))?;

        hunks.push(PatchHunk {
            old_start,
            old_count,
            new_start,
            new_count,
            content: content.to_string(),
        });
    }

    Ok(hunks)
}

// ---------------------------------------------------------------------------
// P1.4: Execute operations
// ---------------------------------------------------------------------------

/// Apply a structured patch to the filesystem rooted at `workspace_root`.
///
/// Each operation is applied in sequence. Operations that succeed are reported
/// in `applied`. Operations that fail are reported in `failed` and previous
/// successful operations remain on disk.
pub fn execute_structured_patch(
    patch: &StructuredPatch,
    workspace_root: &Path,
) -> StructuredPatchResult {
    let mut result = StructuredPatchResult {
        applied: Vec::new(),
        failed: Vec::new(),
        files_changed: Vec::new(),
    };

    for op in &patch.operations {
        match apply_operation(op, workspace_root) {
            Ok(summary) => {
                result.applied.push(summary_applied(op));
                result.files_changed.push(summary);
            }
            Err(error) => {
                result.failed.push(FailedOperation {
                    op_type: op_type_name(op),
                    path: op_path(op).to_string(),
                    error: error.message,
                    hunks_applied_before_failure: error.hunks_applied,
                });
            }
        }
    }

    result
}

struct OperationFailure {
    message: String,
    hunks_applied: Option<usize>,
}

impl From<String> for OperationFailure {
    fn from(message: String) -> Self {
        Self {
            message,
            hunks_applied: None,
        }
    }
}

/// Apply a single operation to the filesystem.
fn apply_operation(
    op: &PatchOperation,
    workspace_root: &Path,
) -> Result<FileChangeSummary, OperationFailure> {
    match op {
        PatchOperation::AddFile { path, content } => {
            let full_path = resolve_path(workspace_root, path)?;
            if full_path.exists() {
                return Err(format!("cannot add '{}': target already exists", path).into());
            }
            create_parent_dirs(&full_path)?;
            let bytes_before: u64 = 0;
            fs::write(&full_path, content)
                .map_err(|e| format!("failed to write '{}': {}", path, e))?;
            let bytes_after = content.len() as u64;
            Ok(FileChangeSummary {
                path: path.clone(),
                change_type: "created".to_string(),
                hunks_applied: 0,
                bytes_before,
                bytes_after,
            })
        }
        PatchOperation::DeleteFile { path } => {
            let full_path = resolve_path(workspace_root, path)?;
            let metadata = fs::metadata(&full_path)
                .map_err(|_| format!("cannot delete '{}': file not found", path))?;
            if !metadata.is_file() {
                return Err(format!("cannot delete '{}': not a regular file", path).into());
            }
            let bytes_before = metadata.len();
            fs::remove_file(&full_path)
                .map_err(|e| format!("failed to delete '{}': {}", path, e))?;
            Ok(FileChangeSummary {
                path: path.clone(),
                change_type: "deleted".to_string(),
                hunks_applied: 0,
                bytes_before,
                bytes_after: 0,
            })
        }
        PatchOperation::AppendFile { path, content } => {
            let full_path = resolve_path(workspace_root, path)?;
            let existing = fs::read_to_string(&full_path)
                .map_err(|_| format!("cannot append to '{}': file not found", path))?;
            let bytes_before = existing.len() as u64;
            let mut new_content = existing;
            new_content.push_str(content);
            fs::write(&full_path, &new_content)
                .map_err(|e| format!("failed to append to '{}': {}", path, e))?;
            let bytes_after = new_content.len() as u64;
            Ok(FileChangeSummary {
                path: path.clone(),
                change_type: "appended".to_string(),
                hunks_applied: 0,
                bytes_before,
                bytes_after,
            })
        }
        PatchOperation::UpdateFile {
            path,
            move_to,
            hunks,
        } => {
            let full_path = resolve_path(workspace_root, path)?;
            let existing = fs::read_to_string(&full_path)
                .map_err(|_| format!("cannot update '{}': file not found", path))?;
            let bytes_before = existing.len() as u64;
            let modified = apply_patch_hunks(&existing, hunks).map_err(|(message, applied)| {
                OperationFailure {
                    message,
                    hunks_applied: Some(applied),
                }
            })?;
            let hunks_applied = hunks.len();

            if let Some(dest) = move_to {
                let dest_path = resolve_path(workspace_root, dest)?;
                create_parent_dirs(&dest_path)?;
                fs::write(&dest_path, &modified)
                    .map_err(|e| format!("failed to write moved file '{}': {}", dest, e))?;
                fs::remove_file(&full_path).map_err(|e| {
                    format!(
                        "failed to remove original file '{}' after move: {}",
                        path, e
                    )
                })?;
                let bytes_after = modified.len() as u64;
                Ok(FileChangeSummary {
                    path: dest.clone(),
                    change_type: "moved".to_string(),
                    hunks_applied,
                    bytes_before,
                    bytes_after,
                })
            } else {
                fs::write(&full_path, &modified)
                    .map_err(|e| format!("failed to write '{}': {}", path, e))?;
                let bytes_after = modified.len() as u64;
                Ok(FileChangeSummary {
                    path: path.clone(),
                    change_type: "modified".to_string(),
                    hunks_applied,
                    bytes_before,
                    bytes_after,
                })
            }
        }
    }
}

/// Apply hunks to file content using exact matching.
fn apply_patch_hunks(content: &str, hunks: &[PatchHunk]) -> Result<String, (String, usize)> {
    let mut lines: Vec<String> = content.lines().map(|s| s.to_string()).collect();
    let mut offset: isize = 0;

    for (hunk_index, hunk) in hunks.iter().enumerate() {
        let match_start =
            find_hunk_position(&lines, hunk, offset).map_err(|error| (error, hunk_index))?;
        let parsed: Vec<HunkLine> = parse_hunk_lines(&hunk.content);
        let mut new_lines: Vec<String> = Vec::new();
        let mut pos = match_start;
        let orig_pos = pos;

        for hl in &parsed {
            match hl {
                HunkLine::Context(expected) => {
                    if pos >= lines.len() || !lines_match(&lines[pos], expected) {
                        return Err((
                            format!(
                                "context mismatch at line {} in hunk @@ -{},{} +{},{} @@: expected {:?}, got {:?}",
                                pos + 1,
                                hunk.old_start,
                                hunk.old_count,
                                hunk.new_start,
                                hunk.new_count,
                                expected,
                                lines.get(pos)
                            ),
                            hunk_index,
                        ));
                    }
                    new_lines.push(lines[pos].clone());
                    pos += 1;
                }
                HunkLine::Remove(expected) => {
                    if pos >= lines.len() || !lines_match(&lines[pos], expected) {
                        return Err((
                            format!(
                                "remove mismatch at line {} in hunk @@ -{},{} +{},{} @@: expected {:?}, got {:?}",
                                pos + 1,
                                hunk.old_start,
                                hunk.old_count,
                                hunk.new_start,
                                hunk.new_count,
                                expected,
                                lines.get(pos)
                            ),
                            hunk_index,
                        ));
                    }
                    pos += 1;
                }
                HunkLine::Add(line) => {
                    new_lines.push(line.clone());
                }
            }
        }

        let old_count = parsed
            .iter()
            .filter(|l| matches!(l, HunkLine::Context(_) | HunkLine::Remove(_)))
            .count();
        let new_count = new_lines.len();

        lines.splice(orig_pos..pos, new_lines);
        offset = (orig_pos as isize) + (new_count as isize) - (old_count as isize);
    }

    let mut result = lines.join("\n");
    if content.ends_with('\n') {
        result.push('\n');
    }
    Ok(result)
}

/// Internal representation of a single hunk line for application.
#[derive(Debug)]
enum HunkLine {
    Context(String),
    Remove(String),
    Add(String),
}

/// Parse hunk content lines into HunkLine variants.
fn parse_hunk_lines(content: &str) -> Vec<HunkLine> {
    content
        .lines()
        .filter_map(|line| {
            if let Some(rest) = line.strip_prefix(' ') {
                Some(HunkLine::Context(rest.to_string()))
            } else if let Some(rest) = line.strip_prefix('-') {
                Some(HunkLine::Remove(rest.to_string()))
            } else if let Some(rest) = line.strip_prefix('+') {
                Some(HunkLine::Add(rest.to_string()))
            } else if line.is_empty() {
                // Empty lines are context
                Some(HunkLine::Context(String::new()))
            } else {
                None
            }
        })
        .collect()
}

/// Find the position in the file where a hunk should be applied.
fn find_hunk_position(lines: &[String], hunk: &PatchHunk, offset: isize) -> Result<usize, String> {
    let target = ((hunk.old_start as isize) - 1 + offset).max(0) as usize;

    let parsed = parse_hunk_lines(&hunk.content);

    // Try exact position first
    if matches_hunk_at(lines, &parsed, target) {
        return Ok(target);
    }

    // Search nearby (fuzzy matching within +/- 3 lines)
    for delta in 1..=3isize {
        if target as isize + delta < lines.len() as isize
            && matches_hunk_at(lines, &parsed, (target as isize + delta) as usize)
        {
            return Ok((target as isize + delta) as usize);
        }
        if target as isize >= delta
            && matches_hunk_at(lines, &parsed, (target as isize - delta) as usize)
        {
            return Ok((target as isize - delta) as usize);
        }
    }

    // Fall back to a bounded full-file search when model line numbers are stale.
    // Require a complete normalized match to avoid choosing an ambiguous partial
    // context and modifying the wrong region.
    if let Some(position) = seek_sequence(lines, &parsed, 10_000) {
        return Ok(position);
    }

    Err(format!(
        "could not find matching location for hunk @@ -{},{} +{},{} @@",
        hunk.old_start, hunk.old_count, hunk.new_start, hunk.new_count
    ))
}

fn seek_sequence(lines: &[String], parsed: &[HunkLine], max_search_window: usize) -> Option<usize> {
    let consumed_lines = parsed
        .iter()
        .filter(|line| matches!(line, HunkLine::Context(_) | HunkLine::Remove(_)))
        .count();
    if consumed_lines == 0 || consumed_lines > lines.len() {
        return None;
    }

    let last_start = lines.len().saturating_sub(consumed_lines);
    (0..=last_start.min(max_search_window)).find(|start| matches_hunk_at(lines, parsed, *start))
}

/// Check if a hunk's context/remove lines match at a given position.
fn matches_hunk_at(lines: &[String], parsed: &[HunkLine], start: usize) -> bool {
    let mut pos = start;
    for hl in parsed {
        match hl {
            HunkLine::Context(expected) | HunkLine::Remove(expected) => {
                if pos >= lines.len() || !lines_match(&lines[pos], expected) {
                    return false;
                }
                pos += 1;
            }
            HunkLine::Add(_) => {
                // Add lines don't need to match existing file content
            }
        }
    }
    true
}

/// Normalize common model-produced Unicode punctuation and trailing whitespace.
fn normalize_line(line: &str) -> String {
    line.chars()
        .map(|character| match character {
            '\u{2013}' | '\u{2014}' | '\u{2015}' => '-',
            '\u{2018}' | '\u{2019}' => '\'',
            '\u{201c}' | '\u{201d}' => '"',
            other => other,
        })
        .collect::<String>()
        .trim_end()
        .to_string()
}

fn lines_match(actual: &str, expected: &str) -> bool {
    normalize_line(actual) == normalize_line(expected)
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Resolve a relative path against the workspace root, with validation.
fn resolve_path(workspace_root: &Path, path: &str) -> Result<PathBuf, String> {
    validate_path(path)?;
    let resolved = workspace_root.join(path);
    // Ensure the resolved path is still within workspace_root
    let canonical_ws = workspace_root
        .canonicalize()
        .map_err(|e| format!("cannot canonicalize workspace root: {}", e))?;
    // canonicalize the parent if it exists, otherwise canonicalize the
    // nearest existing ancestor
    let resolved_canonical = match resolved.parent() {
        Some(parent) if parent.exists() => parent
            .canonicalize()
            .map_err(|e| format!("cannot resolve path '{}': {}", path, e))?,
        _ => {
            // For new files, check the nearest existing ancestor
            let mut ancestor = resolved.clone();
            while !ancestor.exists() {
                ancestor = ancestor
                    .parent()
                    .ok_or_else(|| format!("invalid path '{}': no existing ancestor", path))?
                    .to_path_buf();
            }
            ancestor
                .canonicalize()
                .map_err(|e| format!("cannot resolve path '{}': {}", path, e))?
        }
    };
    if !resolved_canonical.starts_with(&canonical_ws) {
        return Err(format!(
            "path '{}' escapes workspace root '{}'",
            path,
            workspace_root.display()
        ));
    }
    Ok(resolved)
}

/// Create parent directories for a file path.
fn create_parent_dirs(file_path: &Path) -> Result<(), String> {
    if let Some(parent) = file_path.parent() {
        if !parent.exists() {
            fs::create_dir_all(parent)
                .map_err(|e| format!("failed to create parent directories: {}", e))?;
        }
    }
    Ok(())
}

/// Get the operation type name as a string.
fn op_type_name(op: &PatchOperation) -> String {
    match op {
        PatchOperation::AddFile { .. } => "add".to_string(),
        PatchOperation::DeleteFile { .. } => "delete".to_string(),
        PatchOperation::UpdateFile { .. } => "update".to_string(),
        PatchOperation::AppendFile { .. } => "append".to_string(),
    }
}

/// Get the path from a PatchOperation.
fn op_path(op: &PatchOperation) -> &str {
    match op {
        PatchOperation::AddFile { path, .. }
        | PatchOperation::DeleteFile { path }
        | PatchOperation::UpdateFile { path, .. }
        | PatchOperation::AppendFile { path, .. } => path.as_str(),
    }
}

/// Create an AppliedOperation summary from a PatchOperation.
fn summary_applied(op: &PatchOperation) -> AppliedOperation {
    match op {
        PatchOperation::AddFile { path, content } => AppliedOperation {
            op_type: "add".to_string(),
            path: path.clone(),
            hunks_applied: None,
            bytes_written: Some(content.len() as u64),
            moved_to: None,
        },
        PatchOperation::DeleteFile { path } => AppliedOperation {
            op_type: "delete".to_string(),
            path: path.clone(),
            hunks_applied: None,
            bytes_written: None,
            moved_to: None,
        },
        PatchOperation::UpdateFile {
            path,
            move_to,
            hunks,
        } => AppliedOperation {
            op_type: "update".to_string(),
            path: path.clone(),
            hunks_applied: Some(hunks.len()),
            bytes_written: None,
            moved_to: move_to.clone(),
        },
        PatchOperation::AppendFile { path, content } => AppliedOperation {
            op_type: "append".to_string(),
            path: path.clone(),
            hunks_applied: None,
            bytes_written: Some(content.len() as u64),
            moved_to: None,
        },
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    // -----------------------------------------------------------------------
    // P1.1: Text format parser tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_text_add_file() {
        let input = "\
*** Begin Patch
Add File: src/new_module.rs
Content:
>>>
use std::collections::HashMap;

pub struct NewModule {
    data: HashMap<String, String>,
}
>>>
*** End Patch";

        let patch = parse_structured_patch(input).unwrap();
        assert_eq!(patch.operations.len(), 1);
        match &patch.operations[0] {
            PatchOperation::AddFile { path, content } => {
                assert_eq!(path, "src/new_module.rs");
                assert!(content.contains("pub struct NewModule"));
                assert!(content.contains("use std::collections::HashMap"));
            }
            other => panic!("expected AddFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_text_delete_file() {
        let input = "\
*** Begin Patch
Delete File: src/obsolete.rs
*** End Patch";

        let patch = parse_structured_patch(input).unwrap();
        assert_eq!(patch.operations.len(), 1);
        match &patch.operations[0] {
            PatchOperation::DeleteFile { path } => {
                assert_eq!(path, "src/obsolete.rs");
            }
            other => panic!("expected DeleteFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_text_update_file() {
        let input = "\
*** Begin Patch
Update File: src/existing.rs
Hunk:
>>>
@@ -10,7 +10,8 @@
 fn main() {
-    let x = old_function();
+    let x = new_function();
+    log::info!(\"starting\");
     process(x);
 }
>>>
*** End Patch";

        let patch = parse_structured_patch(input).unwrap();
        assert_eq!(patch.operations.len(), 1);
        match &patch.operations[0] {
            PatchOperation::UpdateFile {
                path,
                ref move_to,
                ref hunks,
            } => {
                assert_eq!(path, "src/existing.rs");
                assert_eq!(*move_to, None);
                assert_eq!(hunks.len(), 1);
                assert_eq!(hunks[0].old_start, 10);
                assert_eq!(hunks[0].old_count, 7);
                assert_eq!(hunks[0].new_start, 10);
                assert_eq!(hunks[0].new_count, 8);
                assert!(hunks[0].content.contains("-    let x = old_function();"));
                assert!(hunks[0].content.contains("+    let x = new_function();"));
            }
            other => panic!("expected UpdateFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_text_update_file_with_move() {
        let input = "\
*** Begin Patch
Update File: src/old_name.rs
Move to: src/new_name.rs
Hunk:
>>>
@@ -1,3 +1,3 @@
-old
+new
 context
>>>
*** End Patch";

        let patch = parse_structured_patch(input).unwrap();
        assert_eq!(patch.operations.len(), 1);
        match &patch.operations[0] {
            PatchOperation::UpdateFile {
                path,
                ref move_to,
                ref hunks,
            } => {
                assert_eq!(path, "src/old_name.rs");
                assert_eq!(move_to.as_deref(), Some("src/new_name.rs"));
                assert_eq!(hunks.len(), 1);
            }
            other => panic!("expected UpdateFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_text_multiple_operations() {
        let input = "\
*** Begin Patch
Add File: src/new.rs
Content:
>>>
hello world
>>>
*** End Patch

*** Begin Patch
Delete File: src/old.rs
*** End Patch";

        let patch = parse_structured_patch(input).unwrap();
        assert_eq!(patch.operations.len(), 2);
        assert!(matches!(
            patch.operations[0],
            PatchOperation::AddFile { .. }
        ));
        assert!(matches!(
            patch.operations[1],
            PatchOperation::DeleteFile { .. }
        ));
    }

    #[test]
    fn test_parse_text_append_file() {
        let input = "\
*** Begin Patch
Append File: src/module.rs
Content:
>>>

// new append content
>>>
*** End Patch";

        let patch = parse_structured_patch(input).unwrap();
        assert_eq!(patch.operations.len(), 1);
        match &patch.operations[0] {
            PatchOperation::AppendFile { path, content } => {
                assert_eq!(path, "src/module.rs");
                assert!(content.contains("// new append content"));
            }
            other => panic!("expected AppendFile, got {:?}", other),
        }
    }

    // -----------------------------------------------------------------------
    // P1.2: JSON format parser tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_json_all_operations() {
        let input = r#"{
  "operations": [
    {
      "type": "add",
      "path": "src/new_module.rs",
      "content": "use std::collections::HashMap;\n\npub struct NewModule {}"
    },
    {
      "type": "update",
      "path": "src/existing.rs",
      "move_to": "src/renamed.rs",
      "hunks": [
        {
          "old_start": 10,
          "old_count": 7,
          "new_start": 10,
          "new_count": 8,
          "content": " fn main() {\n-    let x = old_function();\n+    let x = new_function();\n+    log::info!(\"starting\");\n     process(x);\n"
        }
      ]
    },
    {
      "type": "delete",
      "path": "src/obsolete.rs"
    },
    {
      "type": "append",
      "path": "src/module.rs",
      "content": "\n// new append content\n"
    }
  ]
}"#;

        let patch = parse_structured_patch_json(input).unwrap();
        assert_eq!(patch.operations.len(), 4);
        assert!(matches!(
            patch.operations[0],
            PatchOperation::AddFile { .. }
        ));
        assert!(matches!(
            patch.operations[1],
            PatchOperation::UpdateFile { .. }
        ));
        assert!(matches!(
            patch.operations[2],
            PatchOperation::DeleteFile { .. }
        ));
        assert!(matches!(
            patch.operations[3],
            PatchOperation::AppendFile { .. }
        ));
    }

    #[test]
    fn test_parse_json_update_with_hunks() {
        let input = r#"{
  "operations": [
    {
      "type": "update",
      "path": "src/existing.rs",
      "hunks": [
        {
          "old_start": 10,
          "old_count": 7,
          "new_start": 10,
          "new_count": 8,
          "content": " fn main() {\n-    let x = old_function();\n+    let x = new_function();\n     process(x);\n"
        }
      ]
    }
  ]
}"#;

        let patch = parse_structured_patch_json(input).unwrap();
        assert_eq!(patch.operations.len(), 1);
        match &patch.operations[0] {
            PatchOperation::UpdateFile {
                path,
                ref move_to,
                ref hunks,
            } => {
                assert_eq!(path, "src/existing.rs");
                assert_eq!(*move_to, None);
                assert_eq!(hunks.len(), 1);
                assert_eq!(hunks[0].old_start, 10);
                assert_eq!(hunks[0].old_count, 7);
                assert_eq!(hunks[0].new_start, 10);
                assert_eq!(hunks[0].new_count, 8);
            }
            other => panic!("expected UpdateFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_json_add_file() {
        let input = r#"{
  "operations": [
    {
      "type": "add",
      "path": "src/test.rs",
      "content": "fn main() {}"
    }
  ]
}"#;

        let patch = parse_structured_patch_json(input).unwrap();
        assert_eq!(patch.operations.len(), 1);
        match &patch.operations[0] {
            PatchOperation::AddFile { path, content } => {
                assert_eq!(path, "src/test.rs");
                assert_eq!(content, "fn main() {}");
            }
            other => panic!("expected AddFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_json_delete_file() {
        let input = r#"{
  "operations": [
    {
      "type": "delete",
      "path": "src/obsolete.rs"
    }
  ]
}"#;

        let patch = parse_structured_patch_json(input).unwrap();
        assert_eq!(patch.operations.len(), 1);
        match &patch.operations[0] {
            PatchOperation::DeleteFile { path } => {
                assert_eq!(path, "src/obsolete.rs");
            }
            other => panic!("expected DeleteFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_json_append_file() {
        let input = r#"{
  "operations": [
    {
      "type": "append",
      "path": "src/mod.rs",
      "content": "\n// end of file\n"
    }
  ]
}"#;

        let patch = parse_structured_patch_json(input).unwrap();
        assert_eq!(patch.operations.len(), 1);
        match &patch.operations[0] {
            PatchOperation::AppendFile { path, content } => {
                assert_eq!(path, "src/mod.rs");
                assert!(content.contains("// end of file"));
            }
            other => panic!("expected AppendFile, got {:?}", other),
        }
    }

    #[test]
    fn test_parse_json_missing_operations() {
        let input = r#"{}"#;
        let result = parse_structured_patch_json(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_json_invalid_json() {
        let input = "not json";
        let result = parse_structured_patch_json(input);
        assert!(result.is_err());
    }

    // -----------------------------------------------------------------------
    // P1.3: Path validation tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_validate_path_ok() {
        assert!(validate_path("src/main.rs").is_ok());
        assert!(validate_path("a/b/c.txt").is_ok());
        assert!(validate_path("single_file").is_ok());
    }

    #[test]
    fn test_validate_path_rejects_absolute() {
        assert!(validate_path("/etc/passwd").is_err());
        assert!(validate_path("/root/file").is_err());
    }

    #[test]
    fn test_validate_path_rejects_traversal() {
        assert!(validate_path("../escape").is_err());
        assert!(validate_path("src/../../etc/passwd").is_err());
        assert!(validate_path("foo/bar/../../..").is_err());
    }

    #[test]
    fn test_validate_path_rejects_empty() {
        assert!(validate_path("").is_err());
    }

    // -----------------------------------------------------------------------
    // P1.4: Execute operations tests
    // -----------------------------------------------------------------------

    #[test]
    fn test_execute_add_file() {
        let tmp = TempDir::new().unwrap();
        let patch = StructuredPatch {
            operations: vec![PatchOperation::AddFile {
                path: "new_file.txt".to_string(),
                content: "line one\nline two\n".to_string(),
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(result.failed.is_empty(), "failures: {:?}", result.failed);
        assert_eq!(result.applied.len(), 1);
        assert_eq!(result.files_changed.len(), 1);
        assert_eq!(result.files_changed[0].change_type, "created");

        let created = fs::read_to_string(tmp.path().join("new_file.txt")).unwrap();
        assert_eq!(created, "line one\nline two\n");
    }

    #[test]
    fn test_execute_add_file_creates_parent_dirs() {
        let tmp = TempDir::new().unwrap();
        let patch = StructuredPatch {
            operations: vec![PatchOperation::AddFile {
                path: "a/b/c/file.txt".to_string(),
                content: "nested content\n".to_string(),
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(result.failed.is_empty(), "failures: {:?}", result.failed);

        let created = fs::read_to_string(tmp.path().join("a/b/c/file.txt")).unwrap();
        assert_eq!(created, "nested content\n");
    }

    #[test]
    fn test_execute_add_file_rejects_existing_target() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("existing.txt");
        fs::write(&path, "original").unwrap();
        let patch = StructuredPatch {
            operations: vec![PatchOperation::AddFile {
                path: "existing.txt".to_string(),
                content: "replacement".to_string(),
            }],
        };

        let result = execute_structured_patch(&patch, tmp.path());
        assert_eq!(result.failed.len(), 1);
        assert_eq!(fs::read_to_string(path).unwrap(), "original");
    }

    #[test]
    fn test_execute_delete_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("doomed.txt");
        fs::write(&file_path, "delete me\n").unwrap();

        let patch = StructuredPatch {
            operations: vec![PatchOperation::DeleteFile {
                path: "doomed.txt".to_string(),
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(result.failed.is_empty(), "failures: {:?}", result.failed);
        assert_eq!(result.files_changed[0].change_type, "deleted");
        assert!(!file_path.exists(), "file should be deleted");
    }

    #[test]
    fn test_execute_delete_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let patch = StructuredPatch {
            operations: vec![PatchOperation::DeleteFile {
                path: "nonexistent.txt".to_string(),
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(!result.failed.is_empty());
        assert!(result.applied.is_empty());
    }

    #[test]
    fn test_execute_update_file_modify() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("existing.txt");
        fs::write(&file_path, "line one\nline two\nline three\n").unwrap();

        let patch = StructuredPatch {
            operations: vec![PatchOperation::UpdateFile {
                path: "existing.txt".to_string(),
                move_to: None,
                hunks: vec![PatchHunk {
                    old_start: 1,
                    old_count: 3,
                    new_start: 1,
                    new_count: 3,
                    content: " line one\n-line two\n+line TWO\n line three".to_string(),
                }],
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(result.failed.is_empty(), "failures: {:?}", result.failed);
        assert_eq!(result.files_changed[0].change_type, "modified");
        assert_eq!(result.files_changed[0].hunks_applied, 1);

        let modified = fs::read_to_string(&file_path).unwrap();
        assert_eq!(modified, "line one\nline TWO\nline three\n");
    }

    #[test]
    fn test_execute_update_file_move() {
        let tmp = TempDir::new().unwrap();
        let src_path = tmp.path().join("old.rs");
        let dest_path = tmp.path().join("new.rs");
        fs::write(&src_path, "old content\n").unwrap();

        let patch = StructuredPatch {
            operations: vec![PatchOperation::UpdateFile {
                path: "old.rs".to_string(),
                move_to: Some("new.rs".to_string()),
                hunks: vec![PatchHunk {
                    old_start: 1,
                    old_count: 1,
                    new_start: 1,
                    new_count: 1,
                    content: "-old content\n+new content".to_string(),
                }],
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(result.failed.is_empty(), "failures: {:?}", result.failed);
        assert_eq!(result.files_changed[0].change_type, "moved");

        assert!(!src_path.exists(), "old file should be removed");
        let moved = fs::read_to_string(&dest_path).unwrap();
        assert_eq!(moved, "new content\n");
    }

    #[test]
    fn test_execute_append_file() {
        let tmp = TempDir::new().unwrap();
        let file_path = tmp.path().join("append_me.txt");
        fs::write(&file_path, "original content").unwrap();

        let patch = StructuredPatch {
            operations: vec![PatchOperation::AppendFile {
                path: "append_me.txt".to_string(),
                content: "\nappended line".to_string(),
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(result.failed.is_empty(), "failures: {:?}", result.failed);
        assert_eq!(result.files_changed[0].change_type, "appended");

        let content = fs::read_to_string(&file_path).unwrap();
        assert_eq!(content, "original content\nappended line");
    }

    #[test]
    fn test_execute_partial_success() {
        let tmp = TempDir::new().unwrap();
        // Create a file that the AddFile will successfully create
        // followed by a DeleteFile that will fail (nonexistent file)

        let patch = StructuredPatch {
            operations: vec![
                PatchOperation::AddFile {
                    path: "success.txt".to_string(),
                    content: "hello".to_string(),
                },
                PatchOperation::DeleteFile {
                    path: "nonexistent.txt".to_string(),
                },
            ],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert_eq!(result.applied.len(), 1);
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.applied[0].path, "success.txt");
        assert_eq!(result.failed[0].path, "nonexistent.txt");

        // First file should still be on disk
        let created = fs::read_to_string(tmp.path().join("success.txt")).unwrap();
        assert_eq!(created, "hello");
    }

    #[test]
    fn test_execute_hunk_fuzzy_match_shifted() {
        let tmp = TempDir::new().unwrap();
        // File with extra empty lines at the top shifting target by 2
        let content = "\n\nline one\nline two\nline three\n";
        fs::write(tmp.path().join("shifted.txt"), content).unwrap();

        let patch = StructuredPatch {
            operations: vec![PatchOperation::UpdateFile {
                path: "shifted.txt".to_string(),
                move_to: None,
                hunks: vec![PatchHunk {
                    old_start: 1,
                    old_count: 3,
                    new_start: 1,
                    new_count: 3,
                    content: " line one\n-line two\n+line TWO\n line three".to_string(),
                }],
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(result.failed.is_empty(), "failures: {:?}", result.failed);
        assert_eq!(result.files_changed[0].hunks_applied, 1);

        let modified = fs::read_to_string(tmp.path().join("shifted.txt")).unwrap();
        assert_eq!(modified, "\n\nline one\nline TWO\nline three\n");
    }

    #[test]
    fn test_execute_hunk_seek_sequence_beyond_nearby_window() {
        let tmp = TempDir::new().unwrap();
        let prefix = (0..15).map(|i| format!("prefix {i}\n")).collect::<String>();
        fs::write(
            tmp.path().join("shifted.txt"),
            format!("{prefix}line one\nline two\nline three\n"),
        )
        .unwrap();
        let patch = StructuredPatch {
            operations: vec![PatchOperation::UpdateFile {
                path: "shifted.txt".to_string(),
                move_to: None,
                hunks: vec![PatchHunk {
                    old_start: 1,
                    old_count: 3,
                    new_start: 1,
                    new_count: 3,
                    content: " line one\n-line two\n+line TWO\n line three".to_string(),
                }],
            }],
        };

        let result = execute_structured_patch(&patch, tmp.path());
        assert!(result.failed.is_empty(), "failures: {:?}", result.failed);
        let modified = fs::read_to_string(tmp.path().join("shifted.txt")).unwrap();
        assert!(modified.ends_with("line one\nline TWO\nline three\n"));
    }

    #[test]
    fn test_execute_hunk_normalizes_unicode_punctuation_and_trailing_space() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("unicode.txt");
        fs::write(&path, "say “hello” — now   \n").unwrap();
        let patch = StructuredPatch {
            operations: vec![PatchOperation::UpdateFile {
                path: "unicode.txt".to_string(),
                move_to: None,
                hunks: vec![PatchHunk {
                    old_start: 1,
                    old_count: 1,
                    new_start: 1,
                    new_count: 1,
                    content: "-say \"hello\" - now\n+updated".to_string(),
                }],
            }],
        };

        let result = execute_structured_patch(&patch, tmp.path());
        assert!(result.failed.is_empty(), "failures: {:?}", result.failed);
        assert_eq!(fs::read_to_string(path).unwrap(), "updated\n");
    }

    #[test]
    fn test_failed_later_hunk_reports_progress_without_writing_partial_file() {
        let tmp = TempDir::new().unwrap();
        let path = tmp.path().join("atomic.txt");
        let original = "one\ntwo\nthree\n";
        fs::write(&path, original).unwrap();
        let patch = StructuredPatch {
            operations: vec![PatchOperation::UpdateFile {
                path: "atomic.txt".to_string(),
                move_to: None,
                hunks: vec![
                    PatchHunk {
                        old_start: 1,
                        old_count: 1,
                        new_start: 1,
                        new_count: 1,
                        content: "-one\n+ONE".to_string(),
                    },
                    PatchHunk {
                        old_start: 3,
                        old_count: 1,
                        new_start: 3,
                        new_count: 1,
                        content: "-missing\n+THREE".to_string(),
                    },
                ],
            }],
        };

        let result = execute_structured_patch(&patch, tmp.path());
        assert_eq!(result.failed.len(), 1);
        assert_eq!(result.failed[0].hunks_applied_before_failure, Some(1));
        assert_eq!(fs::read_to_string(path).unwrap(), original);
    }

    // -----------------------------------------------------------------------
    // Edge cases and error handling
    // -----------------------------------------------------------------------

    #[test]
    fn test_parse_empty_input() {
        let result = parse_structured_patch("");
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_no_begin_marker() {
        let input = "Some random text\n*** End Patch";
        let result = parse_structured_patch(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_missing_end_marker() {
        let input = "*** Begin Patch\nAdd File: test.txt\nContent:\n>>>\ncontent\n>>>";
        let result = parse_structured_patch(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_parse_unknown_operation() {
        let input = "\
*** Begin Patch
Unknown Op: test.txt
*** End Patch";
        let result = parse_structured_patch(input);
        assert!(result.is_err());
    }

    #[test]
    fn test_execute_path_traversal_in_add() {
        let tmp = TempDir::new().unwrap();
        let patch = StructuredPatch {
            operations: vec![PatchOperation::AddFile {
                path: "../escape.txt".to_string(),
                content: "bad".to_string(),
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(!result.failed.is_empty());
    }

    #[test]
    fn test_execute_path_escape() {
        let tmp = TempDir::new().unwrap();
        let patch = StructuredPatch {
            operations: vec![PatchOperation::AddFile {
                path: "foo/../../escape.txt".to_string(),
                content: "bad".to_string(),
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(!result.failed.is_empty());
    }

    #[test]
    fn test_execute_update_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let patch = StructuredPatch {
            operations: vec![PatchOperation::UpdateFile {
                path: "nonexistent.txt".to_string(),
                move_to: None,
                hunks: vec![PatchHunk {
                    old_start: 1,
                    old_count: 1,
                    new_start: 1,
                    new_count: 1,
                    content: "-old\n+new".to_string(),
                }],
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(!result.failed.is_empty());
    }

    #[test]
    fn test_execute_append_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let patch = StructuredPatch {
            operations: vec![PatchOperation::AppendFile {
                path: "nonexistent.txt".to_string(),
                content: "data".to_string(),
            }],
        };
        let result = execute_structured_patch(&patch, tmp.path());
        assert!(!result.failed.is_empty());
    }

    #[test]
    fn test_parse_json_missing_hunks_for_update() {
        let input = r#"{
  "operations": [
    {
      "type": "update",
      "path": "src/file.rs"
    }
  ]
}"#;
        let result = parse_structured_patch_json(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("hunks"));
    }

    #[test]
    fn test_parse_json_unknown_operation_type() {
        let input = r#"{
  "operations": [
    {
      "type": "rename",
      "path": "src/file.rs"
    }
  ]
}"#;
        let result = parse_structured_patch_json(input);
        assert!(result.is_err());
        assert!(result.unwrap_err().contains("rename"));
    }
}
