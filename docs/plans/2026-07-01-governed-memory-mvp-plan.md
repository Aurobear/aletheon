# Governed Memory MVP — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make the live `FactStore` a *governed* memory store — add scope/source/status/pinned/subject, scope-aware retrieval, secret-safety on write, and a user-facing `memory` CLI.

**Architecture:** Target the store the daemon actually uses (`crates/runtime/src/impl/memory/fact_store/`), NOT the unused cognitive `memory` crate (see design §0). Reuse existing `trust_score`/`ttl_days`/`tier`/`tags`/FTS5/`decay_stale`. Add columns via idempotent migration. Expose management through the daemon's untyped JSON-RPC (`method` string) + a new `interact` subcommand.

**Tech Stack:** Rust, `rusqlite` (SQLite + FTS5), `tokio`, `serde_json`, `clap`, `regex`.

**Spec:** `docs/plans/2026-07-01-governed-memory-mvp-design.md`

**Branch:** `auro/feat/20260701-aletheon-governed-memory` (own branch per repo policy).

---

## File map

| File | Change |
|---|---|
| `crates/runtime/src/impl/memory/fact_store/mod.rs` | migration, `FactRow` fields, `map_fact_row`, safety helper, pin/archive |
| `crates/runtime/src/impl/memory/fact_store/query.rs` | governed `add_fact`, scope/status/ttl filters, `set_pinned`/`set_status` |
| `crates/runtime/src/impl/daemon/handler/rpc.rs` | `memory.*` JSON-RPC arms |
| `crates/interact/src/tui/cli.rs` | `Memory` subcommand + `send_rpc` calls |

Each phase ends with a build + commit. Run `cargo build -p aletheon-runtime` and `cargo test -p aletheon-runtime fact_store` unless noted.

---

## Phase 1 — Schema migration & FactRow

### Task 1: Idempotent column migration on `facts`

**Files:**
- Modify: `crates/runtime/src/impl/memory/fact_store/mod.rs` (add `migrate_facts_table`, call it in `open` after `create_schema`, ~mod.rs:100)
- Test: same file `#[cfg(test)]`

- [ ] **Step 1: Write the failing test**

```rust
// in crates/runtime/src/impl/memory/fact_store/mod.rs tests module
#[test]
fn migration_is_idempotent_and_adds_columns() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("f.db");
    // open twice — migration must not error on second run
    { let _ = FactStore::open(&path).unwrap(); }
    let fs = FactStore::open(&path).unwrap();
    let cols: Vec<String> = fs.db
        .prepare("PRAGMA table_info(facts)").unwrap()
        .query_map([], |r| r.get::<_, String>(1)).unwrap()
        .map(|c| c.unwrap()).collect();
    for c in ["scope", "source", "status", "pinned", "subject"] {
        assert!(cols.contains(&c.to_string()), "missing column {c}");
    }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `cargo test -p aletheon-runtime fact_store::tests::migration_is_idempotent_and_adds_columns`
Expected: FAIL (`migrate_facts_table` not defined / columns missing).

- [ ] **Step 3: Add the migration function and call it**

```rust
// mod.rs — inside impl FactStore, near create_schema
/// Add governance columns if missing. Idempotent (guarded by PRAGMA table_info).
fn migrate_facts_table(db: &Connection) -> Result<()> {
    let existing: Vec<String> = db
        .prepare("PRAGMA table_info(facts)")?
        .query_map([], |r| r.get::<_, String>(1))?
        .collect::<std::result::Result<_, _>>()?;
    let add = |name: &str, ddl: &str| -> Result<()> {
        if !existing.iter().any(|c| c == name) {
            db.execute_batch(ddl)?;
        }
        Ok(())
    };
    add("scope",   "ALTER TABLE facts ADD COLUMN scope TEXT NOT NULL DEFAULT 'session';")?;
    add("source",  "ALTER TABLE facts ADD COLUMN source TEXT NOT NULL DEFAULT 'conversation';")?;
    add("status",  "ALTER TABLE facts ADD COLUMN status TEXT NOT NULL DEFAULT 'active';")?;
    add("pinned",  "ALTER TABLE facts ADD COLUMN pinned INTEGER NOT NULL DEFAULT 0;")?;
    add("subject", "ALTER TABLE facts ADD COLUMN subject TEXT NOT NULL DEFAULT '';")?;
    db.execute_batch("CREATE INDEX IF NOT EXISTS idx_facts_scope ON facts(scope);")?;
    db.execute_batch("CREATE INDEX IF NOT EXISTS idx_facts_status ON facts(status);")?;
    Ok(())
}
```

```rust
// mod.rs — in open(), right after Self::create_schema(&db)?;
Self::migrate_facts_table(&db)?;
```

- [ ] **Step 4: Run test to verify it passes**

Run: `cargo test -p aletheon-runtime fact_store::tests::migration_is_idempotent_and_adds_columns`
Expected: PASS.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/memory/fact_store/mod.rs
git commit -m "feat(memory): add governance columns to facts table (idempotent migration)"
```

