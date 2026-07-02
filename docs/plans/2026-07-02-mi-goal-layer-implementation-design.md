# M-I -- Goal Layer / Persistent Objectives -- Refined Implementation Design

**Date:** 2026-07-02
**Source plan:** `docs/plans/2026-07-01-mi-goal-layer-plan.md`
**Source roadmap:** `docs/plans/2026-07-01-modules-roadmap-design.md` § M-I
**Status:** Design (design-only gate; no product code changes)
**Branch:** `auro/feat/20260701-aletheon-governed-memory-design`

This document refines the 2026-07-01 M-I plan against verified codebase ground truth. Every claim in the original plan's ground truth table has been checked against the actual filesystem. Drifts are noted and corrected. Every insertion point carries a verified file path and line number.

---

## 1. Verified Ground Truth Table

| # | Claim | Plan Line Ref | Actual | Verdict |
|---|---|---|---|---|
| 1 | Cargo package names are bare (`runtime`,`interact`,`cognit`,`base`) | -- | `crates/runtime/Cargo.toml` `name = "runtime"`; `crates/interact/Cargo.toml` `name = "interact"`; `crates/cognit/Cargo.toml` `name = "cognit"`; `crates/base/Cargo.toml` `name = "base"` | **MATCH** |
| 2 | Goal state is in-memory + react-loop-scoped | `goal_tracker.rs:70,55,46` | `goal_tracker.rs:70` `pub struct GoalTracker`; `:55` `pub struct Goal`; `:46` `pub enum GoalStatus` | **MATCH** |
| 3 | Goal/GoalStatus NOT serde-serializable | `goal_tracker.rs:45,54` | `GoalStatus`: `#[derive(Debug, Clone, PartialEq)]`; `Goal`: `#[derive(Debug, Clone)]`. No `Serialize`/`Deserialize` | **MATCH** |
| 4 | GoalTracker created fresh per loop, reset per turn | `react_loop/mod.rs` | `mod.rs:171` `let goal_tracker = GoalTracker::new();` inside `ReActLoop::new()`; `mod.rs:215` `self.goal_tracker.reset();` inside `ReActLoop::reset()` | **MATCH** |
| 5 | React loop reads only description | `step.rs`, `tool_exec.rs` | `step.rs:176` `goal: self.goal_tracker.current_goal_description()`; `tool_exec.rs:318` same pattern. Both inside deferred reflection `ReflectionContext` construction, not in the main reasoning prompt injection | **MATCH** -- but note: description is used for reflection context only, not primary context injection |
| 6 | Decomposition lives in INTERFACE crate | `interact/src/acix/task.rs:204,216,221` | `:204` `pub fn decompose(goal: &str) -> TaskGraph`; `:216` `decompose_simple` (alias); `:221` `pub async fn decompose_with_llm` | **MATCH** |
| 7 | Brain has planner for decomposition | `cognit/src/core/planner.rs:52,187` | `:52` `pub fn generate_multi_step_plan(&self, intent: &Intent, reasoning: &str, sub_actions: Vec<(String, Value)>) -> Plan`; `:187` `pub fn parse_subtasks(&self, llm_output: &str) -> Option<Vec<(String, Value)>>` | **MATCH** -- but note: `generate_multi_step_plan` takes an `Intent` + pre-computed `sub_actions`, and `parse_subtasks` operates on raw LLM output. Neither is a drop-in "decompose an objective description string into sub-goals" entry. Phase 6 needs a new thin entry that chains: objective description -> LLM prompt -> parse_subtasks -> persist as sub-goals. |
| 8 | FactStore pattern: WAL, CREATE TABLE IF NOT EXISTS, positional map_fact_row | `fact_store/mod.rs:97,99,100,104,92,250` | `:91-93` struct with `pub(crate) db: Connection`; `:97` `pub fn open(path)`; `:99` `PRAGMA journal_mode=WAL;`; `:100` `Self::create_schema(&db)?;`; `:107` `CREATE TABLE IF NOT EXISTS facts`; `:250` `pub(crate) fn map_fact_row` (positional `row.get(0..11)`) | **MATCH** -- minor drift: plan says line 104 for CREATE TABLE, actual is 107 (schema starts at 104 but the INSERT statement begins there, the CREATE TABLE statement body spans 107-120). The claim is functionally correct; just line offset. |
| 9 | `add_fact` insert idiom: INSERT then `query_row` for id | `fact_store/query.rs:14,24,29` | `:14` `pub fn add_fact(..) -> Result<i64>`; `:25` `INSERT OR IGNORE`; `:29` `SELECT fact_id ... query_row` | **DRIFT** -- plan says line 24 for INSERT, actual is 25 (off by 1). Line 29 for query_row is correct. |
| 10 | rusqlite stores under `impl/`, not `core/` | FactStore, SessionStore, journal | Only `impl/memory/fact_store/`, `impl/session/store.rs`, `impl/session/journal.rs` use `rusqlite`. `core/**` uses none. `cognit/Cargo.toml:24` has a direct `rusqlite = "0.31"` dep but `cognit/src/core/planner.rs` uses no SQLite. | **MATCH** |
| 11 | Handler owns `fact_store` behind `Arc<Mutex<_>>` | `handler/mod.rs:139,640,667` | `:139` `fact_store: Arc<Mutex<FactStore>>` (in struct); `:640` start of `Self {` literal; `:667` `fact_store,` (27th field, passed by move since `Arc` is `Clone`) | **MATCH** |
| 12 | Store opened under `~/.aletheon/` in `new()` | `handler/mod.rs:195,219,223,225` | `:195` `pub async fn new(`; `:219-221` `aletheon_dir` construction; `:223` `FactStore::open(&aletheon_dir.join("fact_store.db"))`; `:225` `Arc::new(Mutex::new(fact_store))` | **MATCH** |
| 13 | JSON-RPC dispatch + shape | `handler/mod.rs:900`, `rpc.rs:18,24,95,61` | `mod.rs:900` `_ => self.handle_rpc(&method, id, request).await`; `rpc.rs:18` `pub(super) async fn handle_rpc(`; `:24` `match method {`; `:95` `"reflect"` arm returns `json!({"jsonrpc":"2.0","id":id,"result":{..}})`; `:61` `let fs = self.fact_store.lock().await` (inside `"clear"` arm) | **MATCH** |
| 14 | CLI subcommand + dispatch pattern | `cli.rs:17,70,155,168` | `:17` `pub const DEFAULT_SOCKET`; `:70` `pub enum Command {`; `:101-104` `Debug { action: debug::DebugCommand }` (not line 69); `:155` `async fn handle_command`; `:168` `Command::Debug { action } => debug::run(socket, action).await` | **MATCH** -- correction: plan said line 69 for Debug variant, actual Command enum starts at 70, Debug variant at 101-104 |
| 15 | `send_rpc` is private in `debug.rs` | `debug.rs:1194` | `:1194` `async fn send_rpc(socket, request) -> Result<Value>` (no `pub`) | **MATCH** -- confirmed private |
| 16 | Module registration idiom | `impl/mod.rs:10`, `impl/memory/mod.rs:8` | `impl/mod.rs:10` `pub mod memory;`; `impl/memory/mod.rs:8` `pub mod fact_store;` | **MATCH** |
| 17 | React loop `chat.rs` injections | `core/chat.rs:113-146` | **FILE DOES NOT EXIST.** `crates/runtime/src/core/chat.rs` not present on filesystem. The chat handling lives in `crates/runtime/src/impl/daemon/handler/chat.rs`. FactStore recall injection happens through the `compose_memory_block()` method at `handler/mod.rs:803` and the system prompt prefix builder at `handler/mod.rs:110`. There is no `core/chat.rs` to reference for the GoalTracker injection pattern. | **MISSING** -- plan's reference to `core/chat.rs` is a phantom file. The real injection happens in `handler/chat.rs` (daemon impl layer, not core). |
| 18 | `cognit` has its own direct `rusqlite` dep | Not in plan | `cognit/Cargo.toml:24` `rusqlite = { version = "0.31", features = ["bundled"] }` -- a non-workspace direct dependency, independent from `runtime`'s workspace `rusqlite`. This does not affect M-I (objective store goes in `runtime`) but is notable for Phase 6 decomposition reloc. | **NEW FINDING** (not in plan) |

### Drift summary

| Severity | Count | Details |
|---|---|---|
| MATCH | 15 | Claims 1-8, 10-16 verified exact |
| DRIFT (line offset 1-3) | 2 | Claims 8, 9: line offsets of 1-3; functionally correct |
| MISSING (file DNE) | 1 | Claim 17: `core/chat.rs` does not exist |
| NEW FINDING | 1 | Claim 18: `cognit` has its own `rusqlite` dep |

**Impact of MISSING claim 17:** The plan suggested using `core/chat.rs` as the pattern for GoalTracker injection. Since that file does not exist, the actual injection sites are in `handler/chat.rs` (for LLM context construction) and the `ReActLoop::compose_user_message` / `PrefixBuilder` paths. Phase 5 (resume-on-start) and Phase 7 (autonomous loop) need to use these real injection points instead.

---

## 2. Architecture Overview

### 2.1 Component Diagram

