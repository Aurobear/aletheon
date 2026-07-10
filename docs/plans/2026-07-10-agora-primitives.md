# Agora + Primitives Absorption — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Absorb RFC-017 (Primitives → `fabric`) and RFC-014 (Agora → new `crates/agora`), giving working-memory state a home while keeping existing IPC and cognitive types intact.

**Architecture:** Add a pure-type `primitives/` module to `fabric` (canonical cognitive objects + typed comm wrappers), then build a session-isolated `agora` crate that depends only on `fabric`. Integration lives in `executive` via two orchestration hook points. Delivered as 3 independently-mergeable PRs (C1 primitives, C2 agora crate, C3 integration).

**Tech Stack:** Rust, tokio, async-trait, serde/serde_json, chrono, anyhow. Follows the Executive Refactor grouped-PR pattern.

**Source spec:** `docs/plans/2026-07-10-agora-primitives-design.md`

---

## File Structure Map

```
crates/fabric/src/
├── ops.rs                    # MODIFY — add AgoraOps trait
├── lib.rs                    # MODIFY — declare + re-export primitives
└── primitives/               # CREATE
    ├── mod.rs                #   re-export cognitive + comm
    ├── cognitive.rs          #   5 re-exports + Hypothesis/Evidence/Narrative/Commitment
    └── comm.rs               #   Command/Query/Event/Stream/Mailbox

crates/agora/                 # CREATE (new crate)
├── Cargo.toml
└── src/
    ├── lib.rs                #   public API re-exports
    ├── scratchpad.rs         #   migrated from mnemosyne/scope.rs
    ├── blackboard.rs         #   key-value shared area
    ├── attention.rs          #   attention state
    ├── task_graph.rs         #   sub-task DAG
    ├── trace.rs              #   reasoning trace + tool/sub-agent outputs
    ├── workspace.rs          #   per-session container aggregating all above
    └── ops.rs                #   AgoraRegistry: impl AgoraOps

crates/mnemosyne/src/impl/
├── scope.rs                  # MODIFY — remove dead Scratchpad + its tests
└── mod.rs                    # MODIFY — drop Scratchpad/ScratchpadEntry/RetentionPolicy re-export

crates/executive/src/
├── core/core_systems.rs      # MODIFY — add `agora` field
└── impl/daemon/handler/
    ├── init.rs               # MODIFY — construct AgoraRegistry
    └── chat.rs               # MODIFY — recall/commit hook points

Cargo.toml                    # MODIFY — add crates/agora to members
```

---

# PR C1 — Primitives in fabric

Branch: `auro/feat/20260710-c1-primitives`. Pure-type additions to `fabric`; no existing type is changed.

### Task 1: Cognitive objects module

**Files:**
- Create: `crates/fabric/src/primitives/cognitive.rs`

- [ ] **Step 1: Write the file** — 5 re-exports of existing types + 4 new structs.

```rust
//! Cognitive objects — the canonical vocabulary of RFC-017.
//!
//! Existing types are re-exported from their current homes (no redefinition).
//! The four objects that had no home before (Hypothesis, Evidence, Narrative,
//! Commitment) are defined here as simple serde structs.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

// -- Re-exports of existing cognitive objects (single source of truth) --
pub use crate::include::brain::{Experience, Observation, Plan};
pub use crate::include::self_field::Intent;
pub use crate::policy::execpolicy::Decision;

// -- New cognitive objects --

/// A tentative explanation awaiting verification.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Hypothesis {
    pub id: String,
    pub statement: String,
    /// Confidence in [0.0, 1.0].
    pub confidence: f64,
    /// IDs of `Evidence` supporting or refuting this hypothesis.
    pub evidence_ids: Vec<String>,
}

/// A piece of evidence bearing on a hypothesis or decision.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Evidence {
    pub id: String,
    /// Where the evidence came from (tool name, memory id, observation id).
    pub source: String,
    pub content: String,
    /// Relative weight in [0.0, 1.0].
    pub weight: f64,
}

/// A running self-narrative summary.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Narrative {
    pub id: String,
    pub summary: String,
    /// Ordered narrative fragments.
    pub entries: Vec<String>,
}

/// Lifecycle of a commitment.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum CommitmentStatus {
    Open,
    Fulfilled,
    Abandoned,
}

/// A commitment the agent has made and intends to honor.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Commitment {
    pub id: String,
    pub statement: String,
    pub created_at: DateTime<Utc>,
    pub status: CommitmentStatus,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn hypothesis_roundtrips_json() {
        let h = Hypothesis {
            id: "h1".into(),
            statement: "the disk is full".into(),
            confidence: 0.8,
            evidence_ids: vec!["e1".into()],
        };
        let json = serde_json::to_string(&h).unwrap();
        let back: Hypothesis = serde_json::from_str(&json).unwrap();
        assert_eq!(back.id, "h1");
        assert_eq!(back.evidence_ids, vec!["e1".to_string()]);
    }

    #[test]
    fn commitment_status_serializes() {
        assert_eq!(
            serde_json::to_string(&CommitmentStatus::Open).unwrap(),
            "\"Open\""
        );
    }
}
```

- [ ] **Step 2: Verify the existing paths resolve** — confirm the three re-export sources exist.

