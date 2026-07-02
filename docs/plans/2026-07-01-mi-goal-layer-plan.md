# M-I — Goal Layer / Persistent Objectives — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Design-only handoff — do not execute product changes until the design-only gate is lifted.**

**Goal:** Give the `runtime` a *persisted* objective. Today a goal lives only for one ReAct loop run (`GoalTracker` is in-memory) and decomposition sits in the wrong crate (`interact`). This plan lands the safe, high-value slice: a SQLite-backed `ObjectiveStore` (mirroring `FactStore`), a `goal.*` JSON-RPC surface, an `aletheon goal` CLI subcommand, resume-of-the-active-objective on daemon start, and backing the in-memory `GoalTracker` with the store. The autonomous Goal→Next-Goal loop and moving `decompose` into `cognit` are designed here but marked LATER (gated on Tier 2a `PermissionManager`, default-off).

**Architecture:** Add a `runtime`-owned `ObjectiveStore` that reuses `FactStore`'s SQLite open/schema pattern (WAL pragma, `CREATE TABLE IF NOT EXISTS`, positional `map_*_row`). The daemon `RequestHandler` owns it behind `Arc<Mutex<_>>` exactly like `fact_store`, opens it under `~/.aletheon/`, and exposes it through the untyped JSON-RPC `match method` fallthrough plus a new `interact` subcommand. On startup the handler resumes the single active top-level objective into the live `GoalTracker`.

**Tech Stack:** Rust, `rusqlite` (SQLite, `bundled`), `tokio`, `serde_json`, `clap`, `tempfile` (tests).

**Spec:** `docs/plans/2026-07-01-modules-roadmap-design.md` § "M-I. Goal Layer / Persistent Objectives"

**Branch:** `auro/feat/20260701-aletheon-goal-layer` (own branch per repo policy).

---

## Ground truth (verified 2026-07-01)

| Claim | Evidence |
|---|---|
| Cargo package names are bare (`runtime`,`interact`,`cognit`,`base`) — NOT `aletheon-*` | `crates/runtime/Cargo.toml` `name = "runtime"`; `crates/interact/Cargo.toml` `name = "interact"`; `crates/cognit/Cargo.toml` `name = "cognit"`; `crates/base/Cargo.toml` `name = "base"` |
| Goal state is in-memory + react-loop-scoped | `crates/runtime/src/core/react_loop/goal_tracker.rs:70` `struct GoalTracker`; `:55` `struct Goal { description, created_at: Instant, status }`; `:46` `enum GoalStatus { InProgress, Completed, Failed, Adjusted }` |
| `Goal`/`GoalStatus` are NOT serde-serializable (only `Debug`/`Clone`/`PartialEq`) | `goal_tracker.rs:45,54` derive lists — no `Serialize` |
| GoalTracker is created fresh per loop and reset per turn | `react_loop/mod.rs` `let goal_tracker = GoalTracker::new();` + `self.goal_tracker.reset();`; `goal_tracker.rs:256` `fn reset` clears all but `spec_source` |
| React loop reads only the description | `react_loop/step.rs` + `react_loop/tool_exec.rs` `goal: self.goal_tracker.current_goal_description()` (`goal_tracker.rs:153`) |
| Decomposition lives in the INTERFACE crate (wrong layer) | `crates/interact/src/acix/task.rs:204` `pub fn decompose(goal: &str) -> TaskGraph`; `:216` `decompose_simple`; `:221` `decompose_with_llm` |
| Brain has a real planner to reuse for decomposition | `crates/cognit/src/core/planner.rs:52` `generate_multi_step_plan`; `:187` `parse_subtasks` |
| `FactStore` SQLite pattern to mirror | `crates/runtime/src/impl/memory/fact_store/mod.rs:97` `pub fn open(path)`; `:99` `PRAGMA journal_mode=WAL`; `:100` `create_schema`; `:104` `CREATE TABLE IF NOT EXISTS`; `:92` `pub(crate) db: Connection`; `:250` `map_fact_row` (positional `row.get(i)`) |
| `add_fact` insert idiom (INSERT then `query_row` for id) | `fact_store/query.rs:14` `pub fn add_fact(..) -> Result<i64>`; `:24` `INSERT OR IGNORE`; `:29` `SELECT ... query_row` |
| rusqlite stores live under `impl/`, not `core/` | only `crates/runtime/src/impl/**` uses `rusqlite` (`impl/memory/fact_store`, `impl/session/store.rs`, `impl/session/journal.rs`); `crates/runtime/src/core/**` uses none |
| Handler owns `fact_store` behind `Arc<Mutex<_>>` | `crates/runtime/src/impl/daemon/handler/mod.rs:139` `fact_store: Arc<Mutex<FactStore>>`; `:640` `Self { .. fact_store, .. }` at `:667` |
| Store is opened under `~/.aletheon/` in `RequestHandler::new` | `handler/mod.rs:195` `pub async fn new`; `:219` `aletheon_dir = home/.aletheon`; `:223` `FactStore::open(&aletheon_dir.join("fact_store.db"))`; `:225` `Arc::new(Mutex::new(..))` |
| Untyped JSON-RPC dispatch + shape to mirror | `handler/mod.rs:900` `_ => self.handle_rpc(&method, id, request).await`; `handler/rpc.rs:18` `handle_rpc(&self, method, id, request)`; `:24` `match method`; `:95` `"reflect"` arm returns `json!({"jsonrpc":"2.0","id":id,"result":{..}})`; `:61` `let fs = self.fact_store.lock().await` |
| CLI subcommand + dispatch pattern to mirror | `crates/interact/src/tui/cli.rs:70` `enum Command`; `:155` `handle_command`; `:168` `Command::Debug { action } => debug::run(socket, action).await`; `DEFAULT_SOCKET` `:17` |
| `send_rpc` helper exists but is private in `debug.rs` | `crates/interact/src/tui/debug.rs:1194` `async fn send_rpc(socket, request) -> Result<Value>` (not `pub`) |
| Module registration idiom | `crates/runtime/src/impl/mod.rs:10` `pub mod memory;`; `impl/memory/mod.rs:8` `pub mod fact_store;` |

