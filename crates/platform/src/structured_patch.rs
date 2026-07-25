//! Pure structured-patch data model and parser. Filesystem authority remains with caller-provided Platform handles.

use serde::{Deserialize, Serialize};
use std::path::{Component, Path};

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
            "absolute path not allowed: '{path_str}' (use a relative path)"
        ));
    }

    let path = Path::new(path_str);
    for component in path.components() {
        match component {
            Component::ParentDir => {
                return Err(format!(
                    "path traversal not allowed: '{path_str}' (contains '..')"
                ));
            }
            Component::RootDir | Component::Prefix(_) => {
                return Err(format!(
                    "absolute or prefixed path not allowed: '{path_str}'"
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
            "unknown operation header: '{first_line}'. Expected one of: Add File:, Delete File:, Update File:, Append File:"
        ))
    }
}

/// Extract content between fence markers (`>>>`) from a patch block.
fn extract_fenced_content(block: &str, op_name: &str) -> Result<String, String> {
    let fence_positions: Vec<usize> = block.match_indices(FENCE).map(|(i, _)| i).collect();

    if fence_positions.len() < 2 {
        return Err(format!(
            "{op_name} operation missing content fences ('>>>'): expected at least two fence markers"
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
        .ok_or_else(|| format!("invalid hunk header: '{line}'"))?;

    let parts: Vec<&str> = inner.split_whitespace().collect();
    if parts.len() < 2 {
        return Err(format!("invalid hunk header: '{line}'"));
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
            .map_err(|_| format!("invalid hunk line number: '{start_str}'"))?;
        let count: u64 = count_str
            .parse()
            .map_err(|_| format!("invalid hunk count: '{count_str}'"))?;
        Ok((start, count))
    } else {
        let start: u64 = s
            .parse()
            .map_err(|_| format!("invalid hunk line number: '{s}'"))?;
        Ok((start, 1))
    }
}

// ---------------------------------------------------------------------------
// P1.2: JSON format parser
// ---------------------------------------------------------------------------

/// Parse a structured patch from JSON format.
pub fn parse_structured_patch_json(input: &str) -> Result<StructuredPatch, String> {
    let raw: serde_json::Value =
        serde_json::from_str(input).map_err(|e| format!("invalid JSON: {e}"))?;

    let ops_array = raw
        .get("operations")
        .and_then(|v| v.as_array())
        .ok_or_else(|| "JSON must contain an 'operations' array".to_string())?;

    let mut operations: Vec<PatchOperation> = Vec::new();

    for (idx, op) in ops_array.iter().enumerate() {
        let op_type = op
            .get("type")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("operation at index {idx} missing 'type' field"))?;

        let path = op
            .get("path")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("operation at index {idx} missing 'path' field"))?;

        validate_path(path)?;

        let operation = match op_type {
            "add" => {
                let content = op.get("content").and_then(|v| v.as_str()).ok_or_else(|| {
                    format!("add operation at index {idx} missing 'content' field")
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
                    format!("append operation at index {idx} missing 'content' field")
                })?;
                PatchOperation::AppendFile {
                    path: path.to_string(),
                    content: content.to_string(),
                }
            }
            other => {
                return Err(format!(
                    "unknown operation type '{other}' at index {idx}. Expected one of: add, update, delete, append"
                ));
            }
        };

        operations.push(operation);
    }

    Ok(StructuredPatch { operations })
}

/// Parse a conventional unified diff into the canonical structured operations.
/// The parser is intentionally in-process so callers never need to grant an
/// ambient-filesystem `patch` subprocess authority.
pub fn parse_unified_diff(input: &str) -> Result<StructuredPatch, String> {
    let lines = input.lines().collect::<Vec<_>>();
    let mut operations = Vec::new();
    let mut index = 0;
    while index < lines.len() {
        let Some(old_header) = lines[index].strip_prefix("--- ") else {
            index += 1;
            continue;
        };
        index += 1;
        let new_header = lines
            .get(index)
            .and_then(|line| line.strip_prefix("+++ "))
            .ok_or_else(|| "unified diff is missing its +++ header".to_string())?;
        index += 1;
        let old_path = diff_path(old_header, "a/")?;
        let new_path = diff_path(new_header, "b/")?;
        let mut hunks = Vec::new();
        while index < lines.len() && !lines[index].starts_with("--- ") {
            if let Some(header) = lines[index].strip_prefix("@@ ") {
                let (old_start, old_count, new_start, new_count) =
                    parse_unified_hunk_header(header)?;
                index += 1;
                let mut content = Vec::new();
                while index < lines.len()
                    && !lines[index].starts_with("@@ ")
                    && !lines[index].starts_with("--- ")
                {
                    let line = lines[index];
                    if line.starts_with([' ', '+', '-']) {
                        content.push(line);
                    } else if line == "\\ No newline at end of file" {
                        // Marker is metadata, not file content.
                    } else {
                        break;
                    }
                    index += 1;
                }
                hunks.push(PatchHunk {
                    old_start,
                    old_count,
                    new_start,
                    new_count,
                    content: content.join("\n"),
                });
            } else {
                index += 1;
            }
        }
        match (old_path.as_deref(), new_path.as_deref()) {
            (None, Some(path)) => {
                let mut content = hunks
                    .iter()
                    .flat_map(|hunk| hunk.content.lines())
                    .filter_map(|line| line.strip_prefix('+'))
                    .collect::<Vec<_>>()
                    .join("\n");
                if input.ends_with('\n') {
                    content.push('\n');
                }
                operations.push(PatchOperation::AddFile {
                    path: path.into(),
                    content,
                });
            }
            (Some(path), None) => operations.push(PatchOperation::DeleteFile { path: path.into() }),
            (Some(_), Some(path)) => operations.push(PatchOperation::UpdateFile {
                path: path.into(),
                move_to: None,
                hunks,
            }),
            (None, None) => return Err("unified diff cannot use /dev/null for both paths".into()),
        }
    }
    if operations.is_empty() {
        return Err("no unified diff file headers found".into());
    }
    Ok(StructuredPatch { operations })
}

fn diff_path(header: &str, prefix: &str) -> Result<Option<String>, String> {
    let path = header.split('\t').next().unwrap_or("").trim();
    if path == "/dev/null" {
        return Ok(None);
    }
    let path = path.strip_prefix(prefix).unwrap_or(path);
    validate_path(path)?;
    Ok(Some(path.to_string()))
}

fn parse_unified_hunk_header(header: &str) -> Result<(u64, u64, u64, u64), String> {
    let range = header.split(" @@").next().unwrap_or(header);
    let mut ranges = range.split_whitespace();
    let old = ranges
        .next()
        .ok_or_else(|| format!("invalid unified hunk header: {header}"))?;
    let new = ranges
        .next()
        .ok_or_else(|| format!("invalid unified hunk header: {header}"))?;
    let (old_start, old_count) = parse_unified_range(old, '-')?;
    let (new_start, new_count) = parse_unified_range(new, '+')?;
    Ok((old_start, old_count, new_start, new_count))
}

fn parse_unified_range(value: &str, sign: char) -> Result<(u64, u64), String> {
    let value = value
        .strip_prefix(sign)
        .ok_or_else(|| format!("unified range '{value}' is missing '{sign}'"))?;
    let mut parts = value.split(',');
    let start = parts
        .next()
        .ok_or_else(|| "unified range is empty".to_string())?
        .parse::<u64>()
        .map_err(|_| format!("invalid unified range: {value}"))?;
    let count = parts
        .next()
        .map(str::parse::<u64>)
        .transpose()
        .map_err(|_| format!("invalid unified range: {value}"))?
        .unwrap_or(1);
    Ok((start, count))
}

/// Parse hunks from a JSON operation object.
fn parse_json_hunks(op: &serde_json::Value, idx: usize) -> Result<Vec<PatchHunk>, String> {
    let hunks_array = op
        .get("hunks")
        .and_then(|v| v.as_array())
        .ok_or_else(|| format!("update operation at index {idx} missing 'hunks' array"))?;

    let mut hunks: Vec<PatchHunk> = Vec::new();
    for (h_idx, h) in hunks_array.iter().enumerate() {
        let old_start = h
            .get("old_start")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("hunk at index {idx}.{h_idx} missing 'old_start' field"))?;
        let old_count = h
            .get("old_count")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("hunk at index {idx}.{h_idx} missing 'old_count' field"))?;
        let new_start = h
            .get("new_start")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("hunk at index {idx}.{h_idx} missing 'new_start' field"))?;
        let new_count = h
            .get("new_count")
            .and_then(|v| v.as_u64())
            .ok_or_else(|| format!("hunk at index {idx}.{h_idx} missing 'new_count' field"))?;
        let content = h
            .get("content")
            .and_then(|v| v.as_str())
            .ok_or_else(|| format!("hunk at index {idx}.{h_idx} missing 'content' field"))?;

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

pub fn apply_patch_hunks(content: &str, hunks: &[PatchHunk]) -> Result<String, (String, usize)> {
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_patch_parses_multiple_operations() {
        let parsed = parse_structured_patch("*** Begin Patch\nAdd File: a.txt\n>>>\na\n>>>\n*** End Patch\n*** Begin Patch\nDelete File: b.txt\n*** End Patch").unwrap();
        assert_eq!(parsed.operations.len(), 2);
    }

    #[test]
    fn traversal_is_rejected() {
        assert!(
            parse_structured_patch("*** Begin Patch\nDelete File: ../secret\n*** End Patch")
                .is_err()
        );
    }

    #[test]
    fn json_patch_round_trips() {
        let parsed = parse_structured_patch_json(
            r#"{"operations":[{"type":"add","path":"a.txt","content":"a"}]}"#,
        )
        .unwrap();
        assert!(matches!(
            parsed.operations[0],
            PatchOperation::AddFile { .. }
        ));
    }

    #[test]
    fn unified_diff_parses_create_modify_and_delete() {
        let parsed = parse_unified_diff(
            "--- /dev/null\n+++ b/new.txt\n@@ -0,0 +1 @@\n+new\n--- a/old.txt\n+++ b/old.txt\n@@ -1 +1 @@\n-old\n+updated\n--- a/gone.txt\n+++ /dev/null\n@@ -1 +0,0 @@\n-gone\n",
        )
        .unwrap();
        assert_eq!(parsed.operations.len(), 3);
        assert!(matches!(
            parsed.operations[0],
            PatchOperation::AddFile { .. }
        ));
        assert!(matches!(
            parsed.operations[1],
            PatchOperation::UpdateFile { .. }
        ));
        assert!(matches!(
            parsed.operations[2],
            PatchOperation::DeleteFile { .. }
        ));
    }
}
