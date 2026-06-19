# P1 Implementation Plan: Checkpoint + Tool Parallelism + Storm Breaker

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Implement three P1 improvements — checkpoint/rewind, tool parallelism, and storm breaker — for agent safety and performance.

**Architecture:** Checkpoint provides snapshot-based rewind for file edits. Tool parallelism partitions read-only tools into concurrent batches. Storm breaker detects and breaks model loops.

**Tech Stack:** Rust, tokio (JoinSet for parallel), serde

---

## File Map

### New Files
| File | Purpose |
|------|---------|
| `crates/aletheon-runtime/src/core/checkpoint.rs` | CheckpointStore, FileSnap, Checkpoint, RewindScope |
| `crates/aletheon-runtime/src/core/storm_breaker.rs` | StormBreaker loop detection |

### Modified Files
| File | Change |
|------|--------|
| `crates/aletheon-abi/src/tool.rs` | Add Previewer trait |
| `crates/aletheon-runtime/src/core/mod.rs` | Export checkpoint, storm_breaker |
| `crates/aletheon-runtime/src/core/react_loop.rs` | Add partition_tool_calls(), MAX_PARALLEL_TOOLS |
| `crates/aletheon-runtime/src/core/controller.rs` | Add rewind(), checkpoints() |

---

## Task 1: Storm Breaker

**Files:**
- Create: `crates/aletheon-runtime/src/core/storm_breaker.rs`
- Modify: `crates/aletheon-runtime/src/core/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/aletheon-runtime/src/core/storm_breaker.rs

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_trigger_below_threshold() {
        let mut sb = StormBreaker::new(3);
        assert!(sb.record("bash", true, "error: file not found").is_none());
        assert!(sb.record("bash", true, "error: file not found").is_none());
    }

    #[test]
    fn trigger_on_consecutive_failures() {
        let mut sb = StormBreaker::new(3);
        sb.record("bash", true, "error: file not found");
        sb.record("bash", true, "error: file not found");
        let directive = sb.record("bash", true, "error: file not found");
        assert!(directive.is_some());
        assert!(directive.unwrap().contains("Storm breaker"));
    }

    #[test]
    fn reset_on_success() {
        let mut sb = StormBreaker::new(3);
        sb.record("bash", true, "error: file not found");
        sb.record("bash", false, "ok"); // success resets
        sb.record("bash", true, "error: file not found");
        sb.record("bash", true, "error: file not found");
        // Only 2 consecutive failures after reset, not 3
        assert!(sb.record("bash", true, "error: file not found").is_some());
    }

    #[test]
    fn different_errors_dont_trigger() {
        let mut sb = StormBreaker::new(3);
        sb.record("bash", true, "error: file not found");
        sb.record("bash", true, "error: permission denied");
        assert!(sb.record("bash", true, "error: timeout").is_none());
    }

    #[test]
    fn trigger_on_consecutive_successes() {
        let mut sb = StormBreaker::new(3);
        sb.record("write_file", false, "ok");
        sb.record("write_file", false, "ok");
        let warning = sb.record("write_file", false, "ok");
        assert!(warning.is_some());
        assert!(warning.unwrap().contains("succeeded"));
    }

    #[test]
    fn reset_clears_all() {
        let mut sb = StormBreaker::new(2);
        sb.record("bash", true, "error");
        sb.record("write_file", false, "ok");
        sb.reset();
        assert!(sb.record("bash", true, "error").is_none());
        assert!(sb.record("write_file", false, "ok").is_none());
    }

    #[test]
    fn different_tools_independent() {
        let mut sb = StormBreaker::new(3);
        sb.record("bash", true, "error");
        sb.record("grep", true, "error");
        sb.record("bash", true, "error");
        // bash has 2, grep has 1 — neither triggers
        assert!(sb.record("grep", true, "error").is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --lib core::storm_breaker::tests 2>&1 | tail -5`
Expected: error: module `storm_breaker` not found

- [ ] **Step 3: Write implementation**