Run: `grep -n "pub struct Intent" crates/fabric/src/include/self_field.rs && grep -n "pub struct Observation\|pub struct Plan\|pub struct Experience" crates/fabric/src/include/brain.rs && grep -n "Decision" crates/fabric/src/policy/execpolicy.rs | head -1`
Expected: all four grep groups return matches (Intent, Observation, Plan, Experience, Decision all found).

> If `Decision` is not in `execpolicy.rs`, fall back to `pub use crate::policy::Decision;` — it is re-exported at `fabric::policy` level (verified in `fabric/src/lib.rs:177`). Prefer the working path.

### Task 2: Communication primitives module

**Files:**
- Create: `crates/fabric/src/primitives/comm.rs`

- [ ] **Step 1: Write the file** — typed wrappers over the existing `Envelope` + a `Mailbox` trait.

```rust
//! Communication primitives — typed wrappers over the wire `Envelope`.
//!
//! Command / Query / Event / Stream make the *intent* of a message explicit at
//! the type level; each lowers to an `Envelope` with the correct `Pattern`.
//! `Mailbox` abstracts send/recv over the existing CommunicationBus.

use std::time::Duration;

use anyhow::Result;
use async_trait::async_trait;

use crate::ipc::envelope::{Endpoint, Envelope, Pattern, Payload, Target};

/// Re-export the wire envelope as a primitive.
pub use crate::ipc::envelope::Envelope as WireEnvelope;

/// A command — perform an action, no response awaited.
pub struct Command {
    pub target: Target,
    pub payload: Payload,
}

impl Command {
    pub fn new(target: Target, payload: Payload) -> Self {
        Self { target, payload }
    }
    /// Lower to an `Envelope` (FireAndForget pattern).
    pub fn into_envelope(self, source: Endpoint) -> Envelope {
        Envelope::new(source, self.target, Pattern::FireAndForget, self.payload)
    }
}

/// A query — request expecting a response within `timeout`.
pub struct Query {
    pub target: Target,
    pub payload: Payload,
    pub timeout: Duration,
}

impl Query {
    pub fn new(target: Target, payload: Payload, timeout: Duration) -> Self {
        Self { target, payload, timeout }
    }
    /// Lower to a Request `Envelope`.
    pub fn into_envelope(self, source: Endpoint) -> Envelope {
        Envelope::request(source, self.target, self.payload, self.timeout)
    }
}

/// An event — async broadcast to a topic.
pub struct Event {
    pub topic: String,
    pub payload: Payload,
}

impl Event {
    pub fn new(topic: impl Into<String>, payload: Payload) -> Self {
        Self { topic: topic.into(), payload }
    }
    /// Lower to a Publish `Envelope` targeting the topic.
    pub fn into_envelope(self, source: Endpoint) -> Envelope {
        Envelope::new(source, Target::Topic(self.topic), Pattern::Publish, self.payload)
    }
}

/// A stream — continuous data flow keyed by a session id.
pub struct Stream {
    pub target: Target,
    pub session_id: u64,
    pub payload: Payload,
}

impl Stream {
    pub fn new(target: Target, session_id: u64, payload: Payload) -> Self {
        Self { target, session_id, payload }
    }
    /// Lower to a Stream `Envelope`.
    pub fn into_envelope(self, source: Endpoint) -> Envelope {
        Envelope::new(
            source,
            self.target,
            Pattern::Stream { session_id: self.session_id },
            self.payload,
        )
    }
}

/// A mailbox — abstract send/recv endpoint. Backed by CommunicationBus.
#[async_trait]
pub trait Mailbox: Send + Sync {
    async fn send(&self, envelope: Envelope) -> Result<()>;
    async fn recv(&self) -> Option<Envelope>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn command_lowers_to_fire_and_forget() {
        let cmd = Command::new(Target::Broadcast, Payload::Empty);
        let env = cmd.into_envelope(Endpoint::System);
        assert!(matches!(env.pattern, Pattern::FireAndForget));
    }

    #[test]
    fn query_lowers_to_request() {
        let q = Query::new(Target::Broadcast, Payload::Empty, Duration::from_millis(500));
        let env = q.into_envelope(Endpoint::System);
        assert!(matches!(env.pattern, Pattern::Request { .. }));
    }

    #[test]
    fn event_targets_topic() {
        let e = Event::new("evolution", Payload::Empty);
        let env = e.into_envelope(Endpoint::System);
        assert!(matches!(env.target, Target::Topic(ref t) if t == "evolution"));
    }
}
```

- [ ] **Step 2: Verify `Envelope::request` signature matches** the call above.

Run: `grep -n "pub fn request" crates/fabric/src/ipc/envelope.rs`
Expected: `pub fn request(source: Endpoint, target: Target, payload: Payload, timeout: Duration) -> Self` (4 positional args in this order).

### Task 3: Wire the primitives module + AgoraOps trait

**Files:**
- Create: `crates/fabric/src/primitives/mod.rs`
- Modify: `crates/fabric/src/lib.rs` (module declarations block ~line 24-31, re-export block)
- Modify: `crates/fabric/src/ops.rs` (append AgoraOps trait)

- [ ] **Step 1: Write `primitives/mod.rs`**