---

## File map

| File | Change |
|---|---|
| `crates/runtime/src/impl/goal/mod.rs` | **new** — module root; `ObjectiveStore`, `ObjectiveRow`, schema, `open`, `map_objective_row`, `#[cfg(test)]` |
| `crates/runtime/src/impl/goal/store.rs` | **new** — CRUD/query API (`create`/`get`/`set_status`/`list`/`active`/`sub_goals`/`resume`) |
| `crates/runtime/src/impl/mod.rs` | add `pub mod goal;` |
| `crates/runtime/src/impl/daemon/handler/mod.rs` | add `objective_store: Arc<Mutex<ObjectiveStore>>` field; open it in `new()`; resume active objective into `GoalTracker` seed |
| `crates/runtime/src/impl/daemon/handler/rpc.rs` | `goal.*` JSON-RPC arms |
| `crates/interact/src/tui/cli.rs` | `Goal` subcommand + `goal_cmd` handler (own `send_rpc` copy) |
| `crates/runtime/src/core/react_loop/goal_tracker.rs` | (Phase 5) `GoalTracker::hydrate_from(ObjectiveRow)` seam so the tracker can be backed by a persisted objective |

> Package/command note: the sibling `governed-memory` plan wrote `cargo build -p aletheon-runtime` / `-p aletheon`; those package names are **wrong** (drift). This plan uses the real names: `cargo test -p runtime` and `cargo build -p interact`.

> Placement note: the roadmap's literal path is `runtime/src/core/goal/`, but every rusqlite-backed store in `runtime` lives under `impl/` (verified). This plan places the store under `impl/goal/` to mirror `FactStore` and keep the DB layer out of `core/`; the `core/react_loop/goal_tracker.rs` seam (Phase 5) is the `core/` surface. See Risks.

Each phase ends with a build + commit. Default checks: `cargo build -p runtime` and `cargo test -p runtime goal` unless noted.

---

## Phase 1 — `ObjectiveStore`: schema + open + create/get

### Task 1: New module, schema, `open`, `create`, `get`

**Files:**
- Create: `crates/runtime/src/impl/goal/mod.rs`
- Create: `crates/runtime/src/impl/goal/store.rs`
- Modify: `crates/runtime/src/impl/mod.rs` (add `pub mod goal;` after `pub mod engine;`, keeping alpha order)

- [ ] **Step 1: Write the failing test**

