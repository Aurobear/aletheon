# Structured Code Editing in Aletheon

**Date:** 2026-07-17

**Status:** Proposed

**Scope:** Production-grade structured patch format, robust hunk application, streaming progress, delta tracking, and model-awareness injection for the `apply_patch` tool. Inspired by Codex's `apply_patch` system.

**Baseline:** Aletheon current working tree (`auro/docs/executable-architecture-plans`).

## 1. Motivation and current-state gap

### 1.1 What exists today

Aletheon has a working `apply_patch` tool at `crates/corpus/src/tools/tools/apply_patch.rs` (771 lines). It supports:

| Feature | Implementation | Lines |
|---------|---------------|-------|
| System `patch` command (`patch -p1 --directory`) | Primary path via `apply_via_patch_command` | 142-215 |
| `--force` retry for new-file creation | Fallback inside system-patch path | 179-213 |
| Native unified-diff parser | `parse_unified_diff` — parses `---`/`+++` headers, `@@` hunk headers, create/delete/modify operations | 341-441 |
| Native hunk application | `apply_hunks` with `find_hunk_position` — exact match with +/-3 line fuzzy search | 511-617 |
| File creation with parent directories | `fs::create_dir_all` in native path only | 267-270 |
| Workspace-boundary validation | `validate_mutation_path` for base_dir and each target file | 74-87 |
| Tests | Create, modify, delete, empty input, hunk application, filename extraction | 619-771 |

The native path is a fallback; the system `patch` binary is always preferred (`apply_patch.rs:89-126`). Only when system `patch` fails does the native parser fire.

### 1.2 What is missing

| Gap | Detail |
|-----|--------|
| **No structured patch format** | Only unified diff is accepted. No multi-file Add/Delete/Update/Move operations in a single structured payload. |
| **No partial-success tracking** | If hunk 3 of 5 fails, the first two hunks are lost (the file was not written yet) or the entire file operation is aborted. No `AppliedPatchDelta` that reports which operations committed. |
| **No Unicode normalization** | `matches_hunk_at` (`:601-616`) uses exact `==` comparison. Unicode dashes, quotes, or whitespace variants break matching. |
| **No file move support** | A rename requires model to issue `bash_exec mv` + an `apply_patch` to update references. |
| **No streaming progress** | The entire patch is applied synchronously. No `PatchProgress` events for TUI feedback. |
| **No delta tracking** | The model has no awareness of which files changed across tool calls in a turn. |
| **No end-of-file append marker** | Appending to an existing file requires a full hunk that references a trailing context line. |
| **No model-awareness injection** | No structured "Files changed this turn" section injected into the model context. |
| **System `patch` dependency** | Relies on the `patch` binary being available inside the sandbox (bubblewrap or process). Native path is a fallback, not the primary. |

### 1.3 Tool synergy gap

The agent currently has 20 tools but only 3 are activated for `code-agent` (`agents/code-agent.toml:6`):

```toml
tools = ["file_read", "file_write", "bash_exec"]
```

None of the code-analysis tools (`code_graph`, `grep`, `glob`, `file_search`) are available. The model cannot discover symbols before patching, search for callers, or find files by glob — it must use `bash_exec` for everything other than reading and writing whole files.

## 2. Design

### 2.1 Structured Patch Format

A single, unambiguous format that supports all filesystem edits a model needs. Two equivalent input formats are accepted:

#### 2.1.1 Text format (model-generated, Codex-compatible)

```
*** Begin Patch
Add File: src/new_module.rs
Content:
>>> 
use std::collections::HashMap;

pub struct NewModule {
    data: HashMap<String, String>,
}
>>> 
*** End Patch

*** Begin Patch
Update File: src/existing.rs
Move to: src/renamed.rs
Hunk:
<<<
@@ -10,7 +10,8 @@
 fn main() {
-    let x = old_function();
+    let x = new_function();
+    log::info!("starting");
     process(x);
 }
>>>
*** End Patch

*** Begin Patch
Delete File: src/obsolete.rs
*** End Patch
```