### Task 2: Extend `FactRow` + `map_fact_row` + SELECT lists

**Files:**
- Modify: `crates/runtime/src/impl/memory/fact_store/mod.rs:22-36` (`FactRow`), `mod.rs:250` (`map_fact_row`)
- Modify: `crates/runtime/src/impl/memory/fact_store/query.rs` (`search_facts` two SQL strings :66/:78, `get_fact` :168) — append the 5 new columns to each SELECT in fixed order.

- [ ] **Step 1: Write the failing test**

```rust
#[test]
fn get_fact_returns_governance_defaults() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FactStore::open(&dir.path().join("f.db")).unwrap();
    let id = fs.add_fact("the sky is blue", "general", "", "", 0.5, "episodic", 0).unwrap();
    let row = fs.get_fact(id).unwrap().unwrap();
    assert_eq!(row.scope, "session");
    assert_eq!(row.source, "conversation");
    assert_eq!(row.status, "active");
    assert!(!row.pinned);
}
```

- [ ] **Step 2: Run test — expected FAIL** (`FactRow` has no field `scope`).

Run: `cargo test -p aletheon-runtime fact_store::tests::get_fact_returns_governance_defaults`

- [ ] **Step 3: Implement**

```rust
// mod.rs FactRow — append fields (keep existing 12 first, in order)
pub struct FactRow {
    pub fact_id: i64,
    pub content: String,
    pub category: String,
    pub tags: String,
    pub source_path: String,
    pub trust_score: f64,
    pub retrieval_count: i64,
    pub helpful_count: i64,
    pub tier: String,
    pub ttl_days: i64,
    pub created_at: String,
    pub updated_at: String,
    // governance (indices 12..=16)
    pub scope: String,
    pub source: String,
    pub status: String,
    pub pinned: bool,
    pub subject: String,
}
```

```rust
// mod.rs map_fact_row — append after updated_at (index 11)
        scope: row.get(12)?,
        source: row.get(13)?,
        status: row.get(14)?,
        pinned: row.get::<_, i64>(15)? != 0,
        subject: row.get(16)?,
```

In `query.rs`, append `f.scope, f.source, f.status, f.pinned, f.subject` (or bare
names in `get_fact`) to the end of the three SELECT column lists (search_facts
:66 and :78, get_fact :168), preserving order so positions 12–16 match.