```rust
// crates/runtime/src/impl/goal/mod.rs — tests module
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
        assert_eq!(row.status, "in_progress");
        assert_eq!(row.session_id, "sess-1");
        assert_eq!(row.scope, "project");
        assert!(row.parent_id.is_none());
    }

    #[test]
    fn schema_and_indexes_exist() {
        let (store, _tmp) = setup();
        let names: Vec<String> = store
            .db
            .prepare("SELECT name FROM sqlite_master WHERE type IN ('table','index') ORDER BY name")
            .unwrap()
            .query_map([], |r| r.get(0))
            .unwrap()
            .collect::<Result<_, _>>()
            .unwrap();
        assert!(names.iter().any(|n| n == "objectives"));
        assert!(names.iter().any(|n| n == "idx_objectives_status"));
        assert!(names.iter().any(|n| n == "idx_objectives_parent"));
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p runtime goal::tests::create_and_get_roundtrip`
Expected: FAIL — `impl/goal` module / `ObjectiveStore` do not exist.

- [ ] **Step 3: Implement the module + schema + create/get**

```rust
// crates/runtime/src/impl/goal/mod.rs
mod store;

use anyhow::{Context, Result};
use rusqlite::Connection;

/// A persisted objective (top-level goal or a sub-goal via `parent_id`).
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ObjectiveRow {
    pub objective_id: i64,
    pub description: String,
    /// One of: 'in_progress' | 'completed' | 'failed' | 'adjusted'
    /// (mirrors `react_loop::goal_tracker::GoalStatus`).
    pub status: String,
    pub parent_id: Option<i64>,
    pub session_id: String,
    pub scope: String,
    pub created_at: String,
    pub updated_at: String,
}

/// SQLite-backed objective store. Mirrors `FactStore`'s open/schema idiom
/// (`impl/memory/fact_store/mod.rs`): WAL, `CREATE TABLE IF NOT EXISTS`,
/// positional `map_*_row`.
pub struct ObjectiveStore {
    pub(crate) db: Connection,
}

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
                scope        TEXT NOT NULL DEFAULT 'session',
                created_at   TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_objectives_status ON objectives(status);
            CREATE INDEX IF NOT EXISTS idx_objectives_parent ON objectives(parent_id);",
        )?;
        Ok(())
    }

    pub(crate) fn map_objective_row(row: &rusqlite::Row) -> rusqlite::Result<ObjectiveRow> {
        Ok(ObjectiveRow {
            objective_id: row.get(0)?,
            description: row.get(1)?,
            status: row.get(2)?,
            parent_id: row.get(3)?,
            session_id: row.get(4)?,
            scope: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
        })
    }
}
```

```rust
// crates/runtime/src/impl/goal/store.rs
use super::{ObjectiveRow, ObjectiveStore};
use anyhow::Result;

/// Fixed column order — every SELECT feeding `map_objective_row` MUST match.
pub(crate) const COLS: &str =
    "objective_id, description, status, parent_id, session_id, scope, created_at, updated_at";

impl ObjectiveStore {
    /// Insert an objective (or sub-goal when `parent` is `Some`). Returns its id.
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
    pub fn get(&self, id: i64) -> Result<Option<ObjectiveRow>> {
        let sql = format!("SELECT {COLS} FROM objectives WHERE objective_id = ?1");
        let mut stmt = self.db.prepare(&sql)?;
        let mut rows = stmt.query_map(rusqlite::params![id], Self::map_objective_row)?;
        match rows.next() {
            Some(r) => Ok(Some(r?)),
            None => Ok(None),
        }
    }
}
```

