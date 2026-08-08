//! Bounded breadth-scanning policy for repository exploration.

use serde_json::Value;
use std::path::{Component, Path};

const MIN_EXPLORATION_INPUT_TOKENS: u64 = 10_000;
const MAX_EXPLORATION_INPUT_TOKENS: u64 = 24_000;

pub(super) fn exploration_input_token_budget(context_window_tokens: usize) -> u64 {
    (context_window_tokens as u64 / 100)
        .clamp(MIN_EXPLORATION_INPUT_TOKENS, MAX_EXPLORATION_INPUT_TOKENS)
}

fn is_inspection_tool(name: &str) -> bool {
    matches!(name, "glob" | "grep" | "file_search")
}

/// Classify a single discovery call `(tool_name, JSON input)` as broad (repository-wide
/// scan) or scoped (exact/leaf-only). Only broad calls consume the guarded counter;
/// scoped discovery remains executable without contributing to the cut-off budget.
///
/// This classifier is intentionally aligned with the typed semantics already used by
/// `corpus::overview_guard::broad_pattern` and does not inspect natural-language
/// content or special-case fixture/repository/language names.
pub(super) fn is_broad_discovery(name: &str, input: &Value) -> bool {
    match name {
        "glob" => is_broad_glob(input),
        "grep" | "file_search" => is_broad_text_search(input),
        _ => false,
    }
}

fn is_broad_glob(input: &Value) -> bool {
    // Validate explicit root: unsafe roots fail closed as broad.
    if let Some(root) = input.get("root") {
        match root.as_str() {
            None => return true,          // non-string root → broad (malformed)
            Some(s) if s.is_empty() => {} // empty → default cwd, ok
            Some(s) => {
                let path = Path::new(s);
                if path.is_absolute() {
                    return true;
                }
                let mut has_normal = false;
                let mut has_curdir = false;
                for comp in path.components() {
                    match comp {
                        Component::RootDir | Component::Prefix(_) | Component::ParentDir => {
                            return true;
                        }
                        Component::CurDir => has_curdir = true,
                        Component::Normal(_) => has_normal = true,
                    }
                }
                // "." / "./" (curdir only) → acceptable, same as default cwd.
                // Any other path with no normal component → broad.
                if !has_normal && !has_curdir {
                    return true;
                }
            }
        }
    }
    let mut patterns: Vec<&str> = Vec::new();
    if let Some(p) = input.get("pattern").and_then(|v| v.as_str()) {
        patterns.push(p);
    }
    if let Some(arr) = input.get("patterns").and_then(|v| v.as_array()) {
        for v in arr {
            if let Some(s) = v.as_str() {
                patterns.push(s);
            }
        }
    }
    patterns.iter().any(|p| glob_pattern_is_broad(p))
}

/// Mirror of `corpus::overview_guard::broad_pattern` semantics:
/// recursive `**` and wildcard-directory segments are broad;
/// exact paths and leaf-only wildcards (e.g. `schema/*.json`) are scoped.
///
/// A pattern with an unsafe prefix (absolute path or `..` component)
/// is always classified broad before wildcard breadth is considered
/// (fail-closed). This checks the pattern string itself, not the
/// separate `root` tool argument.
fn glob_pattern_is_broad(pattern: &str) -> bool {
    let normalized = pattern.replace('\\', "/");
    // Fail-closed: absolute or parent-dir component in pattern → broad.
    if pattern_has_unsafe_components(&normalized) {
        return true;
    }
    if normalized.contains("**") {
        return true;
    }
    let segments: Vec<&str> = normalized.split('/').collect();
    if segments.len() <= 1 {
        return false;
    }
    // Wildcard in any non-leaf (directory) segment = broad
    segments
        .iter()
        .rev()
        .skip(1)
        .any(|seg| seg.contains(['*', '?', '[']))
}

/// A glob *pattern* is unsafe when it is absolute or contains `..`
/// as a path component, regardless of wildcard placement.
fn pattern_has_unsafe_components(pattern: &str) -> bool {
    if pattern.starts_with('/') {
        return true;
    }
    pattern.split('/').any(|seg| seg == "..")
}

fn is_broad_text_search(input: &Value) -> bool {
    let path = input.get("path").and_then(|v| v.as_str());
    match path {
        // Omitted or empty → repository-wide scan.
        None => true,
        Some(p) if p.is_empty() => true,
        Some(p) => {
            let path = Path::new(p);
            // Absolute paths are broad.
            if path.is_absolute() {
                return true;
            }
            let mut has_normal = false;
            for comp in path.components() {
                match comp {
                    Component::RootDir | Component::Prefix(_) => return true,
                    Component::ParentDir => return true,
                    Component::CurDir => {} // neutral alone, does not make it scoped
                    Component::Normal(_) => has_normal = true,
                }
            }
            // "." / "./" / root-relative with no normal component → broad.
            !has_normal
        }
    }
}

pub(super) fn should_close_exploration<'a>(
    iteration: usize,
    cumulative_input_tokens: u64,
    context_window_tokens: usize,
    repository_context_seen: bool,
    broad_discovery_batches: usize,
    tool_calls: impl IntoIterator<Item = (&'a str, &'a Value)>,
) -> bool {
    let calls: Vec<(&str, &Value)> = tool_calls.into_iter().collect();
    let budget = exploration_input_token_budget(context_window_tokens);
    if cumulative_input_tokens < budget {
        return false;
    }
    if calls.is_empty() {
        return false;
    }
    let all_inspection = calls.iter().all(|(name, _)| is_inspection_tool(name));
    if !all_inspection {
        return false;
    }
    let has_any_broad = calls
        .iter()
        .any(|(name, input)| is_broad_discovery(name, input));
    if !has_any_broad {
        return false;
    }

    if !repository_context_seen {
        // Before `repo_inspect`: any broad inspection batch over budget closes.
        iteration > 1
    } else {
        // After `repo_inspect`: broad-discovery batches are counted separately.
        // Scoped discovery and exact reads do not consume the allowance.
        // Only the counter matters here; iteration is not a secondary gate.
        broad_discovery_batches >= 2
    }
}