```rust
//! RFC-017 primitives — the canonical shared vocabulary.
//!
//! Every subsystem communicates using these primitives instead of concrete
//! implementations. Pure types only; no business logic.

pub mod cognitive;
pub mod comm;

pub use cognitive::{
    Commitment, CommitmentStatus, Decision, Evidence, Experience, Hypothesis, Intent, Narrative,
    Observation, Plan,
};
pub use comm::{Command, Event, Mailbox, Query, Stream};
```

- [ ] **Step 2: Register the module in `lib.rs`** — add to the `=== Module declarations ===` block (alphabetical, after `policy`):

```rust
pub mod policy;
pub mod primitives;
pub mod types;
```

- [ ] **Step 3: Add a convenience re-export in `lib.rs`** — after the policy re-export block (~line 88), add:

```rust
// Primitives (RFC-017 canonical vocabulary)
pub use primitives::{Command, Commitment, Event, Evidence, Hypothesis, Mailbox, Narrative, Query, Stream};
```

> Note: `Intent`, `Observation`, `Plan`, `Experience`, `Decision` are already re-exported at crate root from their `include`/`policy` homes — do not re-export them again here (would be a duplicate-import error). Only the four new objects + comm wrappers are added.

- [ ] **Step 4: Append `AgoraOps` to `crates/fabric/src/ops.rs`** (after `CorpusOps`, before the Harness section ~line 55):

```rust
/// Agora (working-memory) operations — the shared cognitive workspace.
///
/// Session-scoped, in-memory. Persists only via `snapshot()` → Mnemosyne.
#[async_trait]
pub trait AgoraOps: Send + Sync {
    /// Write a value onto a session's blackboard.
    async fn publish(&self, session: &str, key: &str, value: serde_json::Value) -> Result<()>;
    /// Read a value from a session's blackboard.
    async fn recall(&self, session: &str, key: &str) -> Result<Option<serde_json::Value>>;
    /// Merge a JSON patch into the session workspace.
    async fn update(&self, session: &str, patch: serde_json::Value) -> Result<()>;
    /// Snapshot the entire session workspace (for debug / commit).
    async fn snapshot(&self, session: &str) -> Result<serde_json::Value>;
    /// Clear a session's workspace.
    async fn clear(&self, session: &str) -> Result<()>;
}
```

### Task 4: Verify C1

- [ ] **Step 1: Build, test, lint, format**

Run:
```bash
cargo build -p fabric && \
cargo test -p fabric primitives && \
cargo clippy -p fabric -- -D warnings && \
cargo fmt --all --check
```
Expected: build OK; the 6 primitives tests pass; clippy clean; fmt clean.

- [ ] **Step 2: Confirm no existing test broke**

Run: `cargo test --workspace 2>&1 | grep -E "test result:.*failed" | grep -v "0 failed" || echo "ALL GREEN"`
Expected: `ALL GREEN`.

- [ ] **Step 3: Commit + PR**

```bash
git checkout -b auro/feat/20260710-c1-primitives
git add crates/fabric/src/primitives crates/fabric/src/lib.rs crates/fabric/src/ops.rs
git commit -m "feat(fabric): add RFC-017 primitives (cognitive objects + comm) and AgoraOps trait

Co-Authored-By: Claude <noreply@anthropic.com>"
git push -u origin auro/feat/20260710-c1-primitives
gh pr create --base dev --title "feat(fabric): RFC-017 primitives + AgoraOps trait (C1)" --body "See docs/plans/2026-07-10-agora-primitives.md PR C1"
```

---

# PR C2 — Agora crate

Branch: `auro/feat/20260710-c2-agora`. New `crates/agora` depending only on `fabric`. Implements `AgoraOps` and migrates the dead `Scratchpad` out of mnemosyne.

### Task 5: Scaffold the crate

**Files:**
- Create: `crates/agora/Cargo.toml`
- Create: `crates/agora/src/lib.rs`
- Modify: `Cargo.toml` (workspace members)

- [ ] **Step 1: Write `crates/agora/Cargo.toml`**

```toml
[package]
name = "agora"
version.workspace = true
edition.workspace = true
rust-version.workspace = true
license.workspace = true
description = "Aletheon Agora - shared cognitive workspace (working memory, blackboard, scratchpad)"

[dependencies]
fabric = { path = "../fabric" }
tokio = { version = "1", features = ["full"] }
anyhow = "1"
async-trait = "0.1"
serde = { version = "1", features = ["derive"] }
serde_json = "1"
tracing = "0.1"

[dev-dependencies]
tokio = { version = "1", features = ["full", "test-util"] }
```

- [ ] **Step 2: Add to workspace members** in root `Cargo.toml` — insert after `"crates/mnemosyne",`:

```toml
    "crates/mnemosyne",
    "crates/agora",
    "crates/metacog",
```

- [ ] **Step 3: Write initial `crates/agora/src/lib.rs`** (modules added as tasks land)