```rust
// crates/runtime/src/impl/mod.rs — add in alpha order (after `pub mod engine;`)
pub mod goal;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p runtime goal::tests::create_and_get_roundtrip goal::tests::schema_and_indexes_exist`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/goal/ crates/runtime/src/impl/mod.rs
git commit -m "feat(goal): SQLite-backed ObjectiveStore with schema + create/get"
```

---

## Phase 2 — Query API: status, list, active, sub-goals, resume

### Task 2: `set_status` / `list` / `active` / `sub_goals` / `resume`

**Files:** Modify `crates/runtime/src/impl/goal/store.rs`.

- [ ] **Step 1: Write the failing test**

```rust
// crates/runtime/src/impl/goal/mod.rs — tests module
#[test]
fn active_returns_latest_in_progress_top_level() {
    let (store, _tmp) = setup();
    let a = store.create("first objective", None, "s", "session").unwrap();
    let b = store.create("second objective", None, "s", "session").unwrap();
    // sub-goals of `b`
    store.create("sub one", Some(b), "s", "session").unwrap();
    store.create("sub two", Some(b), "s", "session").unwrap();
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
    let obj = store.create("resume me", None, "s", "project").unwrap();
    store.create("child a", Some(obj), "s", "project").unwrap();
    let (active, subs) = store.resume().unwrap().unwrap();
    assert_eq!(active.objective_id, obj);
    assert_eq!(subs.len(), 1);
    // list(None) returns all objectives; list(Some("in_progress")) filters
    assert_eq!(store.list(None, 50).unwrap().len(), 2);
    assert_eq!(store.list(Some("in_progress"), 50).unwrap().len(), 2);
}
```

- [ ] **Step 2: Run — expected FAIL** (`set_status`/`list`/`active`/`sub_goals`/`resume` undefined).

Run: `cargo test -p runtime goal::tests::active_returns_latest_in_progress_top_level goal::tests::resume_reconstructs_active_objective_and_subs`

- [ ] **Step 3: Implement**

```rust
// crates/runtime/src/impl/goal/store.rs — extend impl ObjectiveStore
impl ObjectiveStore {
    /// Update status; bumps `updated_at`. Returns true if a row changed.
    pub fn set_status(&self, id: i64, status: &str) -> Result<bool> {
        Ok(self.db.execute(
            "UPDATE objectives SET status = ?1, updated_at = datetime('now')
             WHERE objective_id = ?2",
            rusqlite::params![status, id],
        )? > 0)
    }