```rust
// crates/aletheon-runtime/src/core/storm_breaker.rs

//! Storm breaker — detects and breaks model loops.
//!
//! Tracks consecutive identical tool failures and successes.
//! When a threshold is reached, injects a directive to change approach.

use std::collections::HashMap;

const DEFAULT_THRESHOLD: usize = 3;

/// Tracks tool call patterns to detect loops.
pub struct StormBreaker {
    /// Key: (tool_name, error_signature), Value: consecutive count
    failure_counts: HashMap<(String, String), usize>,
    /// Key: tool_name, Value: consecutive success count
    success_counts: HashMap<String, usize>,
    /// Threshold to trigger
    threshold: usize,
}

impl StormBreaker {
    pub fn new(threshold: usize) -> Self {
        Self {
            failure_counts: HashMap::new(),
            success_counts: HashMap::new(),
            threshold,
        }
    }

    pub fn with_defaults() -> Self {
        Self::new(DEFAULT_THRESHOLD)
    }

    /// Record a tool call result. Returns a directive if loop detected.
    pub fn record(&mut self, tool_name: &str, is_error: bool, content: &str) -> Option<String> {
        if is_error {
            let error_sig = Self::extract_error_signature(content);
            let key = (tool_name.to_string(), error_sig);
            let count = self.failure_counts.entry(key).or_insert(0);
            *count += 1;

            // Reset success counter for this tool
            self.success_counts.remove(tool_name);

            if *count >= self.threshold {
                return Some(format!(
                    "⚠️ Storm breaker: {} has failed {} times with the same error. \
                     Previous attempts did not work. Try a completely different approach.",
                    tool_name, count
                ));
            }
        } else {
            let count = self.success_counts.entry(tool_name.to_string()).or_insert(0);
            *count += 1;

            // Reset failure counters for this tool
            self.failure_counts.retain(|k, _| k.0 != tool_name);

            if *count >= self.threshold {
                return Some(format!(
                    "⚠️ Storm breaker: {} has succeeded {} times in a row. \
                     Verify the result is correct before continuing.",
                    tool_name, count
                ));
            }
        }
        None
    }

    /// Reset all counters (called at turn start).
    pub fn reset(&mut self) {
        self.failure_counts.clear();
        self.success_counts.clear();
    }

    /// Extract a normalized error signature for comparison.
    fn extract_error_signature(content: &str) -> String {
        // Take first 100 chars, lowercase, collapse whitespace
        let s: String = content
            .chars()
            .take(100)
            .flat_map(|c| {
                if c.is_whitespace() {
                    vec![' ']
                } else {
                    vec![c.to_ascii_lowercase()]
                }
            })
            .collect();
        s.trim().to_string()
    }
}
```

- [ ] **Step 4: Add to mod.rs**

In `crates/aletheon-runtime/src/core/mod.rs`, add:
```rust
pub mod storm_breaker;
```

- [ ] **Step 5: Run tests**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --lib core::storm_breaker::tests 2>&1 | tail -15`
Expected: all tests pass

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-runtime/src/core/storm_breaker.rs crates/aletheon-runtime/src/core/mod.rs
git commit -m "feat(runtime): add StormBreaker for loop detection

Detects consecutive identical failures (same tool + same error) and
consecutive identical successes. Injects a directive to change approach
when threshold is reached. Prevents model from getting stuck in loops."
```

---

## Task 2: Previewer Trait + Checkpoint System

**Files:**
- Modify: `crates/aletheon-abi/src/tool.rs`
- Create: `crates/aletheon-runtime/src/core/checkpoint.rs`
- Modify: `crates/aletheon-runtime/src/core/mod.rs`

- [ ] **Step 1: Write the failing test**