BNF grammar:

```
patch          ::= { patch_block }

patch_block    ::= begin_marker newline operation_section end_marker

begin_marker   ::= "*** Begin Patch"
end_marker     ::= "*** End Patch"

operation_section ::= add_file | delete_file | update_file

add_file       ::= "Add File:" ws file_path newline
                   "Content:" newline
                   fence newline
                   { any_line } newline
                   fence newline

delete_file    ::= "Delete File:" ws file_path newline

update_file    ::= "Update File:" ws file_path newline
                   [ "Move to:" ws file_path newline ]
                   "Hunk:" newline
                   fence newline
                   hunk_lines
                   fence newline

hunk_lines     ::= { hunk_line }
hunk_line      ::= "@@" hunk_header "@@" newline { diff_line }
                 | "@@" hunk_header "@@" newline
diff_line      ::= " " context_line | "-" removed_line | "+" added_line

file_path      ::= relative_unix_path   (no "../" traversal)
fence          ::= ">>>"
ws             ::= " "
```

Path safety: All paths are relative to the workspace root. Traversal (`../`) is rejected during validation. Absolute paths are rejected.

#### 2.1.2 JSON format (alternative, programmatic)

```json
{
  "operations": [
    {
      "type": "add",
      "path": "src/new_module.rs",
      "content": "use std::collections::HashMap;\n..."
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
}
```

JSON operations: `add`, `update`, `delete`, `append`.

### 2.2 Rust type definitions

New types to add in `crates/corpus/src/tools/tools/apply_patch.rs` (or a sibling `structured_patch.rs`):

```rust
/// A single operation within a structured patch.
#[derive(Debug, Clone, Serialize, Deserialize)]
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
        move_to: Option<String>,
        hunks: Vec<PatchHunk>,
    },
    AppendFile {
        path: String,
        content: String,
    },
}

/// A single hunk within an UpdateFile operation.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PatchHunk {
    pub old_start: usize,
    pub old_count: usize,
    pub new_start: usize,
    pub new_count: usize,
    /// The combined hunk text: lines prefixed with ' ', '-', '+'.
    pub content: String,
}

/// Result of applying a structured patch, tracking partial progress.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructuredPatchResult {
    /// Operations that fully succeeded.
    pub applied: Vec<AppliedOperation>,
    /// Operations that failed.
    pub failed: Vec<FailedOperation>,
    /// Complete list of files that were changed (for model awareness).
    pub files_changed: Vec<FileChangeSummary>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppliedOperation {
    pub op_type: String,        // "add", "update", "delete", "append"
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
```

### 2.3 Robust Patch Application

#### 2.3.1 Core function

```rust
/// Apply a structured patch with partial-success tracking.
/// Returns `StructuredPatchResult` regardless of partial failure.
pub async fn apply_structured_patch(
    operations: &[PatchOperation],
    workspace_root: &Path,
) -> StructuredPatchResult
```

#### 2.3.2 Validation

Per `PatchOperation::validate(&self, workspace_root)`:
- No path may contain `..` segments.
- No path may be absolute.
- For `AddFile`: target must not exist (return error with existing-content hint).
- For `DeleteFile`: target must exist and must be a regular file (not directory).
- For `UpdateFile`: target must exist and be a regular file.
- For `AppendFile`: target must exist and be a regular file.

#### 2.3.3 Unicode normalization for fuzzy matching

The current `matches_hunk_at` (`apply_patch.rs:601-616`) uses exact string equality. Replace with:

```rust
/// Normalize a line for fuzzy comparison.
/// Handles Unicode dashes (U+2013, U+2014, U+2015 → '-'),
/// Unicode quotes (U+2018..U+201D → ASCII quotes),
/// trailing whitespace collapse.
fn normalize_line(line: &str) -> String {
    use unicode_normalization::UnicodeNormalization;
    let mut s = line.nfkc().collect::<String>();
    // Replace Unicode dashes with ASCII dash
    s = s.replace('\u{2013}', "-").replace('\u{2014}', "-").replace('\u{2015}', "-");
    // Trim trailing whitespace (models often add/remove trailing spaces)
    s.trim_end().to_string()
}

fn lines_match(a: &str, b: &str) -> bool {
    normalize_line(a) == normalize_line(b)
}
```

Add the `unicode-normalization` crate (already in Aletheon's dependency tree via tree-sitter or other crates; verify and declare if needed).

#### 2.3.4 `seek_sequence` algorithm

When exact position match fails and the +/-3 line window fails, fall back to a broader `seek_sequence`:

```rust
/// Find the best match position for a hunk by searching the full file.
/// Uses the context lines as a "fingerprint" and finds the offset
/// that maximizes the number of matching context+remove lines.
fn seek_sequence(
    file_lines: &[String],
    hunk: &PatchHunk,
    max_search_window: usize,
) -> Option<usize> {
    let context_lines: Vec<&str> = extract_context_lines(hunk);
    if context_lines.is_empty() {
        return None;
    }
    // Sliding window over file, count matches
    let best = (0..file_lines.len().saturating_sub(context_lines.len()))
        .max_by_key(|&start| {
            context_lines.iter().enumerate()
                .filter(|(i, ctx_line)| {
                    file_lines.get(start + i)
                        .map(|fl| lines_match(fl, ctx_line))
                        .unwrap_or(false)
                })
                .count()
        });
    best.filter(|&start| {
        let matches = context_lines.iter().enumerate()
            .filter(|(i, ctx_line)| {
                file_lines.get(start + i)
                    .map(|fl| lines_match(fl, ctx_line))
                    .unwrap_or(false)
            })
            .count();
        matches as f64 / context_lines.len() as f64 >= 0.5
    })
}
```

#### 2.3.5 Partial-commit tracking

Each operation is applied in sequence. If an `UpdateFile` with 5 hunks fails on hunk 3:

1. Hunks 1 and 2 are **not** written to disk (the file is saved only after all hunks succeed).
2. The failure is recorded: `FailedOperation { hunks_applied_before_failure: Some(2), error: "..." }`.
3. Previously completed operations (e.g., an `AddFile` that ran before this `UpdateFile`) remain on disk and are reported in `applied`.

If an operation is a `Move` (UpdateFile with `move_to`), the rename is done atomically: write the modified content to the new path first, then delete the old path. If the write succeeds but the delete fails, the operation is reported as partially failed with the old file still present.

#### 2.3.6 Auto-creation of parent directories

Every `AddFile` and `UpdateFile` (with `move_to`) calls `write_file_with_missing_parent_retry`:

```rust
async fn write_file_with_parent_dirs(
    path: &Path,
    content: &str,
) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .await
            .map_err(|e| format!("create_dir_all({}): {e}", parent.display()))?;
    }
    fs::write(path, content)
        .await
        .map_err(|e| format!("write({}): {e}", path.display()))
}
```

The `file_write` tool already does this (`file_write.rs:65-75`), but `apply_patch`'s native path only does it for `PatchOp::Create` (`apply_patch.rs:267-270`), not for modified files in new directories. This must be applied consistently.

### 2.4 Streaming Progress (Phase 2)

#### 2.4.1 Patch progress events

New `TurnEventV1` variant in `crates/fabric/src/ipc/stream.rs`:

```rust
TurnEventV1::PatchProgress {
    /// "started", "file_changed", "file_failed", "completed"
    status: String,
    /// Affected file path, if applicable
    path: Option<String>,
    /// Operation type: "add", "update", "delete", "append", "move"
    operation: Option<String>,
    /// Error message, if status is "file_failed"
    error: Option<String>,
    /// Summary counts when status is "completed"
    applied_count: Option<usize>,
    failed_count: Option<usize>,
}
```

#### 2.4.2 StreamingPatchApplier

```rust
/// Applies structured patch operations sequentially, emitting progress
/// events through a `TurnEventSender`.
pub struct StreamingPatchApplier {
    sender: TurnEventSender,
}

impl StreamingPatchApplier {
    pub fn new(sender: TurnEventSender) -> Self { ... }

    /// Apply operations one at a time, sending a `PatchProgress` event
    /// per operation.
    pub async fn apply(
        &self,
        operations: &[PatchOperation],
        workspace_root: &Path,
    ) -> StructuredPatchResult { ... }
}
```

#### 2.4.3 Argument diff consumption

For models that stream arguments (Anthropic's streaming tool use), implement an `ArgumentDiffConsumer` that parses partial argument JSON or partial `*** Begin Patch ... *** End Patch` blocks and emits `PatchProgress` events with `status: "receiving"` while the argument is still arriving. This is a Phase 2-b item.

### 2.5 Tool Integration (Phase 2)

#### 2.5.1 Upgraded `apply_patch` tool

Extend the existing `ApplyPatchTool::execute` (`apply_patch.rs:46-127`) to accept both formats:

```json
{
  "type": "object",
  "properties": {
    "patch": {
      "type": "string",
      "description": "Unified diff patch content (standard diff format), OR structured patch text (*** Begin Patch format)"
    },
    "patch_json": {
      "type": "object",
      "description": "Structured patch in JSON format. Mutually exclusive with 'patch'."
    },
    "base_dir": {
      "type": "string",
      "description": "Base directory for applying the patch (default: current dir)"
    }
  },
  "oneOf": [
    { "required": ["patch"] },
    { "required": ["patch_json"] }
  ]
}
```

Parsing logic:
1. If `patch_json` is provided → parse as JSON `Vec<PatchOperation>`.
2. If `patch` starts with `*** Begin Patch` → parse as structured text format.
3. Otherwise → fall through to existing unified-diff parsing (current behavior).

#### 2.5.2 Enhanced tool result

Extend `ToolResult` to carry structured delta metadata. Add an optional field to `ToolResultMeta`:

```rust
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ToolResultMeta {
    pub execution_time_ms: u64,
    pub truncated: bool,
    /// Structured patch delta, if this was an apply_patch execution.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub patch_delta: Option<StructuredPatchResult>,
}
```

This allows the turn pipeline to extract `patch_delta` and feed it to the `TurnDiffTracker` without parsing the tool result content string.

#### 2.5.3 Sandbox wrapping

The `apply_patch` tool currently runs in-process. The daemon path (`crates/executive/src/service/turn_pipeline.rs:332-379`) dispatches tool execution through a closure. Application of the structured patch must respect the workspace boundary enforced by `WorkspacePolicy` (in `ToolContext` at `fabric/src/types/tool.rs:43`). For Phase 1, in-process application with `validate_mutation_path` is acceptable. Phase 2 aligns with the Tool Execution Hardening plan (separate plan: `docs/plans/2026-07-17-tool-execution-hardening-plan.md`).

### 2.6 Delta Tracking and Model Awareness (Phase 3)

#### 2.6.1 TurnDiffTracker

```rust
/// Accumulates file changes across all tool calls in a single turn.
/// Injected into model context after each batch of tool results.
#[derive(Debug, Clone, Default)]
pub struct TurnDiffTracker {
    /// Map from file path to cumulative change information.
    files: HashMap<String, TurnFileDelta>,
}

#[derive(Debug, Clone)]
pub struct TurnFileDelta {
    /// How many times this file was touched this turn.
    pub edits: usize,
    /// Total operations applied across all edits.
    pub hunks_applied: usize,
    /// File size at the start of the turn (snapshot).
    pub bytes_before: u64,
    /// File size after the most recent edit.
    pub bytes_after: u64,
}

impl TurnDiffTracker {
    /// Record patch application results into the tracker.
    pub fn record_patch(&mut self, delta: &StructuredPatchResult) { ... }

    /// Record a file_write tool call.
    pub fn record_file_write(&mut self, path: &str, bytes_written: u64) { ... }

    /// Generate a context injection string for the model.
    /// Produces a Markdown-formatted summary.
    pub fn to_context_injection(&self) -> String { ... }

    /// Reset for a new turn.
    pub fn reset(&mut self) { ... }
}
```

#### 2.6.2 Context injection format

The `to_context_injection()` produces a section like:

```
## Files changed this turn

| File | Edits | Hunks | Size |
|------|-------|-------|------|
| `src/main.rs` | 1 | 3 | 1,420B → 1,540B |
| `Cargo.toml` | 1 | 1 | 220B → 240B |

2 files changed, 4 hunks applied.
```

This is appended to the model's system message or injected as a tool result before the next model call.

#### 2.6.3 Integration point

The `TurnDiffTracker` lives on the turn-state struct in `crates/executive/src/service/turn_pipeline.rs`. After every tool execution batch (currently at line ~379: `execute_tool`), the pipeline:
1. Checks if the tool result carries a `patch_delta`.
2. If so, calls `turn_diff_tracker.record_patch(&delta)`.
3. Before the next model call, injects `turn_diff_tracker.to_context_injection()` into the conversation.

### 2.7 Tool Synergy Patterns

With `code_graph`, `grep`, `glob`, and `file_search` activated alongside `apply_patch`, the recommended agent workflow becomes:

```
1. grep "old_symbol" --path src/          → find all occurrences
2. code_graph { operation: "callers", symbol: "old_function", file_path: "src/" }
                                          → find all callers (AST-aware)
3. file_read src/target.rs                 → read the full context
4. apply_patch (structured UpdateFile)     → apply the change with proper hunks
```

This replaces the current pattern of:

```
1. bash_exec "grep -rn old_symbol src/"    → find occurrences (sandbox overhead)
2. bash_exec "cat src/target.rs"           → read file (sandbox overhead)
3. apply_patch (unified diff)              → apply basic patch
```

The structured patch format combined with AST-aware discovery (`code_graph`) enables multi-symbol refactors: the model can find all callers of `old_function`, generate an `UpdateFile` hunk for each call site, and submit them as a single atomic `apply_patch` call.

## 3. Implementation phases

### Phase 1: Structured patch format + robust application + error recovery

**Estimated effort:** 1 week

**Files:**
- `crates/corpus/src/tools/tools/apply_patch.rs` — primary changes
- New: `crates/corpus/src/tools/tools/structured_patch.rs` — format types and parser

**Commit 1: Define structured patch types**
```
feat(corpus): add structured patch types for apply_patch

Define PatchOperation, PatchHunk, StructuredPatchResult, and
related types supporting AddFile, DeleteFile, UpdateFile (with
Move), and AppendFile operations. Include serialization.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Commit 2: Implement structured patch parser**
```
feat(corpus): add structured patch text and JSON parsers

Parse *** Begin Patch / *** End Patch format with BNF-compliant
grammar. Also accept JSON format for programmatic use. Both
produce Vec<PatchOperation>.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Commit 3: Implement robust patch application**
```
feat(corpus): add apply_structured_patch with partial-commit tracking

Implement Unicode-normalized fuzzy matching (seek_sequence fallback),
parent-directory auto-creation for all mutation operations, and
StructuredPatchResult that tracks applied vs failed operations
independently. Replace exact string equality with normalize_line
for Unicode dash/quote resilience.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Commit 4: Integrate structured format into apply_patch tool**
```
feat(corpus): integrate structured patch into apply_patch tool

Accept both patch_json and *** Begin Patch text inputs.
Auto-detect: if patch string starts with "*** Begin Patch",
parse as structured; otherwise fall through to existing
unified-diff path. Add patch_delta to ToolResultMeta.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Commit 5: Tests**
```
test(corpus): add structured patch unit and integration tests

- Parse text format: AddFile, DeleteFile, UpdateFile, Move
- Parse JSON format: all operation types
- Round-trip: parse inputs, apply to temp dir, verify filesystem
- Error recovery: hunk failure preserves prior operations
- Unicode: dashes, quotes, trailing whitespace
- seek_sequence: shifted context, large files
- Workspace boundary: traversal rejection

Co-Authored-By: Claude <noreply@anthropic.com>
```

### Phase 2: Streaming progress + tool integration

**Estimated effort:** 1 week

**Files:**
- `crates/corpus/src/tools/tools/apply_patch.rs` — StreamingPatchApplier
- `crates/fabric/src/ipc/stream.rs` — PatchProgress event variant
- `crates/executive/src/service/turn_pipeline.rs` — wire events

**Commit 6: Add PatchProgress event to TurnEventV1**
```
feat(fabric): add PatchProgress event to TurnEventV1

Define PatchProgress variant with status, path, operation,
error, and count fields. Enable TUI to show live patch progress.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Commit 7: Implement StreamingPatchApplier**
```
feat(corpus): add StreamingPatchApplier with per-operation events

StreamingPatchApplier processes operations sequentially and
emits a PatchProgress event for each file. The apply_patch
tool creates a StreamingPatchApplier when a TurnEventSender
is available in the tool context.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Commit 8: Wire streaming events into turn pipeline**
```
feat(executive): wire PatchProgress events into turn pipeline

In turn_pipeline.rs, pass the TurnEventSender through to tool
execution context. apply_patch tool emits events during
execution. TUI displays progress in real time.

Co-Authored-By: Claude <noreply@anthropic.com>
```

### Phase 3: Model awareness injections + tool synergy

**Estimated effort:** 0.5-1 week

**Files:**
- New: `crates/executive/src/service/turn_diff_tracker.rs` — TurnDiffTracker
- `crates/executive/src/service/turn_pipeline.rs` — integration
- `agents/code-agent.toml` — tool activation

**Commit 9: Implement TurnDiffTracker**
```
feat(executive): add TurnDiffTracker for turn-wide file change awareness

TurnDiffTracker accumulates FileChangeSummary from
apply_patch and file_write tool calls. Generates a Markdown
"Files changed this turn" injection for the model context.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Commit 10: Inject delta summary before model calls**
```
feat(executive): inject file change summary into model context

After each tool execution batch, record deltas via TurnDiffTracker.
Before the next model call, prepend the context injection to the
system message. Model maintains awareness of workspace state
without re-reading files.

Co-Authored-By: Claude <noreply@anthropic.com>
```

**Commit 11: Activate code-analysis tools for code-agent**
```
feat(agents): activate code_graph, grep, glob, apply_patch for code-agent

Enable structured code editing workflow: grep → code_graph →
apply_patch. Replace bash_exec grep/cat patterns with native
tools that have lower sandbox overhead and structured output.

Co-Authored-By: Claude <noreply@anthropic.com>
```

## 4. Acceptance criteria

### Phase 1

| # | Criterion | Verification |
|---|-----------|-------------|
| P1.1 | `StructuredPatch` parser accepts Codex-compatible `*** Begin Patch / *** End Patch` format | Unit test: parse multi-operation text input |
| P1.2 | JSON format parser produces equivalent operations | Unit test: same operations in JSON produce identical `Vec<PatchOperation>` |
| P1.3 | `AddFile` creates file with parent directories | Integration test: `AddFile { path: "a/b/c.rs", content: "..." }` creates nested dirs |
| P1.4 | `DeleteFile` removes existing file | Integration test: pre-create file, apply delete, assert absence |
| P1.5 | `UpdateFile` applies hunks with exact matching | Integration test: modify 3-line file with 1-hunk patch |
| P1.6 | `UpdateFile` applies hunks with fuzzy matching (+/-3 window) | Unit test: hunk at line 10, file shifted by 2 lines, still matches |
| P1.7 | `UpdateFile` with `move_to` renames file after modification | Integration test: modify content, write to new path, old path removed |
| P1.8 | Unicode dashes and quotes normalized during matching | Unit test: file has Unicode em-dash, hunk has ASCII dash, matches |
| P1.9 | `seek_sequence` finds match when hunk is shifted >3 lines | Unit test: file with 100 lines, hunk at offset +15 from header |
| P1.10 | Partial success: failed hunk reports prior operations as applied | Integration test: 3 operations, second fails, first is on disk and reported |
| P1.11 | Path traversal (`../`) is rejected | Unit test: `AddFile { path: "../escape.rs" }` returns error |
| P1.12 | Existing unified-diff format still works (backward compatibility) | Existing tests in `apply_patch.rs` continue to pass |
| P1.13 | `AppendFile` appends to end of existing file | Integration test: pre-create file, append content, verify concatenation |

### Phase 2

| # | Criterion | Verification |
|---|-----------|-------------|
| P2.1 | `PatchProgress` events emitted during patch application | Integration test: capture events, verify sequence: started→file_changed→completed |
| P2.2 | Failed operations emit `file_failed` events | Integration test: force hunk mismatch, capture failed event |
| P2.3 | TUI displays patch progress in real time | Manual verification: apply large patch, observe TUI updates |
| P2.4 | `patch_delta` present in `ToolResultMeta` after structured apply | Unit test: check metadata after apply_patch execution |

### Phase 3

| # | Criterion | Verification |
|---|-----------|-------------|
| P3.1 | `TurnDiffTracker` accumulates changes from multiple tool calls | Unit test: record two patches, verify merged delta |
| P3.2 | Context injection formatted as Markdown table | Unit test: verify output format |
| P3.3 | Context injection injected before subsequent model calls | Integration test: verify system message contains file change section |
| P3.4 | `code-agent.toml` grants code_graph, grep, glob, apply_patch | Config inspection |
| P3.5 | End-to-end workflow: grep → code_graph → apply_patch succeeds | Manual: agent refactors a Rust function across 3 files |

## 5. Risk assessment

| Risk | Likelihood | Impact | Mitigation |
|------|-----------|--------|------------|
| System `patch` removed from sandbox | Medium | High — primary path breaks | Phase 1 makes native path the primary, drops system-patch dependency |
| Unicode normalization changes line semantics | Low | Medium — silent corruption | `seek_sequence` is fallback only; exact match always tried first |
| Partial-commit leaves workspace in inconsistent state | Medium | Medium | Operations are file-granular; files are written atomically; failed hunks don't write |
| Structured format confuses model (format errors) | Medium | Low — retryable | Parser returns clear error messages; model can retry with corrected format |
| `TurnDiffTracker` grows unboundedly | Low | Low — per-turn | Reset after each turn; max tracked files capped at 200 |
| New `unicode-normalization` crate conflicts | Low | Low | Already in dependency tree via other crates; cargo tree to verify |

## 6. Dependencies

- **Phase 1:** No external dependencies beyond existing crates. Self-contained in `crates/corpus`.
- **Phase 2:** Depends on `TurnEventV1` in `crates/fabric`. Requires `TurnEventSender` propagated through tool context — minor plumbing in `turn_pipeline.rs`.
- **Phase 3:** Depends on Phase 2 delta tracking. Depends on Capability Activation Plan (`docs/plans/2026-07-17-capability-activation-and-agent-profiles-plan.md`) for tool activation in agent profiles.

## 7. Rollback

The structured patch format is a superset of the existing unified-diff input. Removing the feature requires:
1. Removing the `patch_json` and structured-format parsing paths from `apply_patch.rs`.
2. Reverting `ToolResultMeta.patch_delta` to the prior struct shape.
3. Reverting agent TOML changes.

No database migrations, no wire protocol changes (TurnEventV1 is additive), no session format changes.