```
┌─────────────────────────────────────────────────────────────────────┐
│                          interact (CLI)                              │
│  cli.rs:Command::Goal  ──►  goal.rs:goal_cmd()  ──► send_rpc()     │
│  goal set/show/status/resume          Unix socket JSON-RPC          │
└───────────────────────────────────┬─────────────────────────────────┘
                                    │ JSON-RPC
┌───────────────────────────────────▼─────────────────────────────────┐
│                     runtime (Daemon)                                  │
│                                                                       │
│  handler/mod.rs                                                       │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ RequestHandler                                                │    │
│  │  objective_store: Arc<Mutex<ObjectiveStore>>  (NEW)          │    │
│  │  fact_store:      Arc<Mutex<FactStore>>        (existing)    │    │
│  │  resumed_objective: Option<(String, Vec<String>)>  (NEW)     │    │
│  │                                                               │    │
│  │  handle() ──► handle_rpc() ──► match method {                │    │
│  │    "goal.set"     ──► store.create()                          │    │
│  │    "goal.show"    ──► store.get() + sub_goals()               │    │
│  │    "goal.status"  ──► store.set_status() / list()            │    │
│  │    "goal.resume"  ──► store.resume()                          │    │
│  │    "goal.decompose" ──► Phase 6 (GATED)                       │    │
│  │  }                                                             │    │
│  └──────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  impl/goal/                                                           │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ ObjectiveStore  (SQLite, ~/.aletheon/objectives.db)          │    │
│  │  open() → PRAGMA WAL → create_schema()                        │    │
│  │  create(desc, parent, session, scope) → objective_id         │    │
│  │  get(id) → Option<ObjectiveRow>                               │    │
│  │  set_status(id, status) → bool                                 │    │
│  │  list(filter, limit) → Vec<ObjectiveRow>                      │    │
│  │  active() → Option<ObjectiveRow>  (top-level in_progress)    │    │
│  │  sub_goals(parent) → Vec<ObjectiveRow>                        │    │
│  │  resume() → Option<(ObjectiveRow, Vec<ObjectiveRow>)>        │    │
│  └──────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  impl/goal/tracker.rs  (Phase 5-7)                                    │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ GoalTrackerAdapter                                             │    │
│  │  Links persisted ObjectiveStore with in-memory GoalTracker    │    │
│  │  hydrate_from(&ObjectiveRow, &[ObjectiveRow]) → calls         │    │
│  │    tracker.set_goal() + tracker.add_sub_goal()                │    │
│  │  persistence_sync(&GoalTracker) → store.set_status()          │    │
│  └──────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  impl/goal/loop.rs  (Phase 7, GATED on Tier 2a)                      │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ GoalLoop                                                       │    │
│  │  pick_next_sub_goal(store)                                     │    │
│  │  advance(objective_id, permission_manager)                     │    │
│  │  mark_complete/fail(objective_id, status)                       │    │
│  │  Gating: config flag AND PermissionManager check               │    │
│  └──────────────────────────────────────────────────────────────┘    │
│                                                                       │
│  core/react_loop/goal_tracker.rs  (existing, Phase 5 seam)           │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ GoalTracker                                                    │    │
│  │  hydrate_from(desc, sub_goals)  (NEW Phase 5 seam)           │    │
│  │  set_goal(), add_sub_goal()     (existing)                    │    │
│  │  reset()                         (existing, spec_source kept) │    │
│  └──────────────────────────────────────────────────────────────┘    │
└───────────────────────────────────────────────────────────────────────┘
                                    │
┌───────────────────────────────────▼─────────────────────────────────┐
│                          base (ABI types)                             │
│  types/objective.rs  (NEW)                                           │
│  ┌──────────────────────────────────────────────────────────────┐    │
│  │ Objective, ObjectiveStatus, ObjectiveSummary                   │    │
│  │  (Serialize + Deserialize for JSON-RPC wire format)           │    │
│  └──────────────────────────────────────────────────────────────┘    │
└───────────────────────────────────────────────────────────────────────┘
```

### 2.2 Objective Lifecycle (Phases 1-5)

```
CREATE ──► DECOMPOSE ──► TRACK ──► COMPLETE/FAIL ──► NEXT (Phase 7)
  │            │            │             │               │
  │  CLI:      │  Phase 6   │  ReAct      │  CLI:         │  Phase 7
  │  goal set  │  (GATED)   │  loop uses  │  goal status  │  (GATED)
  │            │  goal      │  goal       │  --id N       │  GoalLoop
  │            │  decompose │  description│  --state      │  advances
  │            │            │  in context │  completed    │  to next
  │            ▼            ▼             ▼               ▼
  │    ┌──────────────────────────────────────────────────────┐
  └───►│              ObjectiveStore (SQLite)                  │
       │  objectives table: id, desc, status, parent, session, │
       │  scope, created_at, updated_at                        │
       └──────────────────────────────────────────────────────┘
                           │
                           │ resume()
                           ▼
              ┌────────────────────────┐
              │  Daemon startup        │
              │  reads active() →      │
              │  seeds GoalTracker     │
              │  via hydrate_from()    │
              └────────────────────────┘
```

### 2.3 Data Flow: CLI `goal create` through to ReAct context

```
aletheon goal set "ship goal layer" --scope project

    1. CLI (cli.rs:Command::Goal) parses into GoalAction::Set
    2. goal_cmd() in goal.rs builds JSON-RPC request:
       {"jsonrpc":"2.0","id":1,"method":"goal.set","params":{...}}
    3. goal_send_rpc() sends over Unix socket to daemon
    4. Daemon handle() → handle_rpc() dispatches "goal.set" arm
    5. objective_store.lock().await.create(desc, None, session_id, "project")
    6. SQLite INSERT → returns objective_id
    7. Response: {"jsonrpc":"2.0","id":1,"result":{"objective_id":42}}
    8. CLI prints result

    [Later, on next chat turn:]
    9. handler/chat.rs (the real chat handler, not phantom core/chat.rs)
       reads active objective from store OR uses cached resumed_objective
   10. ReActLoop.build_intent() or the handler's prompt construction
       appends goal context to the system/user message
   11. LLM receives: "Current objective: ship goal layer [id:42]"
```

### 2.4 Autonomous Loop Flow (Phase 7 design-only, GATED)

```
GoalLoop (enabled + permission granted):

    ┌──────────────────────────────────┐
    │ 1. Check if active objective     │
    │    has incomplete sub-goals      │
    └──────────────┬───────────────────┘
                   │ yes
                   ▼
    ┌──────────────────────────────────┐
    │ 2. PermissionManager.allow()     │
    │    (Tier 2a gate)                │
    └──────────────┬───────────────────┘
                   │ granted
                   ▼
    ┌──────────────────────────────────┐
    │ 3. Select next incomplete        │
    │    sub-goal (oldest first)       │
    └──────────────┬───────────────────┘
                   │
                   ▼
    ┌──────────────────────────────────┐
    │ 4. Inject sub-goal as current    │
    │    goal into next ReAct turn     │
    └──────────────┬───────────────────┘
                   │
                   ▼
    ┌──────────────────────────────────┐
    │ 5. ReAct loop runs turn          │
    └──────────────┬───────────────────┘
                   │
                   ▼
    ┌──────────────────────────────────┐
    │ 6. Post-turn: GoalLoop marks     │
    │    sub-goal completed/failed     │
    │    via set_status()              │
    └──────────────┬───────────────────┘
                   │ all sub-goals resolved
                   ▼
    ┌──────────────────────────────────┐
    │ 7. Mark parent objective         │
    │    completed                     │
    │    → loop ends or user sets      │
    │      next objective              │
    └──────────────────────────────────┘
```

---

## 3. Database Schema

### SQL DDL (executed in `ObjectiveStore::create_schema`)

```sql
CREATE TABLE IF NOT EXISTS objectives (
    objective_id INTEGER PRIMARY KEY AUTOINCREMENT,
    description  TEXT NOT NULL,
    status       TEXT NOT NULL DEFAULT 'in_progress'
                 CHECK(status IN ('in_progress','completed','failed','adjusted')),
    parent_id    INTEGER REFERENCES objectives(objective_id) ON DELETE CASCADE,
    session_id   TEXT NOT NULL DEFAULT '',
    scope        TEXT NOT NULL DEFAULT 'session'
                 CHECK(scope IN ('session','project','global')),
    created_at   TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE INDEX IF NOT EXISTS idx_objectives_status ON objectives(status);
CREATE INDEX IF NOT EXISTS idx_objectives_parent ON objectives(parent_id);
CREATE INDEX IF NOT EXISTS idx_objectives_session ON objectives(session_id);
CREATE INDEX IF NOT EXISTS idx_objectives_scope ON objectives(scope);
```

### Column reference

| Column | Type | Default | Constraint | Purpose |
|---|---|---|---|---|
| `objective_id` | INTEGER | AUTOINCREMENT | PK | Unique ID, used as parent FK |
| `description` | TEXT | -- | NOT NULL | Human-readable goal description |
| `status` | TEXT | `in_progress` | CHECK 4 values | Mirrors `GoalStatus` enum variants |
| `parent_id` | INTEGER | NULL | FK → objectives(id) | NULL = top-level; else = sub-goal |
| `session_id` | TEXT | `''` | -- | Session that created/owns this goal |
| `scope` | TEXT | `session` | CHECK 3 values | Lifecycle scope: session/project/global |
| `created_at` | TEXT | `datetime('now')` | ISO-8601 | Creation timestamp |
| `updated_at` | TEXT | `datetime('now')` | ISO-8601 | Last mutation timestamp |

### Index rationale

- `idx_objectives_status`: hot path for `active()` (WHERE status='in_progress' AND parent_id IS NULL)
- `idx_objectives_parent`: hot path for `sub_goals(parent)` (WHERE parent_id = ?)
- `idx_objectives_session`: filter by session for multi-session listing
- `idx_objectives_scope`: filter by scope for project-scoped objectives

### Migration strategy (MVP)

No migration framework in MVP. `CREATE TABLE IF NOT EXISTS` is idempotent; adding columns later uses `ALTER TABLE ... ADD COLUMN` with a schema version check. Document this explicitly: Phase 1 schema is append-only via ALTER; never drop/rename columns without a migration numbered file in `impl/goal/migrations/`.