```rust
//! # Aletheon Agora
//!
//! The shared cognitive workspace (RFC-014). Session-isolated, in-memory.
//! Holds working memory: blackboard, attention, task graph, scratchpad, and
//! reasoning trace. Never persistent by itself — persists via snapshot →
//! Mnemosyne, orchestrated by the executive layer.

pub mod attention;
pub mod blackboard;
pub mod ops;
pub mod scratchpad;
pub mod task_graph;
pub mod trace;
pub mod workspace;

pub use attention::Attention;
pub use blackboard::Blackboard;
pub use ops::AgoraRegistry;
pub use scratchpad::{RetentionPolicy, Scratchpad, ScratchpadEntry};
pub use task_graph::{TaskGraph, TaskNode, TaskStatus};
pub use trace::{Trace, TraceEntry};
pub use workspace::Workspace;
```

- [ ] **Step 4: Verify it builds empty** (create empty module files first, or write all modules in Tasks 6-12 before building). Build deferred to Task 13.

### Task 6: Scratchpad module (migrated)

**Files:**
- Create: `crates/agora/src/scratchpad.rs`

- [ ] **Step 1: Write the file** — verbatim migration of the dead code from `mnemosyne/scope.rs:335-429` plus its two tests from `scope.rs:750-783`.

```rust
//! Scratchpad — task-level ephemeral workspace (migrated from mnemosyne, RFC-014).

use serde::{Deserialize, Serialize};

/// Retention policy for a scratchpad when the task completes.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RetentionPolicy {
    /// Discard all entries immediately.
    Discard,
    /// Archive entries into the owning agent's private memory.
    ArchiveToAgent,
    /// Archive entries into the session-scoped memory (visible to parent).
    ArchiveToSession,
}

/// A single entry in a scratchpad.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScratchpadEntry {
    pub key: String,
    pub value: String,
}

/// Task-level scratch space for an agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Scratchpad {
    pub agent_id: String,
    pub task_id: String,
    pub entries: Vec<ScratchpadEntry>,
    pub retention: RetentionPolicy,
}

impl Scratchpad {
    pub fn new(
        agent_id: impl Into<String>,
        task_id: impl Into<String>,
        retention: RetentionPolicy,
    ) -> Self {
        Self {
            agent_id: agent_id.into(),
            task_id: task_id.into(),
            entries: Vec::new(),
            retention,
        }
    }

    pub fn set(&mut self, key: impl Into<String>, value: impl Into<String>) {
        let key = key.into();
        let value = value.into();
        if let Some(entry) = self.entries.iter_mut().find(|e| e.key == key) {
            entry.value = value;
        } else {
            self.entries.push(ScratchpadEntry { key, value });
        }
    }

    pub fn get(&self, key: &str) -> Option<&str> {
        self.entries.iter().find(|e| e.key == key).map(|e| e.value.as_str())
    }

    pub fn remove(&mut self, key: &str) -> bool {
        let len_before = self.entries.len();
        self.entries.retain(|e| e.key != key);
        self.entries.len() < len_before
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn format_entries(&self) -> String {
        self.entries
            .iter()
            .map(|e| format!("[{}]: {}", e.key, e.value))
            .collect::<Vec<_>>()
            .join("\n")
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_scratchpad_basic_operations() {
        let mut sp = Scratchpad::new("agent-1", "task-42", RetentionPolicy::Discard);
        sp.set("step", "1");
        sp.set("result", "ok");
        assert_eq!(sp.get("step"), Some("1"));
        assert_eq!(sp.get("result"), Some("ok"));
        assert_eq!(sp.len(), 2);
        sp.set("step", "2");
        assert_eq!(sp.get("step"), Some("2"));
        assert_eq!(sp.len(), 2);
        assert!(sp.remove("result"));
        assert!(!sp.remove("nonexistent"));
        assert_eq!(sp.len(), 1);
        sp.clear();
        assert!(sp.is_empty());
    }

    #[test]
    fn test_scratchpad_format_entries() {
        let mut sp = Scratchpad::new("agent-1", "task-1", RetentionPolicy::ArchiveToAgent);
        sp.set("a", "1");
        sp.set("b", "2");
        let formatted = sp.format_entries();
        assert!(formatted.contains("[a]: 1"));
        assert!(formatted.contains("[b]: 2"));
    }
}
```

### Task 7: Blackboard module

**Files:**
- Create: `crates/agora/src/blackboard.rs`

- [ ] **Step 1: Write the file** — a JSON key-value store (absorbs observation/artifact/context per the design).

```rust
//! Blackboard — key-value shared area for hypotheses, evidence, and
//! intermediate conclusions. Absorbs observation/artifact/context (RFC-014).

use std::collections::HashMap;

use serde_json::Value;

/// A JSON key-value shared workspace area.
#[derive(Debug, Clone, Default)]
pub struct Blackboard {
    entries: HashMap<String, Value>,
}

impl Blackboard {
    pub fn new() -> Self {
        Self { entries: HashMap::new() }
    }

    /// Write (or overwrite) a key.
    pub fn set(&mut self, key: impl Into<String>, value: Value) {
        self.entries.insert(key.into(), value);
    }

    /// Read a key.
    pub fn get(&self, key: &str) -> Option<&Value> {
        self.entries.get(key)
    }

    /// Remove a key; returns the removed value if present.
    pub fn remove(&mut self, key: &str) -> Option<Value> {
        self.entries.remove(key)
    }

    /// Merge a JSON object patch (top-level keys) into the blackboard.
    /// Non-object patches are ignored.
    pub fn merge(&mut self, patch: Value) {
        if let Value::Object(map) = patch {
            for (k, v) in map {
                self.entries.insert(k, v);
            }
        }
    }

    /// Serialize all entries to a JSON object.
    pub fn to_json(&self) -> Value {
        Value::Object(self.entries.clone().into_iter().collect())
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn set_get_remove() {
        let mut bb = Blackboard::new();
        bb.set("k", json!(1));
        assert_eq!(bb.get("k"), Some(&json!(1)));
        assert_eq!(bb.remove("k"), Some(json!(1)));
        assert!(bb.is_empty());
    }

    #[test]
    fn merge_object_patch() {
        let mut bb = Blackboard::new();
        bb.set("a", json!(1));
        bb.merge(json!({"b": 2, "a": 9}));
        assert_eq!(bb.get("a"), Some(&json!(9)));
        assert_eq!(bb.get("b"), Some(&json!(2)));
    }

    #[test]
    fn to_json_roundtrips() {
        let mut bb = Blackboard::new();
        bb.set("x", json!("y"));
        assert_eq!(bb.to_json(), json!({"x": "y"}));
    }
}
```