```rust
// crates/aletheon-runtime/src/core/checkpoint.rs

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;
    use tempfile::TempDir;

    #[test]
    fn checkpoint_captures_file_snap() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("test.txt");
        fs::write(&file, "original").unwrap();

        let snap = FileSnap::capture(&file).unwrap();
        assert_eq!(snap.path, file);
        assert_eq!(snap.content.as_deref(), Some("original"));
    }

    #[test]
    fn checkpoint_captures_nonexistent_file() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("missing.txt");

        let snap = FileSnap::capture(&file).unwrap();
        assert!(snap.content.is_none());
    }

    #[test]
    fn checkpoint_store_and_list() {
        let tmp = TempDir::new().unwrap();
        let mut store = CheckpointStore::new(tmp.path());

        store.open_checkpoint(1, "first turn", 0).unwrap();
        store.add_snap(FileSnap {
            path: PathBuf::from("/tmp/test.txt"),
            content: Some("content".into()),
        });
        store.seal_checkpoint();

        store.open_checkpoint(2, "second turn", 5).unwrap();
        store.seal_checkpoint();

        assert_eq!(store.checkpoints().len(), 2);
        assert_eq!(store.checkpoints()[0].turn, 1);
        assert_eq!(store.checkpoints()[1].turn, 2);
    }

    #[test]
    fn rewind_code_restores_files() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("test.txt");
        fs::write(&file, "original").unwrap();

        let mut store = CheckpointStore::new(tmp.path());
        store.open_checkpoint(1, "turn 1", 0).unwrap();
        store.add_snap(FileSnap::capture(&file).unwrap());
        store.seal_checkpoint();

        // Modify file
        fs::write(&file, "modified").unwrap();

        // Rewind
        store.rewind_code(1).unwrap();
        assert_eq!(fs::read_to_string(&file).unwrap(), "original");
    }

    #[test]
    fn rewind_code_deletes_created_file() {
        let tmp = TempDir::new().unwrap();
        let file = tmp.path().join("new.txt");

        let mut store = CheckpointStore::new(tmp.path());
        store.open_checkpoint(1, "turn 1", 0).unwrap();
        store.add_snap(FileSnap::capture(&file).unwrap()); // None content
        store.seal_checkpoint();

        // Create file
        fs::write(&file, "created").unwrap();

        // Rewind should delete it
        store.rewind_code(1).unwrap();
        assert!(!file.exists());
    }

    #[test]
    fn get_checkpoint_by_turn() {
        let tmp = TempDir::new().unwrap();
        let mut store = CheckpointStore::new(tmp.path());

        store.open_checkpoint(5, "turn 5", 10).unwrap();
        store.seal_checkpoint();

        assert!(store.get_checkpoint(5).is_some());
        assert!(store.get_checkpoint(3).is_none());
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --lib core::checkpoint::tests 2>&1 | tail -5`
Expected: error: module `checkpoint` not found

- [ ] **Step 3: Add Previewer trait to tool.rs**

In `crates/aletheon-abi/src/tool.rs`, add after the `Tool` trait:

```rust
/// Tools that can preview their change without touching disk.
///
/// Used by the checkpoint system to capture file state before edits.
/// Only edit/write tools implement this. Bash does not.
pub trait Previewer: Tool {
    /// Preview the file change this tool would make.
    /// Returns None if the tool can't preview (e.g., bash).
    fn preview(&self, args: &serde_json::Value) -> Option<FileSnap>;
}
```

- [ ] **Step 4: Write checkpoint.rs**

```rust
// crates/aletheon-runtime/src/core/checkpoint.rs

//! Checkpoint and rewind system.
//!
//! Provides snapshot-based rewind for file edits.
//! One checkpoint per user turn. Uses Previewer trait to capture state.

use std::path::{Path, PathBuf};
use std::time::SystemTime;
use tracing::{info, warn};

/// A snapshot of a file before it was modified.
#[derive(Debug, Clone)]
pub struct FileSnap {
    pub path: PathBuf,
    pub content: Option<String>, // None = file didn't exist
}

impl FileSnap {
    /// Capture the current state of a file.
    pub fn capture(path: &Path) -> std::io::Result<Self> {
        let content = if path.exists() {
            Some(std::fs::read_to_string(path)?)
        } else {
            None
        };
        Ok(Self {
            path: path.to_path_buf(),
            content,
        })
    }

    /// Restore this snapshot to disk.
    pub fn restore(&self) -> std::io::Result<()> {
        match &self.content {
            Some(content) => {
                if let Some(parent) = self.path.parent() {
                    std::fs::create_dir_all(parent)?;
                }
                std::fs::write(&self.path, content)?;
            }
            None => {
                if self.path.exists() {
                    std::fs::remove_file(&self.path)?;
                }
            }
        }
        Ok(())
    }
}

/// Scope of rewind operation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RewindScope {
    Code,
    Conversation,
    Both,
}

/// One checkpoint per user turn.
#[derive(Debug, Clone)]
pub struct Checkpoint {
    pub turn: usize,
    pub time: SystemTime,
    pub prompt: String,
    pub msg_index: usize,
    pub files: Vec<FileSnap>,
}

/// Checkpoint store for a session.
pub struct CheckpointStore {
    #[allow(dead_code)]
    session_dir: PathBuf,
    checkpoints: Vec<Checkpoint>,
    /// Currently open checkpoint (not yet sealed).
    current: Option<Checkpoint>,
}

impl CheckpointStore {
    pub fn new(session_dir: &Path) -> Self {
        Self {
            session_dir: session_dir.to_path_buf(),
            checkpoints: Vec::new(),
            current: None,
        }
    }

    /// Open a new checkpoint for the current turn.
    pub fn open_checkpoint(
        &mut self,
        turn: usize,
        prompt: &str,
        msg_index: usize,
    ) -> std::io::Result<()> {
        self.current = Some(Checkpoint {
            turn,
            time: SystemTime::now(),
            prompt: prompt.to_string(),
            msg_index,
            files: Vec::new(),
        });
        Ok(())
    }

    /// Add a file snapshot to the current open checkpoint.
    pub fn add_snap(&mut self, snap: FileSnap) {
        if let Some(ref mut cp) = self.current {
            // Dedup: only keep first snapshot per path per turn
            if !cp.files.iter().any(|s| s.path == snap.path) {
                cp.files.push(snap);
            }
        }
    }

    /// Seal the current checkpoint (move to completed list).
    pub fn seal_checkpoint(&mut self) {
        if let Some(cp) = self.current.take() {
            info!(turn = cp.turn, files = cp.files.len(), "Checkpoint sealed");
            self.checkpoints.push(cp);
        }
    }

    /// Get all sealed checkpoints.
    pub fn checkpoints(&self) -> &[Checkpoint] {
        &self.checkpoints
    }

    /// Get a checkpoint by turn number.
    pub fn get_checkpoint(&self, turn: usize) -> Option<&Checkpoint> {
        self.checkpoints.iter().find(|cp| cp.turn == turn)
    }

    /// Rewind file state to a checkpoint.
    pub fn rewind_code(&self, turn: usize) -> std::io::Result<()> {
        let cp = self
            .get_checkpoint(turn)
            .ok_or_else(|| std::io::Error::new(std::io::ErrorKind::NotFound, "checkpoint"))?;

        for snap in &cp.files {
            if let Err(e) = snap.restore() {
                warn!(path = %snap.path.display(), error = %e, "Failed to restore snapshot");
            } else {
                info!(path = %snap.path.display(), "Restored snapshot");
            }
        }
        Ok(())
    }
}
```