---

## 4. Phase 1 -- Objective Types in `base`

### File: `crates/base/src/types/objective.rs` (NEW)

```rust
//! Persistent objective types shared across the workspace.
//!
//! These types form the ABI for the goal layer. `ObjectiveStatus` mirrors
//! `runtime::core::react_loop::goal_tracker::GoalStatus` and the SQLite CHECK
//! constraint on `objectives.status`.

use serde::{Deserialize, Serialize};

/// The status of an objective, matching GoalStatus + DB constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveStatus {
    InProgress,
    Completed,
    Failed,
    Adjusted,
}

impl ObjectiveStatus {
    /// String representation for JSON-RPC responses and DB storage.
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectiveStatus::InProgress => "in_progress",
            ObjectiveStatus::Completed => "completed",
            ObjectiveStatus::Failed => "failed",
            ObjectiveStatus::Adjusted => "adjusted",
        }
    }

    /// Parse from a DB status string.
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "in_progress" => Some(ObjectiveStatus::InProgress),
            "completed" => Some(ObjectiveStatus::Completed),
            "failed" => Some(ObjectiveStatus::Failed),
            "adjusted" => Some(ObjectiveStatus::Adjusted),
            _ => None,
        }
    }
}

impl std::fmt::Display for ObjectiveStatus {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.as_str())
    }
}

/// A persisted objective (top-level goal or sub-goal via parent_id).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Objective {
    pub objective_id: i64,
    pub description: String,
    pub status: ObjectiveStatus,
    pub parent_id: Option<i64>,
    pub session_id: String,
    pub scope: String,
    pub created_at: String,
    pub updated_at: String,
}

/// Lightweight summary for list views (no parent/session/scope noise).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ObjectiveSummary {
    pub objective_id: i64,
    pub description: String,
    pub status: ObjectiveStatus,
}

impl Objective {
    /// Convert to a summary suitable for list display.
    pub fn to_summary(&self) -> ObjectiveSummary {
        ObjectiveSummary {
            objective_id: self.objective_id,
            description: self.description.clone(),
            status: self.status,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn status_serde_roundtrip() {
        let statuses = vec![
            ObjectiveStatus::InProgress,
            ObjectiveStatus::Completed,
            ObjectiveStatus::Failed,
            ObjectiveStatus::Adjusted,
        ];
        for s in statuses {
            let json = serde_json::to_string(&s).unwrap();
            let back: ObjectiveStatus = serde_json::from_str(&json).unwrap();
            assert_eq!(s, back);
        }
    }

    #[test]
    fn status_str_roundtrip() {
        assert_eq!(
            ObjectiveStatus::from_str("in_progress"),
            Some(ObjectiveStatus::InProgress)
        );
        assert_eq!(
            ObjectiveStatus::from_str("completed"),
            Some(ObjectiveStatus::Completed)
        );
        assert_eq!(ObjectiveStatus::from_str("bogus"), None);
    }

    #[test]
    fn objective_json_roundtrip() {
        let obj = Objective {
            objective_id: 1,
            description: "ship goal layer".into(),
            status: ObjectiveStatus::InProgress,
            parent_id: None,
            session_id: "sess-1".into(),
            scope: "project".into(),
            created_at: "2026-07-02T00:00:00".into(),
            updated_at: "2026-07-02T00:00:00".into(),
        };
        let json = serde_json::to_string_pretty(&obj).unwrap();
        let back: Objective = serde_json::from_str(&json).unwrap();
        assert_eq!(back.objective_id, 1);
        assert_eq!(back.status, ObjectiveStatus::InProgress);
    }

    #[test]
    fn summary_conversion() {
        let obj = Objective {
            objective_id: 42,
            description: "test".into(),
            status: ObjectiveStatus::Completed,
            parent_id: Some(1),
            session_id: "s".into(),
            scope: "session".into(),
            created_at: String::new(),
            updated_at: String::new(),
        };
        let summary = obj.to_summary();
        assert_eq!(summary.objective_id, 42);
        assert_eq!(summary.description, "test");
        assert_eq!(summary.status, ObjectiveStatus::Completed);
    }
}
```

### File: `crates/base/src/types/mod.rs` -- Insertion

**Insert at line 11** (after `pub mod message;`, before `pub mod paths;` -- alpha order: `objective` < `paths`):

```rust
pub mod objective;
```

Full updated module list (lines 1-16):
```rust
pub mod agent;
pub mod capability;
pub mod context;
pub mod genome;
pub mod hook;
pub mod hook_ext;
pub mod llm_types;
pub mod message;
pub mod objective;      // <-- NEW, after message (alpha)
pub mod paths;
pub mod permission;
pub mod resource;
pub mod sandbox;
pub mod tool;
```

### File: `crates/base/src/lib.rs` -- Insertions

**Insert at line 51** (after `pub use types::message;`, in the flat re-export block):

```rust
pub use types::objective;
```

**Insert at line 121** (after the deep type re-exports block, before the last one):

```rust
pub use types::objective::{Objective, ObjectiveStatus, ObjectiveSummary};
```

### Build check

```bash
cargo build -p base
cargo test -p base objective
```

---

## 5. Phase 2 -- ObjectiveStore (Schema + CRUD + Query API)

### File: `crates/runtime/src/impl/goal/mod.rs` (NEW)

This file follows the exact pattern of `crates/runtime/src/impl/memory/fact_store/mod.rs` (lines 91-265): struct with `pub(crate) db`, `open(path)` constructor, WAL pragma, `create_schema` helper, positional `map_*_row`, and `#[cfg(test)]` module.

```rust
//! Persistent objective store backed by SQLite.
//!
//! Mirrors `FactStore`'s open/schema idiom (`impl/memory/fact_store/mod.rs`):
//! WAL, `CREATE TABLE IF NOT EXISTS`, positional `map_objective_row`.

mod store;

use anyhow::{Context, Result};
use base::objective::{Objective, ObjectiveStatus};
use rusqlite::Connection;

impl ObjectiveStore {
    /// Open (or create) an objective store at the given path.
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let db = Connection::open(path).context("opening objective store DB")?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::create_schema(&db)?;
        Ok(Self { db })
    }

    fn create_schema(db: &Connection) -> Result<()> {
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS objectives (
                objective_id INTEGER PRIMARY KEY AUTOINCREMENT,
                description  TEXT NOT NULL,
                status       TEXT NOT NULL DEFAULT 'in_progress'
                             CHECK(status IN ('in_progress','completed','failed','adjusted')),
                parent_id    INTEGER REFERENCES objectives(objective_id) ON DELETE CASCADE,
                session_id   TEXT NOT NULL DEFAULT '',
                scope        TEXT NOT NULL DEFAULT 'session'
                             CHECK(scope IN ('session','project','global')),
                created_at   TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_objectives_status ON objectives(status);
            CREATE INDEX IF NOT EXISTS idx_objectives_parent ON objectives(parent_id);
            CREATE INDEX IF NOT EXISTS idx_objectives_session ON objectives(session_id);
            CREATE INDEX IF NOT EXISTS idx_objectives_scope ON objectives(scope);",
        )?;
        Ok(())
    }

    /// Map a rusqlite Row to an Objective using positional indices.
    ///
    /// Column order MUST match the `COLS` constant in `store.rs`.
    /// Indices: 0=objective_id, 1=description, 2=status, 3=parent_id,
    ///          4=session_id, 5=scope, 6=created_at, 7=updated_at
    pub(crate) fn map_objective_row(row: &rusqlite::Row) -> rusqlite::Result<Objective> {
        let status_str: String = row.get(2)?;
        let status = ObjectiveStatus::from_str(&status_str)
            .unwrap_or(ObjectiveStatus::InProgress);
        Ok(Objective {
            objective_id: row.get(0)?,
            description: row.get(1)?,
            status,
            parent_id: row.get(3)?,
            session_id: row.get(4)?,
            scope: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }
}

/// SQLite-backed objective store.
///
/// Held behind `Arc<Mutex<ObjectiveStore>>` in `RequestHandler`, mirroring
/// `fact_store`'s ownership pattern exactly.
pub struct ObjectiveStore {
    pub(crate) db: Connection,
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::NamedTempFile;

    fn setup() -> (ObjectiveStore, NamedTempFile) {
        let tmp = NamedTempFile::new().unwrap();
        let store = ObjectiveStore::open(tmp.path()).unwrap();
        (store, tmp)
    }

    #[test]
    fn create_and_get_roundtrip() {
        let (store, _tmp) = setup();
        let id = store
            .create("Ship the goal layer", None, "sess-1", "project")
            .unwrap();
        assert!(id > 0);
        let row = store.get(id).unwrap().unwrap();
        assert_eq!(row.description, "Ship the goal layer");
        assert_eq!(row.status, ObjectiveStatus::InProgress);
        assert_eq!(row.session_id, "sess-1");
        assert_eq!(row.scope, "project");
        assert!(row.parent_id.is_none());
    }

    #[test]
    fn schema_and_indexes_exist() {
        let (store, _tmp) = setup();
        let names: Vec<String> = store
            .db
            .prepare(
                "SELECT name FROM sqlite_master WHERE type IN ('table','index') ORDER BY name",
            )
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<std::result::Result<_, _>>()
            .unwrap();
        assert!(names.iter().any(|n| n == "objectives"));
        assert!(names.iter().any(|n| n == "idx_objectives_status"));
        assert!(names.iter().any(|n| n == "idx_objectives_parent"));
        assert!(names.iter().any(|n| n == "idx_objectives_session"));
    }

    #[test]
    fn active_returns_latest_in_progress_top_level() {
        let (store, _tmp) = setup();
        let a = store
            .create("first objective", None, "s", "session")
            .unwrap();
        let b = store
            .create("second objective", None, "s", "session")
            .unwrap();
        // sub-goals of `b`
        store
            .create("sub one", Some(b), "s", "session")
            .unwrap();
        store
            .create("sub two", Some(b), "s", "session")
            .unwrap();
        // finishing `b` makes `a` the active top-level objective
        assert!(store.set_status(b, "completed").unwrap());
        let active = store.active().unwrap().unwrap();
        assert_eq!(active.objective_id, a);
        // sub_goals only returns children of the given parent
        let subs = store.sub_goals(b).unwrap();
        assert_eq!(subs.len(), 2);
        assert!(subs.iter().all(|s| s.parent_id == Some(b)));
    }

    #[test]
    fn resume_reconstructs_active_objective_and_subs() {
        let (store, _tmp) = setup();
        let obj = store
            .create("resume me", None, "s", "project")
            .unwrap();
        store
            .create("child a", Some(obj), "s", "project")
            .unwrap();
        let (active, subs) = store.resume().unwrap().unwrap();
        assert_eq!(active.objective_id, obj);
        assert_eq!(subs.len(), 1);
        assert_eq!(
            store.list(None, 50).unwrap().len(),
            2
        );
        assert_eq!(
            store.list(Some("in_progress"), 50).unwrap().len(),
            2
        );
    }

    #[test]
    fn status_filtering() {
        let (store, _tmp) = setup();
        let id = store
            .create("complete me", None, "s", "session")
            .unwrap();
        store.set_status(id, "completed").unwrap();
        let in_progress = store.list(Some("in_progress"), 50).unwrap();
        let completed = store.list(Some("completed"), 50).unwrap();
        assert_eq!(in_progress.len(), 0);
        assert_eq!(completed.len(), 1);
        assert_eq!(completed[0].objective_id, id);
    }

    #[test]
    fn parent_cascade_deletes_sub_goals() {
        let (store, _tmp) = setup();
        let parent = store
            .create("parent", None, "s", "session")
            .unwrap();
        let child = store
            .create("child", Some(parent), "s", "session")
            .unwrap();
        // Delete parent (using raw SQL since we don't expose delete in MVP API)
        store
            .db
            .execute(
                "DELETE FROM objectives WHERE objective_id = ?1",
                rusqlite::params![parent],
            )
            .unwrap();
        // Sub-goal should be cascade-deleted
        assert!(store.get(child).unwrap().is_none());
    }
}
```