### Task 8: Attention module

**Files:**
- Create: `crates/agora/src/attention.rs`

- [ ] **Step 1: Write the file** — current focus + priority list.

```rust
//! Attention — the workspace's current focus and priority ordering (RFC-014).

use serde::{Deserialize, Serialize};

/// Attention state: the current focus and a ranked list of foci.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Attention {
    /// The single current focus, if any.
    pub focus: Option<String>,
    /// Ranked foci, highest priority first.
    pub priorities: Vec<String>,
}

impl Attention {
    pub fn new() -> Self {
        Self::default()
    }

    /// Set the current focus and push it to the front of the priority list.
    pub fn set_focus(&mut self, focus: impl Into<String>) {
        let f = focus.into();
        self.priorities.retain(|p| p != &f);
        self.priorities.insert(0, f.clone());
        self.focus = Some(f);
    }

    /// Clear the current focus (priorities are retained).
    pub fn clear_focus(&mut self) {
        self.focus = None;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn set_focus_updates_priorities() {
        let mut a = Attention::new();
        a.set_focus("task-a");
        a.set_focus("task-b");
        assert_eq!(a.focus.as_deref(), Some("task-b"));
        assert_eq!(a.priorities, vec!["task-b".to_string(), "task-a".to_string()]);
    }

    #[test]
    fn refocus_dedups() {
        let mut a = Attention::new();
        a.set_focus("x");
        a.set_focus("y");
        a.set_focus("x");
        assert_eq!(a.priorities, vec!["x".to_string(), "y".to_string()]);
    }
}
```

### Task 9: Task graph module

**Files:**
- Create: `crates/agora/src/task_graph.rs`

- [ ] **Step 1: Write the file** — sub-task nodes with dependencies + status.

```rust
//! Task graph — sub-task nodes, dependencies, and status (RFC-014).

use std::collections::HashMap;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum TaskStatus {
    Pending,
    Running,
    Done,
    Failed,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TaskNode {
    pub id: String,
    pub description: String,
    pub status: TaskStatus,
    /// IDs of tasks that must complete before this one.
    pub deps: Vec<String>,
}

/// A directed task graph keyed by task id.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TaskGraph {
    nodes: HashMap<String, TaskNode>,
}

impl TaskGraph {
    pub fn new() -> Self {
        Self { nodes: HashMap::new() }
    }

    pub fn add(&mut self, id: impl Into<String>, description: impl Into<String>, deps: Vec<String>) {
        let id = id.into();
        self.nodes.insert(
            id.clone(),
            TaskNode { id, description: description.into(), status: TaskStatus::Pending, deps },
        );
    }

    pub fn set_status(&mut self, id: &str, status: TaskStatus) -> bool {
        if let Some(node) = self.nodes.get_mut(id) {
            node.status = status;
            true
        } else {
            false
        }
    }

    pub fn get(&self, id: &str) -> Option<&TaskNode> {
        self.nodes.get(id)
    }

    /// Tasks whose dependencies are all `Done` and are still `Pending`.
    pub fn ready(&self) -> Vec<&TaskNode> {
        self.nodes
            .values()
            .filter(|n| n.status == TaskStatus::Pending)
            .filter(|n| {
                n.deps.iter().all(|d| {
                    self.nodes.get(d).map(|dn| dn.status == TaskStatus::Done).unwrap_or(false)
                })
            })
            .collect()
    }

    pub fn len(&self) -> usize {
        self.nodes.len()
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ready_respects_deps() {
        let mut g = TaskGraph::new();
        g.add("a", "first", vec![]);
        g.add("b", "second", vec!["a".into()]);
        // Only `a` is ready initially.
        let ready: Vec<_> = g.ready().iter().map(|n| n.id.clone()).collect();
        assert_eq!(ready, vec!["a".to_string()]);
        // After a is done, b becomes ready.
        assert!(g.set_status("a", TaskStatus::Done));
        let ready: Vec<_> = g.ready().iter().map(|n| n.id.clone()).collect();
        assert_eq!(ready, vec!["b".to_string()]);
    }
}
```

### Task 10: Trace module

**Files:**
- Create: `crates/agora/src/trace.rs`