- [ ] **Step 5: Add to mod.rs**

In `crates/aletheon-runtime/src/core/mod.rs`, add:
```rust
pub mod checkpoint;
```

- [ ] **Step 6: Run tests**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --lib core::checkpoint::tests 2>&1 | tail -15`
Expected: all tests pass

- [ ] **Step 7: Commit**

```bash
git add crates/aletheon-abi/src/tool.rs crates/aletheon-runtime/src/core/checkpoint.rs crates/aletheon-runtime/src/core/mod.rs
git commit -m "feat: add Previewer trait + Checkpoint/rewind system

- Previewer trait on Tool for pre-edit file snapshots
- CheckpointStore with turn-based checkpoints
- FileSnap capture/restore for code rewind
- Dedup per path per turn
- RewindScope: Code, Conversation, Both"
```

---

## Task 3: Tool Parallelism Partitioning

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs`

- [ ] **Step 1: Write the failing test**

```rust
// Add to react_loop.rs tests

#[test]
fn partition_read_only_batch() {
    let calls = vec![
        ("id1".into(), "read_file".into(), json!({})),
        ("id2".into(), "glob".into(), json!({})),
        ("id3".into(), "grep".into(), json!({})),
    ];
    let batches = partition_tool_calls(&calls);
    assert_eq!(batches.len(), 1);
    assert!(matches!(&batches[0], ToolBatch::Parallel(v) if v.len() == 3));
}

#[test]
fn partition_writer_serial() {
    let calls = vec![
        ("id1".into(), "write_file".into(), json!({})),
        ("id2".into(), "edit_file".into(), json!({})),
    ];
    let batches = partition_tool_calls(&calls);
    assert_eq!(batches.len(), 1);
    assert!(matches!(&batches[0], ToolBatch::Serial(v) if v.len() == 2));
}

#[test]
fn partition_mixed() {
    let calls = vec![
        ("id1".into(), "read_file".into(), json!({})),
        ("id2".into(), "glob".into(), json!({})),
        ("id3".into(), "write_file".into(), json!({})),
        ("id4".into(), "grep".into(), json!({})),
    ];
    let batches = partition_tool_calls(&calls);
    // [read, glob] -> parallel, [write] -> serial, [grep] -> parallel
    assert_eq!(batches.len(), 3);
    assert!(matches!(&batches[0], ToolBatch::Parallel(v) if v.len() == 2));
    assert!(matches!(&batches[1], ToolBatch::Serial(v) if v.len() == 1));
    assert!(matches!(&batches[2], ToolBatch::Parallel(v) if v.len() == 1));
}

#[test]
fn partition_empty() {
    let calls: Vec<(String, String, serde_json::Value)> = vec![];
    let batches = partition_tool_calls(&calls);
    assert!(batches.is_empty());
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --lib core::react_loop::tests::partition_read_only_batch 2>&1 | tail -5`
Expected: error: cannot find function `partition_tool_calls`