### File: `crates/runtime/src/impl/goal/store.rs` (NEW)

```rust
//! CRUD and query operations for ObjectiveStore.
//!
//! Mirror of `fact_store/query.rs`. Every SELECT uses the shared `COLS` constant
//! to guarantee column order matches `map_objective_row`.

use super::{Objective, ObjectiveStore};
use anyhow::Result;

/// Fixed column order — every SELECT feeding `map_objective_row` MUST use this.
/// Indices: 0=objective_id, 1=description, 2=status, 3=parent_id,
///          4=session_id, 5=scope, 6=created_at, 7=updated_at
pub(crate) const COLS: &str =
    "objective_id, description, status, parent_id, session_id, scope, created_at, updated_at";

impl ObjectiveStore {
    /// Insert a top-level objective or sub-goal. Returns the new `objective_id`.
    pub fn create(
        &self,
        description: &str,
        parent: Option<i64>,
        session_id: &str,
        scope: &str,
    ) -> Result<i64> {
        self.db.execute(
            "INSERT INTO objectives (description, parent_id, session_id, scope)
             VALUES (?1, ?2, ?3, ?4)",
            rusqlite::params![description, parent, session_id, scope],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// Fetch one objective by id.
    pub fn get(&self, id: i64) -> Result<Option<Objective>> {
        let sql = format!("SELECT {COLS} FROM objectives WHERE objective_id = ?1");
        let mut stmt = self.db.prepare(&sql)?;
        let mut rows = stmt.query_map(rusqlite::params![id], Self::map_objective_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Update status; bumps `updated_at`. Returns true if a row changed.
    pub fn set_status(&self, id: i64, status: &str) -> Result<bool> {
        let changed = self.db.execute(
            "UPDATE objectives SET status = ?1, updated_at = datetime('now')
             WHERE objective_id = ?2",
            rusqlite::params![status, id],
        )?;
        Ok(changed > 0)
    }

    /// List objectives, optionally filtered by status, newest first.
    pub fn list(&self, status_filter: Option<&str>, limit: usize) -> Result<Vec<Objective>> {
        let mut sql = format!("SELECT {COLS} FROM objectives");
        if status_filter.is_some() {
            sql.push_str(" WHERE status = ?1");
        }
        sql.push_str(&format!(" ORDER BY objective_id DESC LIMIT {}", limit as i64));
        let mut stmt = self.db.prepare(&sql)?;
        let rows = if let Some(s) = status_filter {
            stmt.query_map(rusqlite::params![s], Self::map_objective_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([], Self::map_objective_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }

    /// The single active top-level objective: newest `in_progress` with no parent.
    /// (MVP is single-objective; see non-goals.)
    pub fn active(&self) -> Result<Option<Objective>> {
        let sql = format!(
            "SELECT {COLS} FROM objectives
             WHERE status = 'in_progress' AND parent_id IS NULL
             ORDER BY objective_id DESC LIMIT 1"
        );
        let mut stmt = self.db.prepare(&sql)?;
        let mut rows = stmt.query_map([], Self::map_objective_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }

    /// Direct children of an objective, oldest first (milestone order).
    pub fn sub_goals(&self, parent: i64) -> Result<Vec<Objective>> {
        let sql = format!(
            "SELECT {COLS} FROM objectives WHERE parent_id = ?1 ORDER BY objective_id ASC"
        );
        let mut stmt = self.db.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![parent], Self::map_objective_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Resume: the active top-level objective plus its sub-goals, if any.
    /// Returns `None` when no active objective exists (fresh start).
    pub fn resume(&self) -> Result<Option<(Objective, Vec<Objective>)>> {
        match self.active()? {
            Some(obj) => {
                let subs = self.sub_goals(obj.objective_id)?;
                Ok(Some((obj, subs)))
            }
            None => Ok(None),
        }
    }

    /// Count objectives matching a status filter (for tests and health checks).
    #[cfg(test)]
    pub(crate) fn count_by_status(&self, status: &str) -> Result<usize> {
        let count: i64 = self.db.query_row(
            "SELECT COUNT(*) FROM objectives WHERE status = ?1",
            rusqlite::params![status],
            |r| r.get(0),
        )?;
        Ok(count as usize)
    }
}
```

### File: `crates/runtime/src/impl/mod.rs` -- Insertion

**Insert at line 8** (after `pub mod engine;`, before `pub mod hooks;` -- alpha: `goal` < `hooks`):

```rust
pub mod goal;       // <-- NEW, after engine, before hooks
```

Updated module list (lines 1-16):
```rust
pub mod agent;
pub mod agent_loader;
pub mod agents;
pub mod automation;
pub mod coordinator;
pub mod daemon;
pub mod engine;
pub mod goal;          // <-- NEW
pub mod hooks;
pub mod kernel;
pub mod memory;
pub mod orchestration;
pub mod plugin;
pub mod session;
pub mod skill_router;
pub mod skills;
```

### Build and test

```bash
cargo build -p runtime 2>&1          # Expected: compiles
cargo test -p runtime goal           # Expected: 6 tests PASS
```

---

## 6. Phase 3 -- Daemon: Own the Store + `goal.*` JSON-RPC

### File: `crates/runtime/src/impl/daemon/handler/mod.rs` -- Insertions

**Insertion 1 -- Import** at line 63 (after `use crate::r#impl::memory::fact_store::FactStore;`):

```rust
use crate::r#impl::goal::ObjectiveStore;   // <-- NEW
```

**Insertion 2 -- Struct field** at line 140 (after `fact_store: Arc<Mutex<FactStore>>` at line 139):

```rust
    /// SQLite-backed objective store for persistent goal tracking.
    objective_store: Arc<Mutex<ObjectiveStore>>,
```

**Insertion 3 -- Open store** at line 226 (after the `Arc::new(Mutex::new(fact_store))` block ends):

```rust
        // ObjectiveStore — persisted goals with resume-on-start support
        let objective_store = ObjectiveStore::open(&aletheon_dir.join("objectives.db"))
            .context("opening objective store")?;
        let objective_store = Arc::new(Mutex::new(objective_store));
```

**Insertion 4 -- Struct init** at line 668 (after `fact_store,` at line 667):

```rust
            objective_store,
```

Full context for Insertion 4 (lines 667-669):
```rust
            fact_store,                     // line 667
            objective_store,                // line 668 NEW
            storm_breaker,                  // line 669 (current)
```

### File: `crates/runtime/src/impl/daemon/handler/rpc.rs` -- Insertions

**Insert -- New JSON-RPC arms.** Insert after the `"status"` arm (ending around line 125) and before the next existing arm. Add these inside the `match method {` block at line 24:

```rust
            "goal.set" => {
                let p = &request["params"];
                let description = p["description"].as_str().unwrap_or("");
                let scope = p["scope"].as_str().unwrap_or("session");
                let session_id = self.session_manager.lock().await.session_id.clone();
                let store = self.objective_store.lock().await;
                match store.create(description, None, &session_id, scope) {
                    Ok(id) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "objective_id": id }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32020, "message": e.to_string() }
                    }),
                }
            }
            "goal.show" => {
                let oid = request["params"]["id"].as_i64().unwrap_or(0);
                let store = self.objective_store.lock().await;
                match store.get(oid) {
                    Ok(Some(obj)) => {
                        let subs = store.sub_goals(oid).unwrap_or_default();
                        let summaries: Vec<_> = subs.iter().map(|s| s.to_summary()).collect();
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "objective": obj, "sub_goals": summaries }
                        })
                    }
                    Ok(None) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32021, "message": "objective not found" }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32020, "message": e.to_string() }
                    }),
                }
            }
            "goal.status" => {
                let p = &request["params"];
                let store = self.objective_store.lock().await;
                // With an id: update status. Without: list objectives.
                if let Some(oid) = p["id"].as_i64() {
                    let new_status = p["status"].as_str().unwrap_or("in_progress");
                    match store.set_status(oid, new_status) {
                        Ok(changed) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "ok": changed }
                        }),
                        Err(e) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32020, "message": e.to_string() }
                        }),
                    }
                } else {
                    let filter = p["filter"].as_str();
                    match store.list(filter, 50) {
                        Ok(rows) => {
                            let summaries: Vec<_> = rows.iter().map(|r| r.to_summary()).collect();
                            json!({
                                "jsonrpc": "2.0",
                                "id": id,
                                "result": { "objectives": summaries }
                            })
                        }
                        Err(e) => json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "error": { "code": -32020, "message": e.to_string() }
                        }),
                    }
                }
            }
            "goal.resume" => {
                let store = self.objective_store.lock().await;
                match store.resume() {
                    Ok(Some((obj, subs))) => {
                        let summaries: Vec<_> = subs.iter().map(|s| s.to_summary()).collect();
                        json!({
                            "jsonrpc": "2.0",
                            "id": id,
                            "result": { "objective": obj, "sub_goals": summaries }
                        })
                    }
                    Ok(None) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "objective": null, "sub_goals": [] }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32020, "message": e.to_string() }
                    }),
                }
            }
```

### Key implementation notes

1. **Lock ordering:** `objective_store.lock()` is called independently (not nested inside `fact_store` lock). The `"clear"` arm locks both `fact_store` (line 61) and `session_manager` (line 62) independently via separate `let` bindings -- the same pattern works for `objective_store`.

2. **Never hold a `store` lock across `.await`.** All current arms lock, perform synchronous DB work, and drop the guard before any async operation.

3. **The `id` binding** is the `handle_rpc` parameter at `rpc.rs:21` -- reused identically to the `"clear"` and `"reflect"` arms.

4. **`Objective` derives `Serialize`** (via `base::objective`), so `json!({"objective": obj})` works directly.

### Build check

```bash
cargo build -p runtime 2>&1          # Expected: compiles
```

If the `objective_store` field is unused before Task 4 arms land, add `#[allow(dead_code)]` temporarily above the field (line 140), then remove after Task 4. Alternatively, land Phase 3 + Phase 4 (RPC arms) in a single commit.

### Manual smoke test

```bash
# With daemon running:
echo '{"jsonrpc":"2.0","id":1,"method":"goal.set","params":{"description":"ship goal layer","scope":"project"}}' | nc -U /run/aletheond/aletheond.sock
# Expected: {"jsonrpc":"2.0","id":1,"result":{"objective_id":1}}

echo '{"jsonrpc":"2.0","id":2,"method":"goal.show","params":{"id":1}}' | nc -U /run/aletheond/aletheond.sock
# Expected: {"jsonrpc":"2.0","id":2,"result":{"objective":{...},"sub_goals":[]}}

echo '{"jsonrpc":"2.0","id":3,"method":"goal.resume","params":{}}' | nc -U /run/aletheond/aletheond.sock
# Expected: {"jsonrpc":"2.0","id":3,"result":{"objective":{...},"sub_goals":[]}}
```

---

## 7. Phase 4 -- `interact` CLI `goal` Subcommand

### File: `crates/interact/src/tui/cli.rs` -- Insertions

**Insertion 1 -- Import** at line 6 (after `use super::debug;`):

```rust
mod goal;
```

**Insertion 2 -- GoalAction enum** at line 105 (after the closing `}` of `enum DaemonAction` at line 121):

```rust
#[derive(Subcommand)]
pub enum GoalAction {
    /// Set the active objective
    Set {
        description: String,
        #[arg(long, default_value = "session")]
        scope: String,
    },
    /// Show one objective (with its sub-goals) by id
    Show {
        id: i64,
    },
    /// List objectives, or update one
    Status {
        #[arg(long)]
        id: Option<i64>,
        #[arg(long)]
        state: Option<String>,
        #[arg(long)]
        filter: Option<String>,
    },
    /// Resume the active objective
    Resume,
}
```

**Insertion 3 -- Command variant** at line 105 (after the `Debug` variant):

```rust
    /// Persistent goal / objective management
    Goal {
        #[command(subcommand)]
        action: GoalAction,
    },
```

The updated `Command` enum (lines 69-105+):
```rust
pub enum Command {
    Daemon { action: DaemonAction },
    Reflect,
    ReflectNow,
    Evolution,
    Genome,
    Status,
    RestoreTerminal,
    Debug { action: debug::DebugCommand },
    Goal { action: GoalAction },       // <-- NEW
}
```

**Insertion 4 -- Dispatch arm** at line 169 (after `Command::Debug { action } =>`):

```rust
        Command::Goal { action } => goal::run(socket, action).await,
```

The updated `handle_command` (lines 155-170+):
```rust
async fn handle_command(socket: &PathBuf, cmd: Command) -> Result<()> {
    match cmd {
        Command::Daemon { action } => handle_daemon_action(action).await,
        Command::Reflect => single_message(socket, "/reflect").await,
        Command::ReflectNow => single_message(socket, "/reflect_now").await,
        Command::Evolution => single_message(socket, "/evolution").await,
        Command::Genome => single_message(socket, "/genome").await,
        Command::Status => single_message(socket, "/status").await,
        Command::RestoreTerminal => { /*...*/ Ok(()) }
        Command::Debug { action } => debug::run(socket, action).await,
        Command::Goal { action } => goal::run(socket, action).await,  // <-- NEW
    }
}
```

### File: `crates/interact/src/tui/goal.rs` (NEW)

Mirrors `debug.rs`'s pattern: a `run()` entry point, a private `send_rpc()` helper (copy of `debug.rs:1194`), and per-action dispatch.

```rust
//! CLI handlers for the `aletheon goal` subcommand.
//!
//! Each action sends a JSON-RPC request over the daemon Unix socket
//! and prints the result. Mirrors `debug.rs`'s `send_rpc` + `run` pattern.

use std::io;
use std::path::PathBuf;

use anyhow::{Context, Result};
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::UnixStream;

use super::cli::GoalAction;

/// Entry point dispatched from `handle_command`.
pub async fn run(socket: &PathBuf, action: GoalAction) -> Result<()> {
    let req = match &action {
        GoalAction::Set {
            description,
            scope,
        } => serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "goal.set",
            "params": { "description": description, "scope": scope }
        }),
        GoalAction::Show { id } => serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "goal.show",
            "params": { "id": id }
        }),
        GoalAction::Status { id, state, filter } => serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "goal.status",
            "params": { "id": id, "status": state, "filter": filter }
        }),
        GoalAction::Resume => serde_json::json!({
            "jsonrpc": "2.0",
            "id": 1,
            "method": "goal.resume",
            "params": {}
        }),
    };

    let resp = send_rpc(socket, &req).await?;

    // Pretty-print the result
    if let Some(objs) = resp["result"]["objectives"].as_array() {
        for o in objs {
            println!(
                "[{}] ({}) {}",
                o["objective_id"],
                o["status"].as_str().unwrap_or("?"),
                o["description"].as_str().unwrap_or("")
            );
        }
    } else if let Some(obj) = resp["result"]["objective"].as_object() {
        println!(
            "[{}] ({}) {}",
            obj.get("objective_id")
                .and_then(|v| v.as_i64())
                .unwrap_or(0),
            obj.get("status")
                .and_then(|v| v.as_str())
                .unwrap_or("?"),
            obj.get("description")
                .and_then(|v| v.as_str())
                .unwrap_or("")
        );
        if let Some(subs) = resp["result"]["sub_goals"].as_array() {
            if !subs.is_empty() {
                println!("Sub-goals:");
                for s in subs {
                    println!(
                        "  [{}] ({}) {}",
                        s["objective_id"],
                        s["status"].as_str().unwrap_or("?"),
                        s["description"].as_str().unwrap_or("")
                    );
                }
            }
        }
    } else if resp["result"]["objective"].is_null() {
        println!("No active objective.");
    } else if let Some(id) = resp["result"]["objective_id"].as_i64() {
        println!("Objective created: id={}", id);
    } else {
        println!(
            "{}",
            serde_json::to_string_pretty(&resp["result"]).unwrap_or_default()
        );
    }

    if let Some(err) = resp["error"].as_object() {
        eprintln!(
            "Error: {}",
            err.get("message")
                .and_then(|v| v.as_str())
                .unwrap_or("unknown error")
        );
    }

    Ok(())
}

/// Local copy of `debug.rs:1194`'s `send_rpc` — that function is private.
/// TODO: if `debug::send_rpc` is made `pub(crate)`, collapse this duplicate.
async fn send_rpc(
    socket: &std::path::Path,
    request: &serde_json::Value,
) -> Result<serde_json::Value> {
    let mut stream = UnixStream::connect(socket)
        .await
        .with_context(|| format!("Cannot connect to daemon socket: {}", socket.display()))?;

    let req_str = serde_json::to_string(request)?;
    stream.write_all(req_str.as_bytes()).await?;
    stream.write_all(b"\n").await?;

    let (reader, _) = stream.split();
    let mut reader = BufReader::new(reader);
    let mut response = String::new();
    reader.read_line(&mut response).await?;

    let resp: serde_json::Value = serde_json::from_str(&response)
        .context("Failed to parse daemon response")?;

    Ok(resp)
}
```