- [ ] **Step 1: Write the file** — append-only reasoning trace (tool outputs + sub-agent results).

```rust
//! Trace — append-only reasoning trace: tool outputs and sub-agent results (RFC-014).

use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TraceEntry {
    /// Kind of trace event, e.g. "reasoning", "tool_output", "sub_agent".
    pub kind: String,
    pub content: Value,
}

/// Append-only reasoning trace for a session.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct Trace {
    entries: Vec<TraceEntry>,
}

impl Trace {
    pub fn new() -> Self {
        Self { entries: Vec::new() }
    }

    pub fn push(&mut self, kind: impl Into<String>, content: Value) {
        self.entries.push(TraceEntry { kind: kind.into(), content });
    }

    pub fn entries(&self) -> &[TraceEntry] {
        &self.entries
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn clear(&mut self) {
        self.entries.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn push_and_read() {
        let mut t = Trace::new();
        t.push("tool_output", json!({"tool": "bash", "ok": true}));
        assert_eq!(t.len(), 1);
        assert_eq!(t.entries()[0].kind, "tool_output");
    }
}
```

### Task 11: Workspace module

**Files:**
- Create: `crates/agora/src/workspace.rs`

- [ ] **Step 1: Write the file** — per-session container aggregating all components.

```rust
//! Workspace — a single session's cognitive workspace, aggregating all
//! working-memory components (RFC-014).

use serde_json::{json, Value};

use crate::attention::Attention;
use crate::blackboard::Blackboard;
use crate::task_graph::TaskGraph;
use crate::trace::Trace;

/// One session's in-memory cognitive workspace.
#[derive(Debug, Clone, Default)]
pub struct Workspace {
    pub session_id: String,
    pub blackboard: Blackboard,
    pub attention: Attention,
    pub task_graph: TaskGraph,
    pub trace: Trace,
}

impl Workspace {
    pub fn new(session_id: impl Into<String>) -> Self {
        Self {
            session_id: session_id.into(),
            blackboard: Blackboard::new(),
            attention: Attention::new(),
            task_graph: TaskGraph::new(),
            trace: Trace::new(),
        }
    }

    /// Snapshot the workspace to JSON (for debug / commit to Mnemosyne).
    pub fn snapshot(&self) -> Value {
        json!({
            "session_id": self.session_id,
            "blackboard": self.blackboard.to_json(),
            "attention": {
                "focus": self.attention.focus,
                "priorities": self.attention.priorities,
            },
            "task_count": self.task_graph.len(),
            "trace_len": self.trace.len(),
        })
    }

    /// Clear all workspace state (keeps the session id).
    pub fn clear(&mut self) {
        self.blackboard.clear();
        self.attention = Attention::new();
        self.task_graph = TaskGraph::new();
        self.trace.clear();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn snapshot_includes_session_and_blackboard() {
        let mut ws = Workspace::new("s1");
        ws.blackboard.set("goal", json!("ship it"));
        let snap = ws.snapshot();
        assert_eq!(snap["session_id"], json!("s1"));
        assert_eq!(snap["blackboard"]["goal"], json!("ship it"));
    }

    #[test]
    fn clear_resets_state() {
        let mut ws = Workspace::new("s1");
        ws.blackboard.set("k", json!(1));
        ws.clear();
        assert!(ws.blackboard.is_empty());
        assert_eq!(ws.session_id, "s1");
    }
}
```

### Task 12: AgoraRegistry — implement AgoraOps

**Files:**
- Create: `crates/agora/src/ops.rs`

- [ ] **Step 1: Write the file** — multi-session registry implementing `fabric::ops::AgoraOps`.

```rust
//! AgoraRegistry — manages per-session Workspaces and implements AgoraOps.

use std::collections::HashMap;

use anyhow::Result;
use async_trait::async_trait;
use serde_json::Value;
use tokio::sync::Mutex;

use fabric::ops::AgoraOps;

use crate::workspace::Workspace;

/// Owns one `Workspace` per session id. Cheap to clone via `Arc`.
#[derive(Default)]
pub struct AgoraRegistry {
    sessions: Mutex<HashMap<String, Workspace>>,
}

impl AgoraRegistry {
    pub fn new() -> Self {
        Self { sessions: Mutex::new(HashMap::new()) }
    }
}

#[async_trait]
impl AgoraOps for AgoraRegistry {
    async fn publish(&self, session: &str, key: &str, value: Value) -> Result<()> {
        let mut map = self.sessions.lock().await;
        let ws = map.entry(session.to_string()).or_insert_with(|| Workspace::new(session));
        ws.blackboard.set(key, value);
        Ok(())
    }

    async fn recall(&self, session: &str, key: &str) -> Result<Option<Value>> {
        let map = self.sessions.lock().await;
        Ok(map.get(session).and_then(|ws| ws.blackboard.get(key).cloned()))
    }

    async fn update(&self, session: &str, patch: Value) -> Result<()> {
        let mut map = self.sessions.lock().await;
        let ws = map.entry(session.to_string()).or_insert_with(|| Workspace::new(session));
        ws.blackboard.merge(patch);
        Ok(())
    }

    async fn snapshot(&self, session: &str) -> Result<Value> {
        let map = self.sessions.lock().await;
        Ok(map.get(session).map(|ws| ws.snapshot()).unwrap_or(Value::Null))
    }

    async fn clear(&self, session: &str) -> Result<()> {
        let mut map = self.sessions.lock().await;
        if let Some(ws) = map.get_mut(session) {
            ws.clear();
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[tokio::test]
    async fn publish_then_recall() {
        let reg = AgoraRegistry::new();
        reg.publish("s1", "k", json!("v")).await.unwrap();
        assert_eq!(reg.recall("s1", "k").await.unwrap(), Some(json!("v")));
    }

    #[tokio::test]
    async fn recall_missing_session_is_none() {
        let reg = AgoraRegistry::new();
        assert_eq!(reg.recall("nope", "k").await.unwrap(), None);
    }

    #[tokio::test]
    async fn update_merges_patch() {
        let reg = AgoraRegistry::new();
        reg.publish("s1", "a", json!(1)).await.unwrap();
        reg.update("s1", json!({"b": 2})).await.unwrap();
        assert_eq!(reg.recall("s1", "b").await.unwrap(), Some(json!(2)));
    }

    #[tokio::test]
    async fn snapshot_and_clear() {
        let reg = AgoraRegistry::new();
        reg.publish("s1", "k", json!(1)).await.unwrap();
        let snap = reg.snapshot("s1").await.unwrap();
        assert_eq!(snap["blackboard"]["k"], json!(1));
        reg.clear("s1").await.unwrap();
        assert_eq!(reg.recall("s1", "k").await.unwrap(), None);
    }
}
```