    /// List objectives, optionally filtered by status, newest first.
    pub fn list(&self, status: Option<&str>, limit: usize) -> Result<Vec<ObjectiveRow>> {
        let mut sql = format!("SELECT {COLS} FROM objectives");
        if status.is_some() {
            sql.push_str(" WHERE status = ?1");
        }
        sql.push_str(&format!(" ORDER BY objective_id DESC LIMIT {}", limit as i64));
        let mut stmt = self.db.prepare(&sql)?;
        let rows = if let Some(s) = status {
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
    pub fn active(&self) -> Result<Option<ObjectiveRow>> {
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
    pub fn sub_goals(&self, parent: i64) -> Result<Vec<ObjectiveRow>> {
        let sql = format!(
            "SELECT {COLS} FROM objectives WHERE parent_id = ?1 ORDER BY objective_id ASC"
        );
        let mut stmt = self.db.prepare(&sql)?;
        let rows = stmt
            .query_map(rusqlite::params![parent], Self::map_objective_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?;
        Ok(rows)
    }

    /// Resume: the active objective plus its sub-goals, if any.
    pub fn resume(&self) -> Result<Option<(ObjectiveRow, Vec<ObjectiveRow>)>> {
        match self.active()? {
            Some(obj) => {
                let subs = self.sub_goals(obj.objective_id)?;
                Ok(Some((obj, subs)))
            }
            None => Ok(None),
        }
    }
}
```

- [ ] **Step 4: Run — expected PASS.** Full module: `cargo test -p runtime goal`.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/goal/store.rs crates/runtime/src/impl/goal/mod.rs
git commit -m "feat(goal): status/list/active/sub_goals/resume query API"
```

---

## Phase 3 — Daemon: own the store + `goal.*` JSON-RPC

### Task 3: Wire `ObjectiveStore` into `RequestHandler`

**Files:** Modify `crates/runtime/src/impl/daemon/handler/mod.rs`.

- [ ] **Step 1: Add the field, import, open, and struct init** (mirror `fact_store` exactly)

```rust
// handler/mod.rs — near the FactStore import (:63)
use crate::r#impl::goal::ObjectiveStore;
```

```rust
// handler/mod.rs — in `pub struct RequestHandler`, next to `fact_store` (:139)
    objective_store: Arc<Mutex<ObjectiveStore>>,
```

```rust
// handler/mod.rs — in new(), right after fact_store is built (:223-225)
let objective_store = ObjectiveStore::open(&aletheon_dir.join("objectives.db"))
    .context("opening objective store")?;
let objective_store = Arc::new(Mutex::new(objective_store));
```

```rust
// handler/mod.rs — in the `Self { .. }` literal (:640), next to `fact_store,` (:667)
            objective_store,
```

- [ ] **Step 2: Build** `cargo build -p runtime` — expected: compiles (field is used by Task 4's arms; until then, add `#[allow(dead_code)]` above the field OR land Task 3 + Task 4 in the same commit to avoid the unused-field warning-as-error). Prefer landing 3+4 together.

- [ ] **Step 3: Commit** (fold into Task 4's commit if landing together)

```bash
git add crates/runtime/src/impl/daemon/handler/mod.rs
git commit -m "feat(daemon): RequestHandler owns a persisted ObjectiveStore"
```

### Task 4: `goal.*` JSON-RPC arms

**Files:** Modify `crates/runtime/src/impl/daemon/handler/rpc.rs` (add arms in the `match method` block at `:24`; mirror the `"reflect"` arm shape at `:95` — lock, call, build `json!` result/error). These are reached via the `_ => self.handle_rpc(..)` fallthrough at `handler/mod.rs:900`.

- [ ] **Step 1: Add the arms**

```rust
// handler/rpc.rs — inside `match method { .. }`
"goal.set" => {
    let p = &request["params"];
    let description = p["description"].as_str().unwrap_or("");
    let scope = p["scope"].as_str().unwrap_or("session");
    let session_id = self.session_manager.lock().await.session_id.clone();
    let store = self.objective_store.lock().await;
    match store.create(description, None, &session_id, scope) {
        Ok(id) => json!({"jsonrpc":"2.0","id":id,"result":{"objective_id":id}}),
        Err(e) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32020,"message":e.to_string()}}),
    }
}
"goal.show" => {
    let oid = request["params"]["id"].as_i64().unwrap_or(0);
    let store = self.objective_store.lock().await;
    match store.get(oid) {
        Ok(Some(obj)) => {
            let subs = store.sub_goals(oid).unwrap_or_default();
            json!({"jsonrpc":"2.0","id":id,"result":{"objective":obj,"sub_goals":subs}})
        }
        Ok(None) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32021,"message":"not found"}}),
        Err(e) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32020,"message":e.to_string()}}),
    }
}
"goal.status" => {
    let p = &request["params"];
    let store = self.objective_store.lock().await;
    // With an id: update status. Without: list objectives.
    if let Some(oid) = p["id"].as_i64() {
        let status = p["status"].as_str().unwrap_or("in_progress");
        match store.set_status(oid, status) {
            Ok(ok) => json!({"jsonrpc":"2.0","id":id,"result":{"ok":ok}}),
            Err(e) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32020,"message":e.to_string()}}),
        }
    } else {
        match store.list(p["filter"].as_str(), 50) {
            Ok(rows) => json!({"jsonrpc":"2.0","id":id,"result":{"objectives":rows}}),
            Err(e) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32020,"message":e.to_string()}}),
        }
    }
}
"goal.resume" => {
    let store = self.objective_store.lock().await;
    match store.resume() {
        Ok(Some((obj, subs))) =>
            json!({"jsonrpc":"2.0","id":id,"result":{"objective":obj,"sub_goals":subs}}),
        Ok(None) => json!({"jsonrpc":"2.0","id":id,"result":{"objective":null}}),
        Err(e) => json!({"jsonrpc":"2.0","id":id,"error":{"code":-32020,"message":e.to_string()}}),
    }
}
```

> `ObjectiveRow` derives `Serialize` (Task 1), so `json!({"objective": obj})` works. The `id` binding is the `handle_rpc` parameter (`rpc.rs:21`), reused exactly as the existing arms do.

- [ ] **Step 2: Build** `cargo build -p runtime` — expected: compiles (the `objective_store` field is now used).

- [ ] **Step 3: Manual smoke** (daemon running):
  `echo '{"jsonrpc":"2.0","id":1,"method":"goal.set","params":{"description":"ship goal layer","scope":"project"}}' | nc -U /run/aletheond/aletheond.sock`
  Expected: `{"result":{"objective_id":<n>}}`. Then `goal.show` with `{"id":<n>}` returns the objective; `goal.resume` returns it as the active objective.

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/src/impl/daemon/handler/rpc.rs crates/runtime/src/impl/daemon/handler/mod.rs
git commit -m "feat(daemon): goal.* JSON-RPC (set/show/status/resume)"
```

---

## Phase 4 — `interact` CLI `goal` subcommand

### Task 5: `aletheon goal` subcommand

**Files:** Modify `crates/interact/src/tui/cli.rs` (variant in `enum Command` `:70`, arm in `handle_command` `:155`, a new async handler using a local `send_rpc` copy — `debug.rs:1194`'s `send_rpc` is private).

- [ ] **Step 1: Add the subcommand enum + `Command` variant**

```rust
// cli.rs — new enum near Command
#[derive(Subcommand)]
pub enum GoalAction {
    /// Set the active objective: goal set "description" [--scope project]
    Set { description: String, #[arg(long, default_value = "session")] scope: String },
    /// Show one objective (with its sub-goals) by id
    Show { id: i64 },
    /// List objectives, or update one: goal status [--id N --state completed]
    Status { #[arg(long)] id: Option<i64>,
             #[arg(long)] state: Option<String>,
             #[arg(long)] filter: Option<String> },
    /// Resume the active objective
    Resume,
}

// add to enum Command (after Command::Debug { .. }):
    /// Persistent goal / objective management
    Goal {
        #[command(subcommand)]
        action: GoalAction,
    },
```

- [ ] **Step 2: Add the dispatch arm + handler**

```rust
// cli.rs handle_command match (after the Debug arm at :168):
        Command::Goal { action } => goal_cmd(socket, action).await,
```

```rust
// cli.rs — new async fn. Local send_rpc copy (debug.rs::send_rpc is private).
async fn goal_send_rpc(socket: &std::path::Path, req: &serde_json::Value) -> Result<serde_json::Value> {
    let mut stream = UnixStream::connect(socket).await?;
    stream.write_all(serde_json::to_string(req)?.as_bytes()).await?;
    stream.write_all(b"\n").await?;
    let (reader, _) = stream.split();
    let mut reader = BufReader::new(reader);
    let mut line = String::new();
    reader.read_line(&mut line).await?;
    Ok(serde_json::from_str(&line)?)
}

async fn goal_cmd(socket: &PathBuf, action: GoalAction) -> Result<()> {
    let req = match &action {
        GoalAction::Set { description, scope } => serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"goal.set",
            "params":{"description":description,"scope":scope}}),
        GoalAction::Show { id } => serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"goal.show","params":{"id":id}}),
        GoalAction::Status { id, state, filter } => serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"goal.status",
            "params":{"id":id,"status":state,"filter":filter}}),
        GoalAction::Resume => serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"goal.resume","params":{}}),
    };
    let resp = goal_send_rpc(socket, &req).await?;
    if let Some(objs) = resp["result"]["objectives"].as_array() {
        for o in objs {
            println!("[{}] ({}) {}", o["objective_id"], o["status"].as_str().unwrap_or("?"),
                     o["description"].as_str().unwrap_or(""));
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&resp["result"]).unwrap_or_default());
    }
    Ok(())
}
```

- [ ] **Step 3: Build** `cargo build -p interact` — expected: compiles.

- [ ] **Step 4: End-to-end** (daemon running):
  `aletheon goal set "ship the goal layer" --scope project` → prints `objective_id`.
  `aletheon goal status` → lists it as `in_progress`.
  `aletheon goal show <id>` → prints the objective + `sub_goals`.
  `aletheon goal status --id <id> --state completed` → `goal status` no longer lists it as active; `aletheon goal resume` returns `null`.

- [ ] **Step 5: Commit**

```bash
git add crates/interact/src/tui/cli.rs
git commit -m "feat(cli): aletheon goal set/show/status/resume"
```

---

## Phase 5 — Resume the active objective into `GoalTracker` on start

### Task 6: Hydrate the in-memory tracker from the persisted active objective

**Files:** Modify `crates/runtime/src/core/react_loop/goal_tracker.rs` (add a hydrate seam) and `crates/runtime/src/impl/daemon/handler/mod.rs` (call `resume()` and seed).

> Rationale: `GoalTracker` is created fresh and `reset()` per turn (`react_loop/mod.rs`, `goal_tracker.rs:256`). To make a persisted objective *survive restart and resume*, the handler reads the active objective at startup and seeds the tracker's first turn from it. This does NOT change per-turn reset semantics; it changes what the tracker is seeded WITH.

- [ ] **Step 1: Write the failing test** (tracker can be hydrated from a persisted objective's description + sub-goals)

```rust
// goal_tracker.rs tests module
#[test]
fn hydrate_from_persisted_objective() {
    let mut tracker = GoalTracker::new();
    tracker.hydrate_from("ship goal layer", &["persist store".to_string(), "wire rpc".to_string()]);
    assert_eq!(tracker.current_goal_description(), Some("ship goal layer".into()));
    assert!(tracker.get_context().contains("persist store"));
    assert!(tracker.get_context().contains("wire rpc"));
}
```

- [ ] **Step 2: Run — expected FAIL** (`hydrate_from` undefined).

Run: `cargo test -p runtime goal_tracker::tests::hydrate_from_persisted_objective`

- [ ] **Step 3: Implement the seam + the startup seed**

```rust
// goal_tracker.rs — new method on impl GoalTracker (thin wrapper over set_goal/add_sub_goal)
/// Seed the tracker from a persisted objective (description + ordered sub-goals).
/// Used to resume a cross-session objective on daemon start.
pub fn hydrate_from(&mut self, description: &str, sub_goals: &[String]) {
    self.set_goal(description.to_string());
    for sg in sub_goals {
        self.add_sub_goal(sg.clone());
    }
}
```

```rust
// handler/mod.rs — in new(), after objective_store is built (Task 3), log/prepare the seed.
// The tracker lives inside the ReAct loop; seed it when the loop's tracker is first
// constructed for this session. Read the persisted active objective here:
if let Ok(store) = objective_store.try_lock() {
    if let Ok(Some((obj, subs))) = store.resume() {
        let sub_desc: Vec<String> = subs.iter().map(|s| s.description.clone()).collect();
        info!(objective_id = obj.objective_id, description = %obj.description,
              sub_goals = sub_desc.len(), "Resuming persisted objective on start");
        // Store the resumed seed on the handler (e.g. Option<(String, Vec<String>)>)
        // so the first ReAct turn calls `goal_tracker.hydrate_from(desc, &subs)`
        // instead of starting empty. Add a `resumed_objective` handler field + apply
        // it at the tracker-construction site in react_loop/mod.rs.
    }
}
```

> Implementation note for the executor: the exact seed-application point is where `react_loop` builds its `GoalTracker` (`react_loop/mod.rs` `GoalTracker::new()`). Thread the handler's `resumed_objective` down to that construction and call `hydrate_from` once, on the first turn only (guard with a `take()` so subsequent turns' `reset()` behavior is unchanged). Keep this seam minimal — it is the only `core/` edit in the safe slice.

- [ ] **Step 4: Run — expected PASS.** Full: `cargo test -p runtime goal goal_tracker`.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/core/react_loop/goal_tracker.rs crates/runtime/src/impl/daemon/handler/mod.rs
git commit -m "feat(goal): resume persisted active objective into GoalTracker on start"
```