- [ ] **Step 4: Run test — expected PASS.** Also run full module: `cargo test -p aletheon-runtime fact_store`.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/memory/fact_store/
git commit -m "feat(memory): surface governance columns in FactRow and SELECTs"
```

---

## Phase 2 — Governed write path + secret safety

### Task 3: `add_fact_governed` + secret-safety helper

**Files:**
- Modify: `crates/runtime/src/impl/memory/fact_store/mod.rs` (add `is_sensitive`)
- Modify: `crates/runtime/src/impl/memory/fact_store/query.rs` (add `add_fact_governed`)

- [ ] **Step 1: Write the failing tests**

```rust
#[test]
fn rejects_secrets_unless_explicit() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FactStore::open(&dir.path().join("f.db")).unwrap();
    // conversation source: secret content rejected
    let err = fs.add_fact_governed(
        "my key is sk-abcdefghijklmnopqrstuvwx", "general", "", "project",
        "conversation", "", 0.5, "episodic", 0);
    assert!(err.is_err());
    // explicit source: allowed
    let ok = fs.add_fact_governed(
        "my key is sk-abcdefghijklmnopqrstuvwx", "general", "", "project",
        "explicit", "", 0.5, "episodic", 0);
    assert!(ok.is_ok());
}
```

- [ ] **Step 2: Run — expected FAIL** (`add_fact_governed` undefined).

- [ ] **Step 3: Implement**

```rust
// mod.rs — free fn or assoc fn
pub(crate) fn is_sensitive(content: &str) -> bool {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| Regex::new(
        r#"(?i)(sk-[a-z0-9]{16,}|api[_-]?key\s*[:=]|password\s*[:=]|bearer\s+[a-z0-9._-]{16,}|-----BEGIN [A-Z ]+PRIVATE KEY-----)"#
    ).unwrap());
    re.is_match(content)
}
```

```rust
// query.rs — governed insert; delegates to the same INSERT plus governance cols
#[allow(clippy::too_many_arguments)]
pub fn add_fact_governed(
    &self,
    content: &str, category: &str, tags: &str, scope: &str,
    source: &str, subject: &str, trust: f64, tier: &str, ttl_days: i64,
) -> Result<i64> {
    if source != "explicit" && super::is_sensitive(content) {
        anyhow::bail!("refused to store likely-sensitive content (source={source})");
    }
    self.db.execute(
        "INSERT OR IGNORE INTO facts
           (content, category, tags, source_path, trust_score, tier, ttl_days, scope, source, subject)
         VALUES (?1,?2,?3,'',?4,?5,?6,?7,?8,?9)",
        rusqlite::params![content, category, tags, trust, tier, ttl_days, scope, source, subject],
    )?;
    let fact_id: i64 = self.db.query_row(
        "SELECT fact_id FROM facts WHERE content = ?1",
        rusqlite::params![content], |r| r.get(0))?;
    for entity_name in Self::extract_entities(content) {
        let eid = self.resolve_entity(&entity_name)?;
        self.link_fact_entity(fact_id, eid)?;
    }
    Ok(fact_id)
}
```

- [ ] **Step 4: Run — expected PASS.**

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/impl/memory/fact_store/
git commit -m "feat(memory): governed insert path with secret-safety check"
```

---

## Phase 3 — Governed retrieval + pin/archive

### Task 4: `search_facts_governed` (scope/status/ttl filters + pinned boost)