### Task 13: Remove dead Scratchpad from mnemosyne

**Files:**
- Modify: `crates/mnemosyne/src/impl/scope.rs` (delete lines 331-429 block + tests 748-783)
- Modify: `crates/mnemosyne/src/impl/mod.rs:31-32` (drop the re-export)

- [ ] **Step 1: Delete the Scratchpad definition** — remove `scope.rs` lines 331-429 (the `// Scratchpad --` header through the end of `impl Scratchpad`), and the two tests `test_scratchpad_basic_operations` / `test_scratchpad_format_entries` (lines ~748-783).

- [ ] **Step 2: Update the re-export in `mod.rs`** — change:

```rust
    RetentionPolicy, ScopeFilter, ScopedCoreMemory, ScopedMemoryBlock, ScopedRecallFilter,
    Scratchpad, ScratchpadEntry, WriteOutcome,
```
to:
```rust
    ScopeFilter, ScopedCoreMemory, ScopedMemoryBlock, ScopedRecallFilter, WriteOutcome,
```

- [ ] **Step 3: Verify no dangling references**

Run: `grep -rn "Scratchpad\|RetentionPolicy" crates/mnemosyne/ | grep -v SharedScratchpad`
Expected: no output (all mnemosyne references gone; `SharedScratchpad` in executive/kernel is untouched).

### Task 14: Verify C2

- [ ] **Step 1: Build + test the new crate and mnemosyne**

Run:
```bash
cargo build -p agora -p mnemosyne && \
cargo test -p agora && \
cargo clippy -p agora -p mnemosyne -- -D warnings && \
cargo fmt --all --check
```
Expected: agora builds; ~15 agora unit tests pass; mnemosyne still builds without the removed Scratchpad; clippy + fmt clean.

- [ ] **Step 2: Full workspace green**

Run: `cargo test --workspace 2>&1 | grep -E "test result:.*failed" | grep -v "0 failed" || echo "ALL GREEN"`
Expected: `ALL GREEN`.

- [ ] **Step 3: Commit + PR**

```bash
git checkout dev && git pull origin dev
git checkout -b auro/feat/20260710-c2-agora
git add crates/agora Cargo.toml crates/mnemosyne/src/impl/scope.rs crates/mnemosyne/src/impl/mod.rs
git commit -m "feat(agora): new shared cognitive workspace crate (RFC-014)

Migrate dead Scratchpad out of mnemosyne; add blackboard, attention,
task_graph, trace, workspace, and AgoraRegistry (impl AgoraOps).

Co-Authored-By: Claude <noreply@anthropic.com>"
git push -u origin auro/feat/20260710-c2-agora
gh pr create --base dev --title "feat(agora): shared cognitive workspace crate (C2)" --body "See docs/plans/2026-07-10-agora-primitives.md PR C2"
```

---

# PR C3 — Integration (minimal-viable)

Branch: `auro/feat/20260710-c3-integration`. Wire Agora into `executive`; add recall/commit hook points. Minimal-viable: workspace exists, is read/written, snapshotted at turn end.

### Task 15: Add `agora` to CoreSystems + executive dep

**Files:**
- Modify: `crates/executive/Cargo.toml` (add agora dep)
- Modify: `crates/executive/src/core/core_systems.rs` (import + field)

- [ ] **Step 1: Add the dependency** in `crates/executive/Cargo.toml` `[dependencies]`:

```toml
agora = { path = "../agora" }
```

- [ ] **Step 2: Import in `core_systems.rs`** — add near the other subsystem imports (~line 25):

```rust
use agora::AgoraRegistry;
```

- [ ] **Step 3: Add the field** to `struct CoreSystems` — after the `reflector` field (~line 61):

```rust
    /// Shared cognitive workspace (RFC-014). Session-isolated working memory.
    pub agora: Arc<AgoraRegistry>,
```