---

## Phase 6 — LATER (design-only): move `decompose` into Brain

> **GATED — do NOT implement in this slice.** This phase relocates decomposition
> out of the `interact` layer into Runtime/Brain. It is a cross-crate boundary
> change; land it only after the safe slice (Phases 1–5) is merged and reviewed.

**Design:** `crates/interact/src/acix/task.rs:204` `decompose` (and `:221`
`decompose_with_llm`) do work that belongs in Brain. Introduce a decomposition
entry on `cognit/src/core/planner.rs` (reuse `generate_multi_step_plan` `:52` /
`parse_subtasks` `:187`) that turns an objective description into ordered
sub-goal strings. Runtime calls it when `goal.set` is issued and persists the
returned sub-goals via `ObjectiveStore::create(.., Some(parent), ..)`.
`interact`'s `decompose` becomes a thin client that calls Runtime over JSON-RPC
(or is deleted once no caller remains — verify callers of `TaskManager::decompose*`
before removing). **Acceptance:** decomposition produces the *same* sub-goals from
Runtime as it does today from Interface (golden test on a fixed input).

## Phase 7 — LATER (design-only): autonomous Goal → Next-Goal loop

> **GATED on Tier 2a `PermissionManager`; DEFAULT-OFF.** No proactivity ships
> without an explicit config flag AND the Runtime permission gate.