**Files:** Modify `crates/runtime/src/impl/memory/fact_store/query.rs`.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn governed_search_filters_scope_status_and_expiry() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FactStore::open(&dir.path().join("f.db")).unwrap();
    let keep = fs.add_fact_governed("rust is fast", "general", "", "project", "explicit", "", 0.9, "semantic", 0).unwrap();
    let arch = fs.add_fact_governed("rust is slow", "general", "", "project", "explicit", "", 0.9, "semantic", 0).unwrap();
    fs.set_status(arch, "archived").unwrap();
    let hits = fs.search_facts_governed("rust", Some("project"), false, 0.15, 10).unwrap();
    let ids: Vec<i64> = hits.iter().map(|f| f.fact_id).collect();
    assert!(ids.contains(&keep));
    assert!(!ids.contains(&arch), "archived fact must be excluded");
}
```

- [ ] **Step 2: Run — expected FAIL.**

- [ ] **Step 3: Implement** (mirror `search_facts` :47 but add `scope`/`status`/expiry predicates and order pinned first)

```rust
pub fn search_facts_governed(
    &self, query: &str, scope: Option<&str>, include_archived: bool,
    min_trust: f64, limit: usize,
) -> Result<Vec<FactRow>> {
    if query.trim().is_empty() { return Ok(Vec::new()); }
    let fts = sanitize_fts_query(query);
    let min_trust = if min_trust <= 0.0 { DEFAULT_MIN_TRUST } else { min_trust };
    let mut sql = String::from(
        "SELECT f.fact_id, f.content, f.category, f.tags, f.source_path,
                f.trust_score, f.retrieval_count, f.helpful_count,
                f.tier, f.ttl_days, f.created_at, f.updated_at,
                f.scope, f.source, f.status, f.pinned, f.subject
         FROM facts f INNER JOIN facts_fts fts ON f.fact_id = fts.rowid
         WHERE facts_fts MATCH ?1 AND f.trust_score >= ?2
           AND (f.ttl_days = 0 OR f.created_at >= datetime('now', '-' || f.ttl_days || ' days'))");
    if !include_archived { sql.push_str(" AND f.status = 'active'"); }
    if scope.is_some() { sql.push_str(" AND f.scope = ?3"); }
    sql.push_str(" ORDER BY f.pinned DESC, rank LIMIT ?LIM");
    let sql = sql.replace("?LIM", if scope.is_some() { "?4" } else { "?3" });
    let mut stmt = self.db.prepare(&sql)?;
    let rows = if let Some(s) = scope {
        stmt.query_map(rusqlite::params![fts, min_trust, s, limit as i64], Self::map_fact_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    } else {
        stmt.query_map(rusqlite::params![fts, min_trust, limit as i64], Self::map_fact_row)?
            .collect::<std::result::Result<Vec<_>, _>>()?
    };
    for f in &rows {
        self.db.execute("UPDATE facts SET retrieval_count = retrieval_count + 1 WHERE fact_id = ?1",
            rusqlite::params![f.fact_id])?;
    }
    Ok(rows)
}
```

- [ ] **Step 4: Run — expected PASS.**
- [ ] **Step 5: Commit** `feat(memory): scope/status/ttl-aware governed search with pinned boost`

### Task 5: `set_pinned` / `set_status` / `list_facts`

**Files:** Modify `crates/runtime/src/impl/memory/fact_store/query.rs`.

- [ ] **Step 1: Failing test**

```rust
#[test]
fn pin_and_list() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FactStore::open(&dir.path().join("f.db")).unwrap();
    let id = fs.add_fact_governed("pin me", "general", "", "global", "explicit", "", 0.5, "semantic", 0).unwrap();
    fs.set_pinned(id, true).unwrap();
    let all = fs.list_facts(None, false, 50).unwrap();
    assert!(all.iter().any(|f| f.fact_id == id && f.pinned));
}
```

- [ ] **Step 2: Run — expected FAIL.**
- [ ] **Step 3: Implement**

```rust
pub fn set_pinned(&self, fact_id: i64, pinned: bool) -> Result<bool> {
    Ok(self.db.execute(
        "UPDATE facts SET pinned = ?1, updated_at = datetime('now') WHERE fact_id = ?2",
        rusqlite::params![pinned as i64, fact_id])? > 0)
}
pub fn set_status(&self, fact_id: i64, status: &str) -> Result<bool> {
    Ok(self.db.execute(
        "UPDATE facts SET status = ?1, updated_at = datetime('now') WHERE fact_id = ?2",
        rusqlite::params![status, fact_id])? > 0)
}
pub fn list_facts(&self, scope: Option<&str>, include_archived: bool, limit: usize) -> Result<Vec<FactRow>> {
    let mut sql = String::from(
        "SELECT fact_id, content, category, tags, source_path, trust_score, retrieval_count,
                helpful_count, tier, ttl_days, created_at, updated_at,
                scope, source, status, pinned, subject FROM facts WHERE 1=1");
    if !include_archived { sql.push_str(" AND status = 'active'"); }
    if scope.is_some() { sql.push_str(" AND scope = ?1"); }
    sql.push_str(&format!(" ORDER BY pinned DESC, updated_at DESC LIMIT {}", limit as i64));
    let mut stmt = self.db.prepare(&sql)?;
    let rows = if let Some(s) = scope {
        stmt.query_map(rusqlite::params![s], Self::map_fact_row)?.collect::<std::result::Result<Vec<_>,_>>()?
    } else {
        stmt.query_map([], Self::map_fact_row)?.collect::<std::result::Result<Vec<_>,_>>()?
    };
    Ok(rows)
}
```

- [ ] **Step 4: Run — expected PASS.**
- [ ] **Step 5: Commit** `feat(memory): pin/status/list_facts management APIs`

---

## Phase 4 — Daemon JSON-RPC surface

### Task 6: `memory.*` RPC arms

**Files:** Modify `crates/runtime/src/impl/daemon/handler/rpc.rs` (add arms in the `match method` block; the handler already holds `self.fact_store: Arc<Mutex<FactStore>>`, see `handler/mod.rs:139`).

- [ ] **Step 1: Add the arms** (follow the `reflect` arm shape at rpc.rs:95 — lock, call, build `json!` result/error)

```rust
"memory.add" => {
    let p = &request["params"];
    let content = p["content"].as_str().unwrap_or("");
    let scope = p["scope"].as_str().unwrap_or("session");
    let subject = p["subject"].as_str().unwrap_or("");
    let tags = p["tags"].as_str().unwrap_or("");
    let fs = self.fact_store.lock().await;
    match fs.add_fact_governed(content, "general", tags, scope, "explicit", subject, 0.7, "semantic", 0) {
        Ok(id) => json!({"jsonrpc":"2.0","id":id_var,"result":{"fact_id":id}}),
        Err(e) => json!({"jsonrpc":"2.0","id":id_var,"error":{"code":-32010,"message":e.to_string()}}),
    }
}
"memory.list" => {
    let p = &request["params"];
    let scope = p["scope"].as_str();
    let fs = self.fact_store.lock().await;
    match fs.list_facts(scope, p["all"].as_bool().unwrap_or(false), 50) {
        Ok(rows) => json!({"jsonrpc":"2.0","id":id_var,"result":{"facts":rows}}),
        Err(e) => json!({"jsonrpc":"2.0","id":id_var,"error":{"code":-32010,"message":e.to_string()}}),
    }
}
"memory.search" => {
    let p = &request["params"];
    let fs = self.fact_store.lock().await;
    match fs.search_facts_governed(p["query"].as_str().unwrap_or(""), p["scope"].as_str(), false, 0.15, 20) {
        Ok(rows) => json!({"jsonrpc":"2.0","id":id_var,"result":{"facts":rows}}),
        Err(e) => json!({"jsonrpc":"2.0","id":id_var,"error":{"code":-32010,"message":e.to_string()}}),
    }
}
"memory.show" => {
    let id = request["params"]["id"].as_i64().unwrap_or(0);
    let fs = self.fact_store.lock().await;
    match fs.get_fact(id) {
        Ok(Some(row)) => json!({"jsonrpc":"2.0","id":id_var,"result":{"fact":row}}),
        Ok(None) => json!({"jsonrpc":"2.0","id":id_var,"error":{"code":-32011,"message":"not found"}}),
        Err(e) => json!({"jsonrpc":"2.0","id":id_var,"error":{"code":-32010,"message":e.to_string()}}),
    }
}
"memory.forget" => {
    let p = &request["params"];
    let id = p["id"].as_i64().unwrap_or(0);
    let hard = p["hard"].as_bool().unwrap_or(false);
    let fs = self.fact_store.lock().await;
    let res = if hard { fs.delete_fact(id) } else { fs.set_status(id, "archived") };
    match res {
        Ok(ok) => json!({"jsonrpc":"2.0","id":id_var,"result":{"ok":ok}}),
        Err(e) => json!({"jsonrpc":"2.0","id":id_var,"error":{"code":-32010,"message":e.to_string()}}),
    }
}
"memory.pin" | "memory.unpin" => {
    let id = request["params"]["id"].as_i64().unwrap_or(0);
    let fs = self.fact_store.lock().await;
    match fs.set_pinned(id, method == "memory.pin") {
        Ok(ok) => json!({"jsonrpc":"2.0","id":id_var,"result":{"ok":ok}}),
        Err(e) => json!({"jsonrpc":"2.0","id":id_var,"error":{"code":-32010,"message":e.to_string()}}),
    }
}
```

> Note: `FactRow` already derives `Serialize` (mod.rs:22), so `json!({"facts": rows})` works. Bind the incoming id once: `let id_var = id.clone();` at the top of `handle_rpc` if not already in scope (the existing arms use the `id` param directly — reuse it).

- [ ] **Step 2: Build** `cargo build -p aletheon-runtime` — expected: compiles.
- [ ] **Step 3: Manual smoke** (daemon running): 
  `echo '{"jsonrpc":"2.0","id":1,"method":"memory.add","params":{"content":"aletheon uses FactStore","scope":"project"}}' | nc -U /run/aletheond/aletheond.sock`
  Expected: `{"result":{"fact_id":<n>}}`. Then `memory.search` with `{"query":"FactStore"}` returns it.
- [ ] **Step 4: Commit** `feat(daemon): memory.* JSON-RPC (add/list/search/show/forget/pin)`

---

## Phase 5 — `interact` CLI subcommand

### Task 7: `memory` subcommand

**Files:** Modify `crates/interact/src/tui/cli.rs` (variant in `Command` :69, arm in `handle_command` :155, new async handlers using the `send_rpc` pattern from `debug.rs:1194`).

- [ ] **Step 1: Add the subcommand enum + variant**

```rust
// cli.rs — new enum near Command
#[derive(clap::Subcommand)]
pub enum MemoryAction {
    /// Save a fact: memory add "text" [--scope project]
    Add { text: String, #[arg(long, default_value = "session")] scope: String,
          #[arg(long, default_value = "")] subject: String },
    /// List facts
    List { #[arg(long)] scope: Option<String>, #[arg(long)] all: bool },
    /// Search facts
    Search { query: String, #[arg(long)] scope: Option<String> },
    /// Show one fact by id
    Show { id: i64 },
    /// Forget (archive, or --hard to delete)
    Forget { id: i64, #[arg(long)] hard: bool },
    /// Pin / unpin
    Pin { id: i64 },
    Unpin { id: i64 },
}

// add to enum Command:
    /// Governed memory management
    Memory { #[command(subcommand)] action: MemoryAction },
```

- [ ] **Step 2: Add the dispatch arm + handler**

```rust
// cli.rs handle_command match:
    Some(Command::Memory { action }) => memory_cmd(&args.socket, action).await,
```

```rust
// cli.rs — new async fn, using send_rpc pattern (copy the helper from debug.rs or call it if pub)
async fn memory_cmd(socket: &std::path::Path, action: MemoryAction) -> anyhow::Result<()> {
    let req = match &action {
        MemoryAction::Add { text, scope, subject } => serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"memory.add",
            "params":{"content":text,"scope":scope,"subject":subject}}),
        MemoryAction::List { scope, all } => serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"memory.list","params":{"scope":scope,"all":all}}),
        MemoryAction::Search { query, scope } => serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"memory.search","params":{"query":query,"scope":scope}}),
        MemoryAction::Show { id } => serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"memory.show","params":{"id":id}}),
        MemoryAction::Forget { id, hard } => serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"memory.forget","params":{"id":id,"hard":hard}}),
        MemoryAction::Pin { id } => serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"memory.pin","params":{"id":id}}),
        MemoryAction::Unpin { id } => serde_json::json!({
            "jsonrpc":"2.0","id":1,"method":"memory.unpin","params":{"id":id}}),
    };
    let resp = send_rpc(socket, &req).await?;   // reuse debug::send_rpc (make it pub(crate)) or inline
    if let Some(facts) = resp["result"]["facts"].as_array() {
        for f in facts {
            println!("[{}] ({}/{}) {}", f["fact_id"], f["scope"].as_str().unwrap_or("?"),
                     f["status"].as_str().unwrap_or("?"), f["content"].as_str().unwrap_or(""));
        }
    } else {
        println!("{}", serde_json::to_string_pretty(&resp["result"]).unwrap_or_default());
    }
    Ok(())
}
```

> `send_rpc` currently lives in `debug.rs` (`async fn send_rpc`, debug.rs:1194). Make it `pub(crate)` and import, or copy the 15-line helper into `cli.rs`.

- [ ] **Step 3: Build** `cargo build -p aletheon` — expected: compiles.
- [ ] **Step 4: End-to-end** (daemon running):
  `aletheon memory add "aletheon uses FactStore" --scope project` → prints `fact_id`.
  `aletheon memory search "FactStore"` → lists it.
  `aletheon memory pin <id>` then `aletheon memory list` → pinned first.
  `aletheon memory forget <id>` → `memory list` no longer shows it; `--all` still does.
- [ ] **Step 5: Commit** `feat(cli): aletheon memory add/list/search/show/forget/pin`

---

## Phase 6 — Scope-aware injection (optional within MVP)

### Task 8: pass current scope to FactStore recall in chat

**Files:** Modify `crates/runtime/src/impl/daemon/handler/chat.rs:113-146` (the FactStore recall/injection block) to call `search_facts_governed(&query, Some(current_scope), false, 0.15, 4)` instead of `search_facts`, where `current_scope` is derived from session/project context (default `"session"`).

- [ ] **Step 1:** Replace the `fs.search_facts(&query, None, 0.15, 4)` call (chat.rs:121) with the governed variant, passing the session/project scope.
- [ ] **Step 2:** Build `cargo build -p aletheon-runtime`.
- [ ] **Step 3:** Manual: a `project`-scoped fact is injected on a project-context turn; a `session`-scoped fact from another session is not.
- [ ] **Step 4: Commit** `feat(memory): scope-filtered fact injection in chat`

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** schema (T1–T2), explicit save/forget (T3, T6/T7 forget), safety (T3), task-relevant retrieval + scope (T4, T8), pin/status (T5), CLI (T6–T7). Layered injection = T8 (scope tier); full always/task/optional tiering deferred per design non-goals.
- **Placeholder scan:** none — every task has real code + exact commands.
- **Type consistency:** `FactRow` fields ↔ `map_fact_row` positional indices 12–16 ↔ SELECT column order verified across `search_facts`, `search_facts_governed`, `list_facts`, `get_fact`. `FactRow: Serialize` confirmed (mod.rs:22).

## Risks / notes for the implementer

- **Column order is load-bearing:** `map_fact_row` reads by index. Every SELECT that feeds it MUST list the 12 original columns first, then `scope, source, status, pinned, subject`. `SELECT *` is NOT safe here (column order after ALTER is append-order, which happens to match — but prefer explicit lists).
- **`add_fact` (legacy) still works** — it doesn't set governance columns, so they take table defaults. Leave it for existing callers; new writes use `add_fact_governed`.
- **Socket smoke test** needs the daemon running against a config with a provider (see Tier 0 `config/default.toml` fix) — not required for `cargo test` unit coverage.
- Does not touch the cognitive `memory` crate (M-H).