- [ ] **Step 3: Write implementation**

Add to `react_loop.rs`:

```rust
use aletheon_abi::tool::ConcurrencyClass;

/// Maximum parallel tool executions.
const MAX_PARALLEL_TOOLS: usize = 8;

/// A batch of tool calls to execute together.
pub enum ToolBatch {
    /// Read-only tools that can run in parallel.
    Parallel(Vec<(String, String, serde_json::Value)>),
    /// Writer tools that must run serially.
    Serial(Vec<(String, String, serde_json::Value)>),
}

/// Classify a tool's concurrency class by name.
/// Uses known tool names rather than requiring a ToolRegistry reference.
fn classify_tool(tool_name: &str) -> ConcurrencyClass {
    match tool_name {
        "read_file" | "glob" | "grep" | "file_read" | "system_status"
        | "process_list" | "memory_search" | "ls" | "web_fetch" | "web_search" => {
            ConcurrencyClass::ReadOnly
        }
        "bash_exec" | "bash" => ConcurrencyClass::SideEffect,
        _ => ConcurrencyClass::SideEffect,
    }
}

/// Partition tool calls into contiguous read-only (parallel) and writer (serial) batches.
pub fn partition_tool_calls(
    calls: &[(String, String, serde_json::Value)],
) -> Vec<ToolBatch> {
    if calls.is_empty() {
        return Vec::new();
    }

    let mut batches: Vec<ToolBatch> = Vec::new();
    let mut current_parallel: Vec<(String, String, serde_json::Value)> = Vec::new();
    let mut current_serial: Vec<(String, String, serde_json::Value)> = Vec::new();

    for call in calls {
        let is_read_only = classify_tool(&call.1) == ConcurrencyClass::ReadOnly;

        if is_read_only {
            // Flush serial batch if any
            if !current_serial.is_empty() {
                batches.push(ToolBatch::Serial(std::mem::take(&mut current_serial)));
            }
            current_parallel.push(call.clone());
        } else {
            // Flush parallel batch if any
            if !current_parallel.is_empty() {
                batches.push(ToolBatch::Parallel(std::mem::take(&mut current_parallel)));
            }
            current_serial.push(call.clone());
        }
    }

    // Flush remaining
    if !current_parallel.is_empty() {
        batches.push(ToolBatch::Parallel(current_parallel));
    }
    if !current_serial.is_empty() {
        batches.push(ToolBatch::Serial(current_serial));
    }

    batches
}
```

- [ ] **Step 4: Run tests**

Run: `cd /home/aurobear/Bear-ws/work/aletheon && cargo test -p aletheon-runtime --lib core::react_loop::tests::partition 2>&1 | tail -15`
Expected: all partition tests pass

- [ ] **Step 5: Commit**

```bash
git add crates/aletheon-runtime/src/core/react_loop.rs
git commit -m "feat(runtime): add tool parallelism partitioning

partition_tool_calls() groups contiguous read-only tools into parallel
batches (max 8 concurrent) and writer tools into serial batches.
Read-only tools: read_file, glob, grep, file_read, etc.
Writer/side-effect tools: bash_exec, write_file, edit_file, etc."
```

---

## Final Verification

- [ ] **Run full test suite**

```bash
cd /home/aurobear/Bear-ws/work/aletheon && cargo test 2>&1 | grep "test result: ok" | awk '{sum += $4} END {print "Total:", sum, "passed"}'
```

Expected: all tests pass (existing + new)

- [ ] **Commit all changes**

```bash
git add -u && git commit -m "feat: P1 — checkpoint + tool parallelism + storm breaker

Phase 1: Checkpoint/rewind system
- Previewer trait for pre-edit file snapshots
- CheckpointStore with turn-based checkpoints
- RewindScope: Code, Conversation, Both

Phase 2: Tool parallelism partitioning
- partition_tool_calls() for concurrent read-only execution
- MAX_PARALLEL_TOOLS = 8

Phase 3: Storm breaker anti-loop
- Detects consecutive identical failures/successes
- Injects directive to change approach"
```