**Design:** Add `runtime/src/impl/goal/loop.rs` (a `GoalLoop`) that, when enabled,
advances the active objective: pick the next incomplete sub-goal, run it through
the ReAct loop, mark it `completed`/`failed` via `ObjectiveStore::set_status`, and
stop when all sub-goals resolve (marking the parent `completed`). Wire behind:
(1) a config flag (default `false`), and (2) the Runtime `PermissionManager`
(Tier 2a) checked before each autonomous advance. **Non-goals unchanged:** no goal
*generation* (objectives stay user-set), no multi-objective scheduling.
**Acceptance:** with the flag OFF the loop never advances; with it ON (and
permission granted) the objective advances one sub-goal per cycle and survives a
mid-run restart via `resume()`.

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** persisted `Objective` store (T1–T2) ↔ "persisted `Objective` (id, description, status, parent, created/updated, linked session/scope)"; `goal set/show/status/resume` over JSON-RPC (T4) ↔ spec's exact verb list; CLI subcommand (T5) ↔ "`goal` subcommand"; resume-on-start (T6) ↔ "resume the active objective on daemon start"; back the tracker with the store (T6) ↔ "back the tracker with the persisted store". LATER: move decompose (P6) and gated loop (P7) map to the spec's Tier-2a-gated items and non-goals.
- **Placeholder scan:** none in the safe slice (P1–P5) — every task has real Rust + exact `cargo`/socket commands. P6–P7 are explicitly design-only and marked GATED.
- **Type consistency:** `ObjectiveRow` fields ↔ `map_objective_row` positional indices 0–7 ↔ the `COLS` SELECT order (identical string reused in `get`/`list`/`active`/`sub_goals`). `ObjectiveRow: Serialize` (T1) → `json!` in RPC works. `GoalStatus` values (`in_progress`/`completed`/`failed`/`adjusted`) mirror the `CHECK` constraint and `goal_tracker.rs:46`.
- **Anchor drift fixed:** package names are `runtime`/`interact` (not `aletheon-runtime`/`aletheon` as the governed-memory sibling wrote); commands corrected throughout.