### Task 16: Construct AgoraRegistry in init

**Files:**
- Modify: `crates/executive/src/impl/daemon/handler/init.rs` (~line 542, inside the `CoreSystems { ... }` literal)

- [ ] **Step 1: Add the field initializer** — after `reflector,` in the `CoreSystems { ... }` construction (~line 542):

```rust
            reflector,
            agora: Arc::new(agora::AgoraRegistry::new()),
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo build -p executive 2>&1 | tail -5`
Expected: `Finished` (no missing-field or unresolved-import errors).

### Task 17: Recall/commit hook points

**Files:**
- Modify: `crates/executive/src/impl/daemon/handler/chat.rs`

- [ ] **Step 1: Locate the turn boundaries** — find where a turn begins (after messages assembled, before harness runs) and ends (after final response).

Run: `grep -n "recall\|fn handle_chat\|self.subsystems" crates/executive/src/impl/daemon/handler/chat.rs | head -20`
Expected: identifies the chat handler entry and existing subsystem-access sites to anchor the hooks near.

- [ ] **Step 2: Add the recall-injection hook (turn start)** — after the session id is known and before the harness/reasoning runs, inject a compact working-memory marker. Minimal-viable: publish the session goal onto the blackboard so the workspace is populated and observable.

```rust
// RFC-014 recall injection: seed the Agora workspace for this turn.
if let Err(e) = self
    .subsystems
    .agora
    .publish(&session_id, "turn_input", serde_json::json!(user_input))
    .await
{
    tracing::warn!("agora publish (recall injection) failed: {e}");
}
```

> `session_id` and `user_input` are the existing local bindings in `handle_chat`; if their names differ, use the actual bindings found in Step 1 (e.g. `sid`, `message`). Do not introduce new state.

- [ ] **Step 3: Add the commit hook (turn end)** — after the final assistant response is produced, snapshot the workspace and log it (minimal-viable: snapshot + trace, no Mnemosyne write yet — deferred as documented).

```rust
// RFC-014 commit: snapshot the Agora workspace at turn end.
match self.subsystems.agora.snapshot(&session_id).await {
    Ok(snap) => tracing::debug!(target: "agora", "workspace snapshot: {snap}"),
    Err(e) => tracing::warn!("agora snapshot failed: {e}"),
}
```

> Minimal-viable per design §5b: the snapshot is logged, not yet persisted via `MnemosyneOps::store()`. Deep commit is a later increment. The hook point exists so the later increment is a one-line change.

- [ ] **Step 4: Verify the hooks compile and the AgoraOps trait is in scope**

Ensure `use fabric::ops::AgoraOps;` is present at the top of `chat.rs` (the trait must be in scope to call `.publish()`/`.snapshot()`). Add it if missing.

Run: `cargo build -p executive 2>&1 | tail -5`
Expected: `Finished`.

### Task 18: Verify C3

- [ ] **Step 1: Full verification**

Run:
```bash
cargo build --workspace && \
cargo test --workspace 2>&1 | grep -E "test result:.*failed" | grep -v "0 failed" || echo "ALL GREEN"
cargo clippy --workspace -- -D warnings && \
cargo fmt --all --check
```
Expected: build OK; `ALL GREEN`; clippy clean; fmt clean.

- [ ] **Step 2: Commit + PR**

```bash
git checkout dev && git pull origin dev
git checkout -b auro/feat/20260710-c3-integration
git add crates/executive/Cargo.toml crates/executive/src/core/core_systems.rs crates/executive/src/impl/daemon/handler/init.rs crates/executive/src/impl/daemon/handler/chat.rs
git commit -m "feat(executive): wire Agora into CoreSystems + recall/commit hooks (RFC-014)

Minimal-viable integration: per-session workspace seeded at turn start,
snapshotted at turn end. Mnemosyne persistence deferred to a later increment.

Co-Authored-By: Claude <noreply@anthropic.com>"
git push -u origin auro/feat/20260710-c3-integration
gh pr create --base dev --title "feat(executive): Agora integration + recall/commit hooks (C3)" --body "See docs/plans/2026-07-10-agora-primitives.md PR C3"
```

---

## Self-Review Notes

- **Spec coverage:** RFC-017 cognitive objects → Task 1; comm primitives → Task 2; AgoraOps → Task 3. RFC-014 crate + 6 modules → Tasks 5-12; Scratchpad migration → Tasks 6 & 13; CoreSystems wiring → Tasks 15-16; recall/commit hooks → Task 17. Boundary decisions (SharedScratchpad kept, agora→fabric only, no persistence yet) honored in Tasks 13, 5, 17.
- **Deferred (per design, not omissions):** Mnemosyne `store()` on commit (Task 17 Step 3), deep trace/task_graph population from the harness, RFC-015/016.
- **Type consistency:** `AgoraOps` signature identical in fabric (Task 3) and impl (Task 12). `session`/`key`/`value` param names match. `RetentionPolicy`/`Scratchpad`/`ScratchpadEntry` names preserved across migration (Task 6) and removal (Task 13).
- **Risk:** Task 17 depends on exact local binding names in `chat.rs` — Step 1 grep resolves them before editing; guidance says use actual bindings, add no new state.
```