### Build and test

```bash
cargo build -p interact 2>&1          # Expected: compiles

# End-to-end (daemon running):
aletheon goal set "ship the goal layer" --scope project
# → Objective created: id=1

aletheon goal status
# → [1] (in_progress) ship the goal layer

aletheon goal show 1
# → [1] (in_progress) ship the goal layer
#    Sub-goals: (none)

aletheon goal status --id 1 --state completed
# → [1] (completed) ship the goal layer

aletheon goal resume
# → No active objective.
```

---

## 8. Phase 5 -- Resume the Active Objective into `GoalTracker` on Start

This is the most architecturally sensitive phase: the `GoalTracker` lives inside `ReActLoop` (line 148 of `react_loop/mod.rs`), which is constructed deep in `AletheonRuntime::new()` (line 43 of `orchestrator.rs`). The handler cannot directly seed the tracker -- it must thread a seed through the construction chain OR seed post-construction.

### Architecture decision: Handler-field seed pattern

The handler stores a `resumed_objective: Option<(String, Vec<String>)>` field. On daemon startup, it reads `ObjectiveStore::resume()` and caches the description + sub-goal strings. When `ReActLoop` is constructed (inside `AletheonRuntime::new()` called from `handler/mod.rs`), the seed is passed through and applied once before the first turn.

**Alternative considered (rejected):** Threading a callback or post-construction seed call. This would require making `GoalTracker` accessible from outside `ReActLoop`, breaking encapsulation. The field-seed pattern is simpler and keeps the seam on `GoalTracker` itself.

### File: `crates/runtime/src/core/react_loop/goal_tracker.rs` -- Insertion

**Insert at line 262** (after the `reset()` method, before the `#[cfg(test)]` block at line 265):

```rust
    /// Seed the tracker from a persisted objective.
    ///
    /// Used to resume a cross-session objective on daemon start.
    /// Call exactly once, before the first turn. `reset()` semantics
    /// (clearing goal/sub-goals/criteria/constraints, preserving spec_source)
    /// are unchanged for subsequent turns.
    pub fn hydrate_from(&mut self, description: &str, sub_goals: &[String]) {
        self.set_goal(description.to_string());
        for sg in sub_goals {
            self.add_sub_goal(sg.clone());
        }
    }
```

### File: `crates/runtime/src/core/react_loop/goal_tracker.rs` -- Test insertion

**Insert at line 437** (after the last test, before closing `}`):

```rust
    #[test]
    fn hydrate_from_persisted_objective() {
        let mut tracker = GoalTracker::new();
        tracker.hydrate_from(
            "ship goal layer",
            &["persist store".to_string(), "wire rpc".to_string()],
        );
        assert_eq!(
            tracker.current_goal_description(),
            Some("ship goal layer".into())
        );
        let ctx = tracker.get_context();
        assert!(ctx.contains("persist store"));
        assert!(ctx.contains("wire rpc"));

        // reset clears the hydrated goal
        tracker.reset();
        assert!(tracker.current_goal_description().is_none());
    }
```

### File: `crates/runtime/src/impl/daemon/handler/mod.rs` -- Insertions

**Insertion A -- Struct field** at line 141 (after `objective_store` field):

```rust
    /// Cached active objective + sub-goals for resume-on-start.
    /// Applied once to GoalTracker before the first chat turn.
    resumed_objective: Option<(String, Vec<String>)>,
```

**Insertion B -- Read and cache** at line 228 (after objective_store construction block ends):

```rust
        // Resume active objective for session continuity
        let resumed_objective = {
            let store = objective_store.lock().await;
            match store.resume() {
                Ok(Some((obj, subs))) => {
                    let sub_desc: Vec<String> =
                        subs.iter().map(|s| s.description.clone()).collect();
                    info!(
                        objective_id = obj.objective_id,
                        description = %obj.description,
                        sub_goals = sub_desc.len(),
                        "Resuming persisted objective on start"
                    );
                    Some((obj.description.clone(), sub_desc))
                }
                Ok(None) => {
                    info!("No active objective to resume — fresh start");
                    None
                }
                Err(e) => {
                    warn!(error = %e, "Failed to read active objective on start");
                    None
                }
            }
        };
```

**Insertion C -- Struct init** at line 668 (after `objective_store,`):

```rust
            resumed_objective,             // <-- NEW
```

### Wiring the seed into ReActLoop

The handler constructs `ReActLoop` via `AletheonRuntime::new()`. The seed needs to reach `GoalTracker` at the right moment -- after construction, before the first turn's `reset()`.

**OPTION A (Recommended): Add a `seed_goal` method to `ReActLoop`** that calls `goal_tracker.hydrate_from()` exactly once. The handler calls it after construction but before the first chat turn.

```rust
// react_loop/mod.rs -- new method on ReActLoop
/// Seed the goal tracker from persisted state (resume-on-start).
/// Must be called before the first turn; subsequent turns' reset() is unaffected.
pub fn seed_goal(&mut self, description: &str, sub_goals: &[String]) {
    self.goal_tracker.hydrate_from(description, sub_goals);
}
```

Insert at `react_loop/mod.rs:219` (after `seed_messages`, before `should_continue`).

The handler calls it right after `AletheonRuntime` construction (around `handler/mod.rs:425-480` area where the runtime is built):

```rust
// handler/mod.rs -- after runtime construction
if let Some((ref desc, ref subs)) = resumed_objective {
    runtime.seed_goal(desc, subs);
}
```

**OPTION B (Fallback): Add `goal_seed` to `AletheonRuntime`** and thread through its `new()` constructor. This requires changing `AletheonRuntime::new()` signature and is more invasive.

**RECOMMENDATION: Use Option A.** It's a one-line method addition to `ReActLoop` and a one-line call site in the handler. No signature changes.

### Build and test

```bash
cargo test -p runtime goal_tracker::tests::hydrate_from_persisted_objective
# Expected: PASS

cargo test -p runtime goal
# Expected: all 7 goal tests PASS (6 store + 1 tracker)

cargo build -p runtime
# Expected: compiles
```

### Integration test: Resume across restart

```bash
# 1. Start daemon, set a goal
aletheon goal set "multi-turn objective" --scope project
# → id=1

# 2. Stop daemon
aletheon daemon stop

# 3. Restart daemon
aletheon daemon start

# 4. Check resume
aletheon goal resume
# → [1] (in_progress) multi-turn objective

# 5. New chat turn should receive goal context
aletheon "what is my current objective?"
# → Expected: LLM response references "multi-turn objective"
```

---

## 9. Phases 6-7 -- Design-Only Skeletons (GATED)

These phases are **NOT IMPLEMENTED** in the safe slice (Phases 1-5). They are included here as design contracts to prevent architectural divergence between design and eventual implementation.

### Phase 6: Move Decompose into Brain

**Gate:** Phases 1-5 merged + reviewed.
**Dependency:** None (independent of Tier 2a, but safer after safe slice lands).

**Design contract:**

```rust
// cognit/src/core/planner.rs — NEW method (thin entry over existing parse_subtasks)

/// Decompose an objective description into ordered sub-goal strings.
///
/// Thin wrapper: sends objective to LLM via the existing provider path,
/// parses the response with `parse_subtasks` (line 187), and returns
/// a list of sub-goal descriptions.
///
/// TODO(Phase 6): Wire this into `goal.set` JSON-RPC so that when a
/// new objective is set, decomposition runs automatically and persists
/// sub-goals via `ObjectiveStore::create(.., Some(parent), ..)`.
#[allow(dead_code)]  // GATED until Phase 6
pub async fn decompose_objective(
    &self,
    objective_description: &str,
    provider: &dyn crate::r#impl::llm::LlmProvider,
) -> anyhow::Result<Vec<String>> {
    // 1. Build decomposition prompt from objective_description
    // 2. Call provider.complete(prompt)
    // 3. Parse with parse_subtasks(llm_output)
    // 4. Return Vec<String> of sub-goal descriptions
    todo!("Phase 6: GATED — implement after Phases 1-5 land")
}
```

**Acceptance test (golden):** Given a fixed objective description (e.g., "build a REST API"), decomposition from `cognit::decompose_objective` produces the **same** sub-goal strings as `interact::acix::task::decompose` does today. Verify with a golden-file test.

**Migration path for `interact/src/acix/task.rs`:**
1. Add deprecation comment on `decompose` (line 204), `decompose_simple` (line 216), `decompose_with_llm` (line 221)
2. Verify all callers of `TaskManager::decompose*` before deletion
3. Once zero callers remain, delete the three functions

### Phase 7: Autonomous Goal Loop

**Gate:** Tier 2a `PermissionManager` landed + config flag `autonomous_goal_loop = false` (default).

**Design contract:**