## Risks / notes for the implementer

- **Store placement deviates from the spec's literal path.** The roadmap says
  `runtime/src/core/goal/`, but every rusqlite store in `runtime` lives under
  `impl/` (verified: `impl/memory/fact_store`, `impl/session/store.rs`,
  `impl/session/journal.rs`; `core/**` uses no rusqlite). This plan puts the DB
  layer under `impl/goal/` and keeps the only `core/` touch to the
  `goal_tracker.rs` hydrate seam. If a reviewer insists on the literal path, move
  the module but keep rusqlite out of the hot `core` reasoning path.
- **`rusqlite::Connection` is not `Send`** (noted at `impl/session/session_manager.rs`).
  `ObjectiveStore` is therefore held as `Arc<Mutex<_>>` and every RPC arm locks it
  before use — mirror the `fact_store` locking exactly; never hold the lock across
  an `.await` that itself needs the store.
- **Column order is load-bearing.** `map_objective_row` reads by index; every
  SELECT must use the shared `COLS` constant. Do not `SELECT *`.
- **Startup seed is the subtle part (T6).** The tracker is built inside
  `react_loop` and `reset()` each turn. Apply `hydrate_from` once (guard with
  `take()`), on the first turn only, so per-turn reset semantics are unchanged.
  If threading `resumed_objective` into `react_loop/mod.rs` proves invasive, an
  acceptable fallback is to seed via the existing `set_goal` path on the first
  chat turn — but verify the call site before choosing.
- **`send_rpc` is private in `debug.rs`** (`:1194`); the CLI handler ships its own
  small copy (`goal_send_rpc`) to avoid changing `debug.rs` visibility. If a later
  change makes it `pub(crate)`, collapse the duplicate.
- **Socket smoke tests** need a running daemon with a valid provider (see Tier 0
  `config/default.toml` fix); not required for `cargo test` unit coverage.
- **P6/P7 are OUT OF SCOPE here.** Moving `decompose` is a cross-crate boundary
  change; the autonomous loop must stay behind a default-off flag AND the Tier 2a
  `PermissionManager`. Do not begin either until the safe slice is merged.
- **No autonomous goal generation** in this MVP — objectives are user-set via
  `goal set` / `goal.set`. Keep it that way until P7's gate exists.