```rust
// runtime/src/impl/goal/loop.rs — NEW file (GATED)

use crate::r#impl::goal::ObjectiveStore;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Advances the active objective: picks the next incomplete sub-goal,
/// runs it through the ReAct loop, marks it completed/failed, and
/// stops when all sub-goals resolve.
///
/// # Safety
/// - Gated behind `config.autonomous_goal_loop` (default `false`)
/// - Every advance checks `PermissionManager::allow()` (Tier 2a)
/// - Never generates objectives; only advances user-set ones
/// - Survives mid-run restart via `ObjectiveStore::resume()`
#[allow(dead_code)]  // GATED until Phase 7 + Tier 2a
pub struct GoalLoop {
    store: Arc<Mutex<ObjectiveStore>>,
    enabled: bool,
    // TODO(Tier 2a): permission_manager: Arc<PermissionManager>,
}

impl GoalLoop {
    /// Create a new goal loop. Disabled by default.
    pub fn new(store: Arc<Mutex<ObjectiveStore>>, enabled: bool) -> Self {
        Self { store, enabled }
    }

    /// Check if there is work to do and advance one sub-goal if permitted.
    /// Returns the sub-goal description to inject, or None if idle/disabled.
    pub async fn try_advance(&self) -> Option<String> {
        if !self.enabled {
            return None;
        }
        // TODO(Tier 2a):
        // if !self.permission_manager.allow_autonomous_advance().await {
        //     return None;
        // }
        let store = self.store.lock().await;
        let active = store.active().ok()??;
        let subs = store.sub_goals(active.objective_id).ok()?;
        let next = subs.into_iter().find(|s| {
            s.status == base::objective::ObjectiveStatus::InProgress
        })?;
        Some(next.description)
    }

    /// Mark a sub-goal completed. If all sub-goals are done, mark the parent too.
    pub async fn mark_completed(&self, sub_goal_id: i64, parent_id: i64) -> anyhow::Result<()> {
        let store = self.store.lock().await;
        store.set_status(sub_goal_id, "completed")?;
        let remaining = store.sub_goals(parent_id)?;
        if remaining.iter().all(|s| {
            s.status == base::objective::ObjectiveStatus::Completed
        }) {
            store.set_status(parent_id, "completed")?;
        }
        Ok(())
    }
}
```

**Config flag** in `DaemonConfig` (or `RuntimeConfig`):

```rust
// In the config struct that DaemonConfig/RequestHandler::new uses:
/// Enable autonomous goal-loop advancement. DEFAULT: false.
/// When enabled, the runtime advances through sub-goals without user prompts
/// after each turn, provided the PermissionManager (Tier 2a) allows.
#[serde(default)]  // default = false
pub autonomous_goal_loop: bool,
```

**Acceptance criteria:**
1. Flag OFF: loop never advances; manual `goal status --id N --state completed` works as before.
2. Flag ON + permission granted: objective advances one sub-goal per cycle, surviving mid-run restart via `resume()`.
3. Flag ON + permission denied: loop logs warning and stops; no autonomous advance occurs.
4. No autonomous goal *generation* -- objectives remain user-set.

---

## 10. TDD Test Code and Commands

### Unit tests (run without daemon)

```bash
# Phase 1: Objective types in base
cargo test -p base objective
# Tests: status_serde_roundtrip, status_str_roundtrip, objective_json_roundtrip, summary_conversion

# Phase 2: ObjectiveStore
cargo test -p runtime goal
# Tests: create_and_get_roundtrip, schema_and_indexes_exist,
#        active_returns_latest_in_progress_top_level,
#        resume_reconstructs_active_objective_and_subs,
#        status_filtering, parent_cascade_deletes_sub_goals

# Phase 5: GoalTracker hydrate seam
cargo test -p runtime goal_tracker::tests::hydrate_from_persisted_objective
# Test: hydrate_from_persisted_objective
```

### Integration tests (require daemon running)

```bash
# Phase 3-4 integration: JSON-RPC + CLI
# These use shell scripts sending nc commands to the daemon socket.

# Test: goal.set + goal.show roundtrip
echo '{"jsonrpc":"2.0","id":1,"method":"goal.set","params":{"description":"integration test","scope":"session"}}' | nc -U /run/aletheond/aletheond.sock | jq '.result.objective_id'
# Assert: integer > 0

# Test: goal.resume on fresh start
echo '{"jsonrpc":"2.0","id":1,"method":"goal.resume","params":{}}' | nc -U /run/aletheond/aletheond.sock | jq '.result.objective'
# Assert: null

# Test: goal.status list
echo '{"jsonrpc":"2.0","id":1,"method":"goal.status","params":{"filter":"in_progress"}}' | nc -U /run/aletheond/aletheond.sock | jq '.result.objectives | length'
# Assert: >= 0
```

### Resume-on-start integration test (Phase 5)

```bash
#!/bin/bash
# Test: objective survives daemon restart

# 1. Set a goal
aletheon goal set "resume test objective" --scope project

# 2. Get daemon PID and kill
PID=$(cat /tmp/aletheon/aletheond.pid)
kill $PID
sleep 2

# 3. Restart daemon
aletheon daemon start --detach
sleep 3

# 4. Check resume
RESULT=$(aletheon goal resume)
echo "$RESULT" | grep -q "resume test objective"
if [ $? -eq 0 ]; then
    echo "PASS: objective survived restart"
else
    echo "FAIL: objective lost after restart"
    exit 1
fi
```

### Goal completion triggers next-goal test (Phase 7, design-only)

```bash
#!/bin/bash
# GATED: requires Phase 7 loop enabled + PermissionManager (Tier 2a)
# This script describes the expected behavior for future validation.

# 1. Set objective with sub-goals
aletheon goal set "three step objective" --scope project
# (Phase 6 decompose creates sub-goals 1, 2, 3)

# 2. Enable autonomous loop in config
# config: autonomous_goal_loop = true

# 3. Start daemon and observe:
#    - Sub-goal 1 becomes active
#    - ReAct loop runs turn against sub-goal 1
#    - Sub-goal 1 marked completed
#    - Sub-goal 2 becomes active
#    - ... repeat ...
#    - All sub-goals completed → parent marked completed
#    - Loop idles (no next objective)

# 4. Restart daemon mid-loop
#    - resume() returns the in-progress objective + remaining sub-goals
#    - Loop picks up where it left off
```

---

## 11. Phase Ordering Rationale

| Phase | Depends On | Can Land Independently? | Rationale |
|---|---|---|---|
| 1 (Objective types) | Nothing | Yes | Base types have no deps; can land as first PR |
| 2 (ObjectiveStore) | Phase 1 | No | Needs `Objective` type from base |
| 3 (Daemon store ownership) | Phase 2 | No | Needs `ObjectiveStore` to open |
| 4 (CLI subcommand) | Phase 3 | No | Needs `goal.*` RPC to call |
| 5 (Resume-on-start) | Phase 3 | No | Needs ObjectiveStore wired into handler |
| 6 (Decompose relocation) | Phases 1-5 | Yes (but after safe slice) | Independent of Tier 2a; cross-crate boundary change |
| 7 (Autonomous loop) | Phases 1-5 + Tier 2a | No | Hard dep on PermissionManager |

**Recommended commit sequence:**
1. Phase 1 commit (base types only)
2. Phase 2 commit (store + tests)
3. Phases 3-4 squashed commit (handler field + RPC arms + CLI -- avoids unused-field warning)
4. Phase 5 commit (hydrate seam + resume seed)

Phases 6-7 are on separate branches that build on the merged safe slice.

**Why Phase 1 is independent:** `base/src/types/objective.rs` has no rusqlite deps, no runtime deps, no interact deps. It can compile and test with `cargo test -p base objective` before any other phase lands.

**Why Phases 3-4 should be one commit:** The `objective_store` field in `RequestHandler` (Phase 3) is unused until the JSON-RPC arms (Phase 4) reference it. Rust's `unused` warning-as-error would block `cargo build -p runtime` if they land separately without `#[allow(dead_code)]`. Squashing avoids the dead-code annotation.

**Gating conditions for Phase 6:**
- Safe slice (Phases 1-5) merged to main
- All callers of `interact::acix::task::decompose*` identified
- Golden test file prepared (fixed input → expected sub-goals)

**Gating conditions for Phase 7:**
- Tier 2a `PermissionManager` merged
- `autonomous_goal_loop` config flag defaulting to `false`
- Integration test showing flag OFF = no autonomous advance

---

## 12. Integration Test Strategy

### 12.1 Smoke test matrix

| Scenario | Steps | Expected Result | Phase |
|---|---|---|---|
| Set + show roundtrip | `goal set "X"` → `goal show <id>` | Returns objective with correct description, status=in_progress | 4 |
| Status filter | `goal set "A"`, `goal set "B"`, `goal status --id <A> --state completed`, `goal status --filter in_progress` | Only B listed | 4 |
| Resume empty | Fresh daemon, `goal resume` | `objective: null` | 4 |
| Resume with active | `goal set "X"`, `goal resume` | Returns X as active | 4 |
| Restart persistence | `goal set "X"`, stop daemon, start daemon, `goal resume` | Returns X as active | 5 |
| Sub-goal listing | `goal show <parent with sub-goals>` | All children listed under sub_goals | 4 |
| Cascade delete | Delete parent, check children | Children cascade-deleted (unit test only) | 2 |
| Concurrent sessions | Two sessions each set an objective | Each sees only its own via `session_id` filter | 4 |

### 12.2 Resume-on-start edge cases

1. **Corrupt DB file:** `objectives.db` is truncated/malformed → `ObjectiveStore::open` returns `Err` → daemon logs warning, starts with `resumed_objective = None` (no crash).
2. **Missing `~/.aletheon/` directory:** `create_dir_all` already runs before FactStore open (line 222). ObjectiveStore open gets the same guarantee.
3. **Multiple active objectives (concurrent sessions):** `active()` returns the newest `in_progress` top-level. If two sessions set objectives simultaneously, the newest wins. This is correct for MVP (single-objective). Multi-objective scheduling is a non-goal.
4. **Resume with completed objective:** `active()` filters `status = 'in_progress'`. A completed objective is not returned; `resume()` returns `None`.

### 12.3 File-based integration test (can run in CI)

```rust
// tests/integration/goal_resume_test.rs (in runtime crate, under tests/)
// Requires: tempfile in dev-dependencies

#[cfg(test)]
mod goal_integration {
    use tempfile::TempDir;
    use runtime::r#impl::goal::ObjectiveStore;

    #[test]
    fn resume_survives_close_reopen() {
        let dir = TempDir::new().unwrap();
        let db_path = dir.path().join("objectives.db");

        // Create and populate
        {
            let store = ObjectiveStore::open(&db_path).unwrap();
            let id = store.create("persistent goal", None, "s1", "project").unwrap();
            store.create("milestone 1", Some(id), "s1", "project").unwrap();
        } // store dropped → DB connection closed

        // Reopen and resume
        {
            let store = ObjectiveStore::open(&db_path).unwrap();
            let (obj, subs) = store.resume().unwrap().unwrap();
            assert_eq!(obj.description, "persistent goal");
            assert_eq!(subs.len(), 1);
            assert_eq!(subs[0].description, "milestone 1");
        }
    }
}
```

---

## 13. Rollback Plan

### Per-phase rollback

| Phase | Rollback | Risk |
|---|---|---|
| 1 | Remove `objective.rs` + revert `types/mod.rs` + `lib.rs` insertions | Zero -- no callers yet |
| 2 | Remove `impl/goal/` + revert `impl/mod.rs` insertion | Zero -- no callers yet |
| 3-4 | Remove handler field + RPC arms + CLI command | Low -- no data migration. If objectives.db was created, it's just a file with no other consumers. |
| 5 | Remove `hydrate_from` from `goal_tracker.rs`, `seed_goal` from `react_loop/mod.rs`, handler field | Low -- GoalTracker API is additive; removing it restores exact pre-existing behavior. |
| 6 | Remove `decompose_objective` from planner.rs, restore acix/task.rs deprecation comments | Medium -- if any caller was migrated, restore it back to interact::acix::task::decompose |
| 7 | Remove `loop.rs`, config flag, handler wiring | Low -- flag defaults to false; removing the code is additive-only removal |

### Full rollback (emergency: revert all phases)

```bash
git revert <phase-5-commit> <phase-3-4-commit> <phase-2-commit> <phase-1-commit>
```

The ObjectiveStore DB file (`~/.aletheon/objectives.db`) has no other consumers and can be safely left or deleted. It is not read by any pre-existing code path.

### Data migration on rollback

No data is migrated in the safe slice. The `objectives` table is created fresh and only populated via `goal.set` CLI/RPC. Rolling back leaves the SQLite file orphaned -- no recovery needed.

---

## 14. Risk Assessment

### 14.1 Autonomous loop safety (Phase 7)

**Risk:** Autonomous goal advancement without user oversight could run destructive tool sequences.

**Mitigation:**
- Default OFF (`autonomous_goal_loop: false`)
- Every advance gated on Tier 2a `PermissionManager`
- Objectives are user-set only (no autonomous generation)
- Tool execution still goes through the existing `ToolRunnerWithGuard` (sandbox, approval, audit)
- Loop stops on sub-goal failure (no retry without user intervention)

**Residual risk:** LOW with both gates active. MEDIUM if flag is ON but PermissionManager is not yet hardened (Tier 2a risk, not M-I risk).

### 14.2 Goal decomposition quality (Phase 6)

**Risk:** LLM-based decomposition may produce nonsensical or cyclic sub-goals.

**Mitigation:**
- Decomposition is invoked per objective set, not loop-automated
- User can review sub-goals via `goal show <id>` immediately after `goal set`
- Sub-goals are persisted independently; user can override via `goal.status --id <sub> --state adjusted`
- Golden test verifies decomposition output matches expected format

**Residual risk:** LOW -- LLM output is reviewable before loop acts on it.

### 14.3 Concurrent session goal conflicts

**Risk:** Two daemon sessions set objectives simultaneously. `active()` returns the newest top-level.

**Mitigation:**
- MVP is single-objective; concurrent sessions are a non-goal for this slice
- `session_id` column allows future per-session `active()` filtering
- `scope` column (`session`/`project`/`global`) provides future scoping
- Current `active()` query: `WHERE status = 'in_progress' AND parent_id IS NULL ORDER BY objective_id DESC LIMIT 1` -- returns exactly one, newest-first

**Residual risk:** LOW for single-user MVP. MEDIUM for multi-user deployment (not targeted).

### 14.4 `rusqlite::Connection` is not `Send`

**Risk:** Holding a `Connection` across an `.await` point in async contexts causes compile errors.

**Mitigation:**
- `ObjectiveStore` held behind `Arc<Mutex<ObjectiveStore>>` (Send-safe)
- Every RPC arm locks, does synchronous DB work, drops the lock before any `.await`
- Pattern mirrors `fact_store` exactly (verified at `rpc.rs:61`)

**Residual risk:** NEGLIGIBLE -- compile-time enforced by `Send` bound.

### 14.5 Column order drift

**Risk:** `map_objective_row` reads by positional index. A SELECT using a different column order silently reads wrong data.

**Mitigation:**
- Every SELECT uses the shared `COLS` constant (`store.rs` line 5)
- No `SELECT *` anywhere
- Schema test (`schema_and_indexes_exist`) verifies 8 columns in correct order
- If a migration adds a column to the end of the table, `COLS` and `map_objective_row` must be updated atomically

**Residual risk:** LOW -- caught by schema test; convention enforced by `COLS` constant and code review.

### 14.6 Phantom file reference (`core/chat.rs`)

**Risk:** The original plan referenced `runtime/src/core/chat.rs:113-146` as the injection point for FactStore recall into the ReAct loop. This file does not exist. The actual injection happens in `handler/chat.rs` and via `compose_memory_block()` in `handler/mod.rs:803`.

**Mitigation for Phase 5-7:** Use the actual injection points:
- Goal context injects via `ReActLoop::seed_goal()` (Phase 5) or the handler's prompt construction in `handler/chat.rs`
- Do NOT create `core/chat.rs` -- it would be a new file with no existing callers

**Residual risk:** NEGLIGIBLE -- this design document corrects the reference. The plan's conceptual intent (inject goal into ReAct context) is preserved; only the file path is wrong.

### 14.7 Test isolation (tempfile SQLite)

**Risk:** Tests using `tempfile::NamedTempFile` for SQLite databases may leak file descriptors on panic.

**Mitigation:**
- `NamedTempFile` implements `Drop` -- the file is deleted when it goes out of scope, even on panic
- Tests use `setup()` helper that returns `(ObjectiveStore, NamedTempFile)` -- both are dropped at end of test
- `cargo test` runs each test in its own thread by default (no shared DB state)

**Residual risk:** NEGLIGIBLE.

---

## 15. Complete File Manifest

| File | Status | Phase | Description |
|---|---|---|---|
| `crates/base/src/types/objective.rs` | NEW | 1 | Objective, ObjectiveStatus, ObjectiveSummary types |
| `crates/base/src/types/mod.rs` | MODIFY: insert line 11 | 1 | Add `pub mod objective;` |
| `crates/base/src/lib.rs` | MODIFY: insert lines 51, 121 | 1 | Re-export objective types |
| `crates/runtime/src/impl/goal/mod.rs` | NEW | 2 | ObjectiveStore struct, schema, open, map, tests |
| `crates/runtime/src/impl/goal/store.rs` | NEW | 2 | CRUD + query API (create/get/set_status/list/active/sub_goals/resume) |
| `crates/runtime/src/impl/mod.rs` | MODIFY: insert line 8 | 2 | Add `pub mod goal;` |
| `crates/runtime/src/impl/daemon/handler/mod.rs` | MODIFY: lines 63, 140, 226, 668, etc. | 3, 5 | Import, field, open, init, resume seed |
| `crates/runtime/src/impl/daemon/handler/rpc.rs` | MODIFY: after line 125 | 3 | `goal.set/show/status/resume` JSON-RPC arms |
| `crates/interact/src/tui/cli.rs` | MODIFY: lines 6, 105, 169 | 4 | GoalAction enum, Command variant, dispatch arm |
| `crates/interact/src/tui/goal.rs` | NEW | 4 | CLI handlers (run, send_rpc) |
| `crates/runtime/src/core/react_loop/goal_tracker.rs` | MODIFY: lines 262, 437 | 5 | `hydrate_from` seam + test |
| `crates/runtime/src/core/react_loop/mod.rs` | MODIFY: line 219 | 5 | `seed_goal` method on ReActLoop |
| `crates/runtime/src/impl/goal/loop.rs` | NEW (GATED) | 7 | GoalLoop skeleton (design-only, TODO Tier 2a) |
| `crates/cognit/src/core/planner.rs` | MODIFY (GATED) | 6 | `decompose_objective` skeleton (design-only) |

**Total files:** 13 (5 new, 8 modified). Phases 6-7 files are design-only skeletons, not built.

---

## 16. Summary

The original M-I plan's ground truth claims are **87.5% accurate** (14/16 exact matches, 2 minor line offsets of 1-3 lines). The one material drift is the phantom `core/chat.rs` reference, which this design corrects by using the actual injection points in `handler/chat.rs` and `handler/mod.rs`.

The safe slice (Phases 1-5) delivers:
- A persisted `ObjectiveStore` mirroring `FactStore`'s SQLite pattern
- `goal.set/show/status/resume` JSON-RPC + CLI
- Resume-on-start for session continuity across daemon restarts
- Zero changes to the hot ReAct loop path beyond a one-line `hydrate_from` seam

Phases 6-7 are fully designed with skeleton code and `TODO(Tier 2a)` markers, ready for implementation once their gates are met.
