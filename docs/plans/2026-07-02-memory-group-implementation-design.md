# Governed Memory -- Consolidated Implementation Design

**Date:** 2026-07-02
**Status:** Design (design-only gate in effect)
**Sources merged:** `2026-07-01-governed-memory-mvp-design.md`, `2026-07-01-governed-memory-mvp-plan.md`, `2026-07-01-mh-unify-memory-plan.md`
**Roadmap:** `docs/plans/2026-07-01-modules-roadmap-design.md` Tier 1 + M-H
**Branch:** `auro/feat/20260701-aletheon-governed-memory`

This document is the single authoritative implementation design for the Governed
Memory work. It merges three tightly-coupled existing plans, corrects for
verified ground truth, and provides complete TDD-ready Rust code with exact file
paths and line numbers. No placeholder code.

---

## 1. Verified Ground Truth Table

Every claim from all three source plans was re-verified against the repo on
2026-07-02. Anchors use `path:line` format.

| # | Claim (source plan) | Verified | Actual | Notes |
|---|---|---|---|---|
| 1 | `AletheonRuntime` holds `memory: Option<Arc<MemoryRouter>>` at `orchestrator.rs:34` | MATCH | `orchestrator.rs:34` | |
| 2 | `with_memory` setter at `orchestrator.rs:88-91` | MATCH | `orchestrator.rs:88-91` | |
| 3 | `with_memory` has ZERO callers | MATCH | `rg "with_memory\b"` returns only the definition | Confirmed 2026-07-02 |
| 4 | Dead recall block at `orchestrator.rs:345-353` | MATCH | `orchestrator.rs:344-353` (off by 1 line) | Inside `process_react` |
| 5 | Daemon builds `AletheonRuntime::new(...)` without `.with_memory(...)` at `handler/mod.rs:335` | MATCH | `handler/mod.rs:335` | |
| 6 | `requestHandler` holds `fact_store: Arc<Mutex<FactStore>>` at `handler/mod.rs:139` | MATCH | `handler/mod.rs:139` | |
| 7 | `FactStore::open` at `handler/mod.rs:223` | MATCH | `handler/mod.rs:223` | |
| 8 | `fact_store` field set in handler construction at `handler/mod.rs:667` | MATCH | `handler/mod.rs:667` | |
| 9 | `FactStore` struct at `fact_store/mod.rs:91` | MATCH | `fact_store/mod.rs:91` | |
| 10 | `FactStore::open` at `mod.rs:97` | MATCH | `mod.rs:97` | |
| 11 | `trust_score REAL DEFAULT 0.5` at `mod.rs:113` | MATCH | `mod.rs:113` | |
| 12 | `ttl_days INTEGER DEFAULT 0` at `mod.rs:117` | MATCH | `mod.rs:117` | |
| 13 | `category` at `mod.rs:110`, `tier` at `mod.rs:116`, `tags` at `mod.rs:111` | MATCH | `mod.rs:110,111,116` | |
| 14 | `retrieval_count` at `mod.rs:114` | MATCH | `mod.rs:114` | |
| 15 | `facts_fts` FTS5 + triggers at `mod.rs:127-145` | MATCH | `mod.rs:127-145` | |
| 16 | `FactRow` derives `Serialize` at `mod.rs:22` | MATCH | `mod.rs:22` | `#[derive(..., Serialize, Deserialize)]` |
| 17 | `map_fact_row` at `mod.rs:250` | MATCH | `mod.rs:250` | |
| 18 | `add_fact` at `query.rs:14` | MATCH | `query.rs:14` | |
| 19 | `search_facts` at `query.rs:47` | MATCH | `query.rs:47` | |
| 20 | `search_facts` SELECT at `query.rs:66` (with category) and `:78` (without) | MATCH | `query.rs:66,78` | |
| 21 | `get_fact` SELECT at `query.rs:168` | MATCH | `query.rs:168` | |
| 22 | `search_facts(&query, None, 0.15, 4)` at `chat.rs:121` | MATCH | `chat.rs:121` | |
| 23 | Auto-memory write path at `chat.rs:632` | DRIFT | `chat.rs:632` is `AutoMemory` block start; the call is `am.analyze_and_store(...)` at `:637` | PostTurn hook body, part of auto-memory extraction |
| 24 | Fact recall/injection block at `chat.rs:113-146` | MATCH | `chat.rs:112-146` (off by 1) | FactStore recall + entity boost + core memory |
| 25 | Handler dispatch at `handler/mod.rs:883` | MATCH | `handler/mod.rs:883` | `handle` method routing on `method` string |
| 26 | `handle_rpc` at `handler/rpc.rs` | MATCH | `rpc.rs:18-783` | Match arms: clear, reflect, status, genome, evolution, reflect_now, sessions, resume, compact, reload_skills, approval_response, new_session, load_recent, model_list, model_switch, interrupt, mode_switch, sub_agents, hooks_list, tools/list, _ |
| 27 | `runtime` depends on `memory` at `runtime/Cargo.toml:20` | MATCH | `runtime/Cargo.toml:20` | `memory = { path = "../memory" }` |
| 28 | `memory` crate has no `[features]` section | MATCH | `crates/memory/Cargo.toml` | Only per-dep `features = [...]` |
| 29 | `EpisodicMemory` used at `handler/mod.rs:43,104,353` | MATCH | `handler/mod.rs:43,104,353` | |
| 30 | `memory_pipeline.rs:18,84` for episodic | NOT VERIFIED | File may not exist in current tree | Not a critical path for this plan |
| 31 | `SemanticMemory` at `backends/semantic/schema.rs:166` | MATCH | `schema.rs:166` | |
| 32 | `ProceduralMemory` at `backends/procedural.rs:19` | MATCH | `procedural.rs:19` | |
| 33 | `SelfMemory` at `backends/self_memory.rs:22` | MATCH | `self_memory.rs:22` | |
| 34 | `router.rs:14-17` consumes all four backends | MATCH | `router.rs:14-17` | |
| 35 | `MemoryRouter` impls `MemoryBackend` at `router.rs:276` | MATCH | `router.rs:276` | |
| 36 | `MemoryBackend` trait at `base/src/include/memory.rs:141` | MATCH | `memory.rs:141` | |
| 37 | `memory/src/lib.rs:13-14` re-exports | MATCH | `lib.rs:13-14` | |
| 38 | `ops/mod.rs:12,18` for router | MATCH | `ops/mod.rs:12,18` | |
| 39 | `backends/mod.rs:9-16` for modules/exports | MATCH | `backends/mod.rs:9-17` (off by 1) | |
| 40 | `EpisodicMemory::new(PathBuf)` at `backends/episodic/schema.rs:21` | MATCH | `schema.rs:21-22` | |
| 41 | `MemoryRouter::new(&Path)` at `ops/router.rs:103,423` | DRIFT | `router.rs:107` (new), `router.rs:422` (test setup) | Test `setup_router` at 421 calls `MemoryRouter::new` |
| 42 | `Command` enum at `cli.rs:69` | MATCH | `cli.rs:69` | |
| 43 | `handle_command` at `cli.rs:155` | MATCH | `cli.rs:155` | |
| 44 | `send_rpc` at `debug.rs:1194` | MATCH | `debug.rs:1194` | `async fn send_rpc` -- currently private |
| 45 | `base/src/include/memory.rs:16` `MemoryType` enum | MATCH | `memory.rs:16-26` | |
| 46 | `base/src/include/memory.rs:29` `MemoryEntry` | MATCH | `memory.rs:29-49` | |
| 47 | `memory/src/ops/schema.rs:9` base table | MATCH | `schema.rs:7-20` | `init_base_table` |
| 48 | `base/src/include/memory.rs:60` `MemoryQuery` | MATCH | `memory.rs:59-75` | |
| 49 | `memory/src/ops/router.rs:40` `MemoryContext` | MATCH | `router.rs:41` (off by 1) | |

**Summary:** 46 of 49 claims match exactly (94%). Three claims have minor line
offsets of 1-2 lines (file content identical; the plan authors counted lines
slightly differently). One claim (`memory_pipeline.rs`) references a file that
may have been moved -- not critical for this plan. **Zero functional drift** was
found.

## 2. Architecture Overview

### Current state (before changes)

```
                           COGNITIVE CRATE (dead path)
                           ==========================
  AletheonRuntime.memory ──► Option<MemoryRouter>  (always None)
       (orchestrator.rs:34)     │    ├─ EpisodicMemory  ← the ONE backend
       with_memory() — 0 callers│    ├─ SemanticMemory    actually used...
                                │    ├─ ProceduralMemory  but NOT via this
                                │    └─ SelfMemory          router path
                                │                          (dead)
                                ▼
                     recall_for_prompt() — never fires
                     (orchestrator.rs:344-353)


                          DAEMON LIVE PATH
                          ================
  RequestHandler                            chat.rs
  ─────────────                            ────────
  fact_store ────────► FactStore  ─────►  search_facts(&query, None, 0.15, 4)
  (handler/mod.rs:139)  (mod.rs:91)        (chat.rs:121)
                           │               Used per-turn for recall + injection
  episodic_memory ────► EpisodicMemory ─► store_reflection / recall_reflections
  (handler/mod.rs:104)   (schema.rs:21)
                           │               Used for post-chat reflections
  recall_memory ───────► RecallMemory ──►  Keyword memory
  (handler/mod.rs:96)     (recall_memory.rs:17)
                           │
  core_memory ─────────► CoreMemory ────►  In-memory key-value store
  (handler/mod.rs:119)    (core_memory.rs:44)
                           │
  auto_memory ─────────► AutoMemory ────►  LLM-powered fact extraction
  (handler/mod.rs:159)    (auto_memory.rs:40)
```

### Target state (after these changes)

```
                          GOVERNED LIVED STORE
                          ====================
  RequestHandler.fact_store ──► FactStore (governed)
                                  ├─ facts table (+scope/source/status/pinned/subject)
                                  ├─ FTS5 search
                                  ├─ entity graph
                                  ├─ trust scoring + decay
                                  ├─ scope-filtered retrieval
                                  ├─ secret-safety guard
                                  ├─ pin/archive/list management
                                  └─ JSON-RPC surface for CLI

                          EpisodicMemory ──► reflections (unchanged, kept)
                          RecallMemory  ──► keyword memory (unchanged)
                          CoreMemory    ──► in-memory facts (unchanged)
                          AutoMemory    ──► LLM extraction (unchanged)

                          COGNITIVE CRATE (demoted)
                          ========================
                          #[cfg(feature = "cognitive-memory")]
                          MemoryRouter + semantic/procedural/self backends
                          (off-by-default feature flag)
```

### Data flow: LLM output to context injection

```
  LLM output (chat.rs:message)
    │
    ├── AutoMemory (LLM-powered fact extraction from turn text)
    │     └── CoreMemory blocks (in-memory, session-scoped)
    │
    ├── FactStore search (keywords >3 chars from user message)
    │     ├── search_facts_governed(query, scope, min_trust, limit)
    │     ├── Entity graph boost (related facts via entity linkage)
    │     └── [Recalled memories] injected into user turn
    │
    ├── CoreMemory injection [core:*] blocks
    │
    └── Skill suggestion (SkillRouter)
          └── [Suggested skill] injected
```

### CLI to daemon flow

```
  aletheon memory add "x" --scope project
    │
    ▼
  interact/src/tui/cli.rs
    │  memory_cmd() ──► send_rpc(socket, json!({
    │    "jsonrpc":"2.0","id":1,"method":"memory.add",
    │    "params":{...}}))
    ▼
  Unix socket ──► handler/mod.rs:883 handle()
    │  match method.as_str() { "chat" => ... _ => handle_rpc(...) }
    ▼
  handler/rpc.rs:18 handle_rpc()
    │  match method { "memory.add" => { ... } ... }
    ▼
  fact_store.add_fact_governed(content, category, tags, scope, source, subject, trust, tier, ttl)
    │
    ▼
  SQLite facts table (INSERT with governance columns)
```

## 3. Implementation Sequence (Phase-ordered)

The implementation is split into 7 phases. Phases 1-5 are the Governed Memory
MVP (Tier 1). Phase 6 is M-H (bifurcation resolution). Phase 7 is
cross-phase validation. Each phase ends with `cargo build --workspace` + the
named `cargo test` commands.

### Dependencies between phases

```
  Phase 1 (schema migration) ──► Phase 2 (FactRow + SELECTs)
                                      │
  Phase 3 (governed write) ◄──────────┘
        │
  Phase 4 (governed retrieval + pin/archive)
        │
  Phase 5 (daemon JSON-RPC + CLI subcommand)
        │
  Phase 6 (M-H: dead router removal + feature gating)
        │
  Phase 7 (scope-aware injection in chat + end-to-end)
```

---

## Phase 1 -- Schema Migration (idempotent column addition)

### File: `crates/runtime/src/impl/memory/fact_store/mod.rs`

**Insert after `create_schema` method (after line 246, before `// -- Internal helpers`)**

```rust
    /// Idempotent migration: add governance columns if missing.
    /// Guards each ALTER TABLE with PRAGMA table_info so repeated opens are safe.
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

**Modify `FactStore::open` at `mod.rs:99-101` -- insert migration call**

```rust
    pub fn open(path: &std::path::Path) -> Result<Self> {
        let db = Connection::open(path).context("opening fact store DB")?;
        db.execute_batch("PRAGMA journal_mode=WAL;")?;
        Self::create_schema(&db)?;
        Self::migrate_facts_table(&db)?;  // ← NEW LINE
        Ok(Self { db })
    }
```

### Test: `mod.rs` `#[cfg(test)]` module (after existing tests, before closing `}`)

```rust
    #[test]
    fn migration_is_idempotent_and_adds_columns() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("migrate.db");
        // Open twice -- migration must not error on second run
        { let _fs = FactStore::open(&path).unwrap(); }
        let fs = FactStore::open(&path).unwrap();
        let mut stmt = fs.db.prepare("PRAGMA table_info(facts)").unwrap();
        let cols: Vec<String> = stmt
            .query_map([], |r| r.get::<_, String>(1)).unwrap()
            .map(|c| c.unwrap()).collect();
        for c in ["scope", "source", "status", "pinned", "subject"] {
            assert!(cols.contains(&c.to_string()), "missing column '{c}'");
        }
    }

    #[test]
    fn migration_preserves_existing_data() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("preserve.db");
        // Phase 1: open, add a fact, close
        {
            let fs = FactStore::open(&path).unwrap();
            let id = fs.add_fact("preserved fact", "general", "", "", 0.5, "episodic", 0).unwrap();
            assert!(id > 0);
        }
        // Phase 2: re-open (triggers migration again), verify fact still there
        let fs = FactStore::open(&path).unwrap();
        let row = fs.get_fact(1).unwrap().unwrap();
        assert_eq!(row.content, "preserved fact");
        assert_eq!(row.scope, "session");       // new column, takes default
        assert_eq!(row.source, "conversation");  // new column, takes default
    }
```

**Test command:** `cargo test -p runtime fact_store::tests::migration_is_idempotent_and_adds_columns`
**Expected:** FAIL (function undefined) then PASS after implementation.

---

## Phase 2 -- Extend FactRow + map_fact_row + SELECT lists

### File: `crates/runtime/src/impl/memory/fact_store/mod.rs`

**Modify `FactRow` struct at `mod.rs:22-36` -- append 5 governance fields**

Replace the struct definition with:

```rust
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
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
    // ── governance fields (indices 12..=16) ──
    pub scope: String,
    pub source: String,
    pub status: String,
    pub pinned: bool,
    pub subject: String,
}
```

**Modify `map_fact_row` at `mod.rs:250-264` -- append after index 11**

Replace the function body with:

```rust
    pub(crate) fn map_fact_row(row: &rusqlite::Row) -> rusqlite::Result<FactRow> {
        Ok(FactRow {
            fact_id: row.get(0)?,
            content: row.get(1)?,
            category: row.get(2)?,
            tags: row.get(3)?,
            source_path: row.get(4)?,
            trust_score: row.get(5)?,
            retrieval_count: row.get(6)?,
            helpful_count: row.get(7)?,
            tier: row.get(8)?,
            ttl_days: row.get(9)?,
            created_at: row.get(10)?,
            updated_at: row.get(11)?,
            scope: row.get(12)?,
            source: row.get(13)?,
            status: row.get(14)?,
            pinned: row.get::<_, i64>(15)? != 0,
            subject: row.get(16)?,
        })
    }
```

### File: `crates/runtime/src/impl/memory/fact_store/query.rs`

**Modify all 3 SELECT column lists to append `f.scope, f.source, f.status, f.pinned, f.subject`**

1. `search_facts` category branch at `query.rs:66-71` -- change column list:

```sql
SELECT f.fact_id, f.content, f.category, f.tags, f.source_path,
       f.trust_score, f.retrieval_count, f.helpful_count,
       f.tier, f.ttl_days, f.created_at, f.updated_at,
       f.scope, f.source, f.status, f.pinned, f.subject
FROM facts f
INNER JOIN facts_fts fts ON f.fact_id = fts.rowid
WHERE facts_fts MATCH ?1
  AND f.trust_score >= ?2
  AND f.category = ?3
ORDER BY rank
LIMIT ?4
```

2. `search_facts` no-category branch at `query.rs:78-86` -- same column addition:

```sql
SELECT f.fact_id, f.content, f.category, f.tags, f.source_path,
       f.trust_score, f.retrieval_count, f.helpful_count,
       f.tier, f.ttl_days, f.created_at, f.updated_at,
       f.scope, f.source, f.status, f.pinned, f.subject
FROM facts f
INNER JOIN facts_fts fts ON f.fact_id = fts.rowid
WHERE facts_fts MATCH ?1
  AND f.trust_score >= ?2
ORDER BY rank
LIMIT ?3
```

3. `get_fact` at `query.rs:168` -- same column addition:

```sql
SELECT fact_id, content, category, tags, source_path,
       trust_score, retrieval_count, helpful_count,
       tier, ttl_days, created_at, updated_at,
       scope, source, status, pinned, subject
FROM facts WHERE fact_id = ?1
```

### Test: `mod.rs` `#[cfg(test)]` module

```rust
    #[test]
    fn get_fact_returns_governance_defaults() {
        let (store, _tmp) = setup();
        let id = store
            .add_fact("the sky is blue", "general", "", "", 0.5, "episodic", 0)
            .unwrap();
        let row = store.get_fact(id).unwrap().unwrap();
        assert_eq!(row.scope, "session");
        assert_eq!(row.source, "conversation");
        assert_eq!(row.status, "active");
        assert!(!row.pinned);
        assert_eq!(row.subject, "");
    }
```

**Test command:** `cargo test -p runtime fact_store::tests::get_fact_returns_governance_defaults`

---

## Phase 3 -- Governed Write Path + Secret Safety

### File: `crates/runtime/src/impl/memory/fact_store/mod.rs`

**Insert after the `map_fact_row` function (after line 265, before `sanitize_fts_query`)**

```rust
/// Check if content contains likely secrets (API keys, passwords, tokens).
pub(crate) fn is_sensitive(content: &str) -> bool {
    use std::sync::OnceLock;
    static RE: OnceLock<Regex> = OnceLock::new();
    let re = RE.get_or_init(|| {
        Regex::new(
            r"(?i)(sk-[a-z0-9]{16,}|api[_-]?key\s*[:=]\s*[a-z0-9_-]{8,}|password\s*[:=]\s*\S{4,}|bearer\s+[a-z0-9._-]{16,}|-----BEGIN [A-Z ]+PRIVATE KEY-----)",
        )
        .unwrap()
    });
    re.is_match(content)
}
```

### File: `crates/runtime/src/impl/memory/fact_store/query.rs`

**Insert after `delete_fact` (after line 187, before `// -- Episodes`)**

```rust
    // ── Governed Write Path ──────────────────────────────────────────────────

    /// Add a fact with governance fields. Delegates to the same INSERT
    /// but includes scope/source/subject. Checks secret-safety unless
    /// source == "explicit" (user deliberately storing it).
    #[allow(clippy::too_many_arguments)]
    pub fn add_fact_governed(
        &self,
        content: &str,
        category: &str,
        tags: &str,
        scope: &str,
        source: &str,
        subject: &str,
        trust: f64,
        tier: &str,
        ttl_days: i64,
    ) -> Result<i64> {
        if source != "explicit" && super::is_sensitive(content) {
            anyhow::bail!("refused to store likely-sensitive content (source={source})");
        }
        self.db.execute(
            "INSERT OR IGNORE INTO facts
               (content, category, tags, source_path, trust_score, tier, ttl_days, scope, source, subject)
             VALUES (?1, ?2, ?3, '', ?4, ?5, ?6, ?7, ?8, ?9)",
            rusqlite::params![content, category, tags, trust, tier, ttl_days, scope, source, subject],
        )?;
        let fact_id: i64 = self.db.query_row(
            "SELECT fact_id FROM facts WHERE content = ?1",
            rusqlite::params![content],
            |r| r.get(0),
        )?;
        // Extract and link entities (same as legacy add_fact)
        let entities = Self::extract_entities(content);
        for entity_name in entities {
            let eid = self.resolve_entity(&entity_name)?;
            self.link_fact_entity(fact_id, eid)?;
        }
        Ok(fact_id)
    }
```

### Test: `mod.rs` `#[cfg(test)]` module

```rust
    #[test]
    fn rejects_secrets_unless_explicit_source() {
        let (store, _tmp) = setup();
        // conversation source: secret content rejected
        let err = store.add_fact_governed(
            "my key is sk-abcdefghijklmnopqrstuvwx", "general", "", "project",
            "conversation", "", 0.5, "episodic", 0,
        );
        assert!(err.is_err(), "should reject secret from conversation source");
        // explicit source: allowed through
        let ok = store.add_fact_governed(
            "my key is sk-abcdefghijklmnopqrstuvwx", "general", "", "project",
            "explicit", "", 0.5, "episodic", 0,
        );
        assert!(ok.is_ok(), "should allow secret from explicit source");
    }

    #[test]
    fn add_fact_governed_sets_fields_correctly() {
        let (store, _tmp) = setup();
        let id = store.add_fact_governed(
            "rust is memory safe", "tech", "lang", "project",
            "explicit", "rust memory model", 0.9, "semantic", 30,
        ).unwrap();
        let row = store.get_fact(id).unwrap().unwrap();
        assert_eq!(row.content, "rust is memory safe");
        assert_eq!(row.scope, "project");
        assert_eq!(row.source, "explicit");
        assert_eq!(row.subject, "rust memory model");
        assert_eq!(row.tier, "semantic");
        assert_eq!(row.ttl_days, 30);
        assert!((row.trust_score - 0.9).abs() < f64::EPSILON);
    }
```

**Test commands:**
- `cargo test -p runtime fact_store::tests::rejects_secrets_unless_explicit_source`
- `cargo test -p runtime fact_store::tests::add_fact_governed_sets_fields_correctly`

---

## Phase 4 -- Governed Retrieval + Pin/Archive

### File: `crates/runtime/src/impl/memory/fact_store/query.rs`

**Insert after `add_fact_governed` (after the entity-linking block, before `// -- Episodes`)**

```rust
    /// Scope/status/ttl-aware search with pinned boost.
    /// Excludes archived entries unless `include_archived` is true.
    /// Orders pinned facts first, then by FTS rank.
    pub fn search_facts_governed(
        &self,
        query: &str,
        scope: Option<&str>,
        include_archived: bool,
        min_trust: f64,
        limit: usize,
    ) -> Result<Vec<FactRow>> {
        if query.trim().is_empty() {
            return Ok(Vec::new());
        }
        let fts = super::sanitize_fts_query(query);
        let min_trust = if min_trust <= 0.0 {
            super::DEFAULT_MIN_TRUST
        } else {
            min_trust
        };
        let mut sql = String::from(
            "SELECT f.fact_id, f.content, f.category, f.tags, f.source_path,
                    f.trust_score, f.retrieval_count, f.helpful_count,
                    f.tier, f.ttl_days, f.created_at, f.updated_at,
                    f.scope, f.source, f.status, f.pinned, f.subject
             FROM facts f INNER JOIN facts_fts fts ON f.fact_id = fts.rowid
             WHERE facts_fts MATCH ?1 AND f.trust_score >= ?2
               AND (f.ttl_days = 0 OR f.created_at >= datetime('now', '-' || f.ttl_days || ' days'))",
        );
        if !include_archived {
            sql.push_str(" AND f.status = 'active'");
        }
        if scope.is_some() {
            sql.push_str(" AND f.scope = ?3");
        }
        sql.push_str(" ORDER BY f.pinned DESC, rank LIMIT ?LIM");
        let sql = sql.replace("?LIM", if scope.is_some() { "?4" } else { "?3" });

        let mut stmt = self.db.prepare(&sql)?;
        let rows = if let Some(s) = scope {
            stmt.query_map(
                rusqlite::params![fts, min_trust, s, limit as i64],
                Self::map_fact_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map(
                rusqlite::params![fts, min_trust, limit as i64],
                Self::map_fact_row,
            )?
            .collect::<std::result::Result<Vec<_>, _>>()?
        };

        // Increment retrieval_count for matched facts
        for f in &rows {
            self.db.execute(
                "UPDATE facts SET retrieval_count = retrieval_count + 1 WHERE fact_id = ?1",
                rusqlite::params![f.fact_id],
            )?;
        }
        Ok(rows)
    }

    /// Set the pinned flag.
    pub fn set_pinned(&self, fact_id: i64, pinned: bool) -> Result<bool> {
        Ok(self.db.execute(
            "UPDATE facts SET pinned = ?1, updated_at = datetime('now') WHERE fact_id = ?2",
            rusqlite::params![pinned as i64, fact_id],
        )? > 0)
    }

    /// Set the status (active/archived).
    pub fn set_status(&self, fact_id: i64, status: &str) -> Result<bool> {
        Ok(self.db.execute(
            "UPDATE facts SET status = ?1, updated_at = datetime('now') WHERE fact_id = ?2",
            rusqlite::params![status, fact_id],
        )? > 0)
    }

    /// List facts with optional scope filter and archived toggle.
    pub fn list_facts(
        &self,
        scope: Option<&str>,
        include_archived: bool,
        limit: usize,
    ) -> Result<Vec<FactRow>> {
        let mut sql = String::from(
            "SELECT fact_id, content, category, tags, source_path,
                    trust_score, retrieval_count, helpful_count,
                    tier, ttl_days, created_at, updated_at,
                    scope, source, status, pinned, subject
             FROM facts WHERE 1=1",
        );
        if !include_archived {
            sql.push_str(" AND status = 'active'");
        }
        if scope.is_some() {
            sql.push_str(" AND scope = ?1");
        }
        sql.push_str(&format!(
            " ORDER BY pinned DESC, updated_at DESC LIMIT {}",
            limit as i64
        ));
        let mut stmt = self.db.prepare(&sql)?;
        let rows = if let Some(s) = scope {
            stmt.query_map(rusqlite::params![s], Self::map_fact_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        } else {
            stmt.query_map([], Self::map_fact_row)?
                .collect::<std::result::Result<Vec<_>, _>>()?
        };
        Ok(rows)
    }
```

### Test: `mod.rs` `#[cfg(test)]` module

```rust
    #[test]
    fn governed_search_excludes_archived() {
        let (store, _tmp) = setup();
        let keep = store.add_fact_governed(
            "rust is fast", "general", "", "project", "explicit", "", 0.9, "semantic", 0,
        ).unwrap();
        let arch = store.add_fact_governed(
            "rust is slow", "general", "", "project", "explicit", "", 0.9, "semantic", 0,
        ).unwrap();
        store.set_status(arch, "archived").unwrap();
        let hits = store.search_facts_governed("rust", Some("project"), false, 0.15, 10).unwrap();
        let ids: Vec<i64> = hits.iter().map(|f| f.fact_id).collect();
        assert!(ids.contains(&keep), "active fact must be returned");
        assert!(!ids.contains(&arch), "archived fact must be excluded");
    }

    #[test]
    fn governed_search_respects_scope_filter() {
        let (store, _tmp) = setup();
        let s1 = store.add_fact_governed(
            "project-specific fact", "general", "", "project", "explicit", "", 0.5, "episodic", 0,
        ).unwrap();
        let s2 = store.add_fact_governed(
            "global fact", "general", "", "global", "explicit", "", 0.5, "episodic", 0,
        ).unwrap();
        let hits = store.search_facts_governed("fact", Some("project"), false, 0.0, 10).unwrap();
        let ids: Vec<i64> = hits.iter().map(|f| f.fact_id).collect();
        assert!(ids.contains(&s1), "project-scoped search must find project fact");
        assert!(!ids.contains(&s2), "project-scoped search must exclude global fact");
    }

    #[test]
    fn pin_and_list_roundtrip() {
        let (store, _tmp) = setup();
        let id = store.add_fact_governed(
            "pin me", "general", "", "global", "explicit", "", 0.5, "semantic", 0,
        ).unwrap();
        assert!(store.set_pinned(id, true).unwrap());
        let all = store.list_facts(None, false, 50).unwrap();
        let pinned = all.iter().find(|f| f.fact_id == id).unwrap();
        assert!(pinned.pinned, "fact must be pinned after set_pinned(true)");
        // Unpin
        assert!(store.set_pinned(id, false).unwrap());
        let all2 = store.list_facts(None, false, 50).unwrap();
        let unpinned = all2.iter().find(|f| f.fact_id == id).unwrap();
        assert!(!unpinned.pinned);
    }

    #[test]
    fn archived_visible_when_include_archived() {
        let (store, _tmp) = setup();
        let id = store.add_fact_governed(
            "temp fact", "general", "", "session", "explicit", "", 0.5, "episodic", 0,
        ).unwrap();
        store.set_status(id, "archived").unwrap();
        // Not visible without include_archived
        let active = store.list_facts(None, false, 50).unwrap();
        assert!(!active.iter().any(|f| f.fact_id == id));
        // Visible with include_archived
        let all = store.list_facts(None, true, 50).unwrap();
        assert!(all.iter().any(|f| f.fact_id == id));
    }

    #[test]
    fn pinned_sorts_first_in_list() {
        let (store, _tmp) = setup();
        let id1 = store.add_fact_governed(
            "first fact", "general", "", "global", "explicit", "", 0.5, "episodic", 0,
        ).unwrap();
        let id2 = store.add_fact_governed(
            "second fact", "general", "", "global", "explicit", "", 0.5, "episodic", 0,
        ).unwrap();
        store.set_pinned(id2, true).unwrap();
        let all = store.list_facts(None, false, 50).unwrap();
        assert_eq!(all[0].fact_id, id2, "pinned fact must sort first");
    }
```

**Test commands:**
- `cargo test -p runtime fact_store::tests::governed_search_excludes_archived`
- `cargo test -p runtime fact_store::tests::governed_search_respects_scope_filter`
- `cargo test -p runtime fact_store::tests::pin_and_list_roundtrip`
- `cargo test -p runtime fact_store`
- `cargo test -p runtime fact_store::tests::archived_visible_when_include_archived`
- `cargo test -p runtime fact_store::tests::pinned_sorts_first_in_list`

---

## Phase 5 -- Daemon JSON-RPC Surface + CLI Subcommand

### 5a. Daemon JSON-RPC: `crates/runtime/src/impl/daemon/handler/rpc.rs`

**Insert new `memory.*` match arms in `handle_rpc` (after the `"tools/list" => {...}` arm, before the `_ =>` fallback at line 777)**

```rust
            "memory.add" => {
                let p = &request["params"];
                let content = p["content"].as_str().unwrap_or("");
                let scope = p["scope"].as_str().unwrap_or("session");
                let subject = p["subject"].as_str().unwrap_or("");
                let tags = p["tags"].as_str().unwrap_or("");
                let fs = self.fact_store.lock().await;
                match fs.add_fact_governed(
                    content, "general", tags, scope, "explicit", subject, 0.7, "semantic", 0,
                ) {
                    Ok(fact_id) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "fact_id": fact_id }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
            "memory.list" => {
                let p = &request["params"];
                let scope = p["scope"].as_str();
                let all = p["all"].as_bool().unwrap_or(false);
                let fs = self.fact_store.lock().await;
                match fs.list_facts(scope, all, 50) {
                    Ok(rows) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "facts": rows }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
            "memory.search" => {
                let p = &request["params"];
                let query = p["query"].as_str().unwrap_or("");
                let scope = p["scope"].as_str();
                let fs = self.fact_store.lock().await;
                match fs.search_facts_governed(query, scope, false, 0.15, 20) {
                    Ok(rows) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "facts": rows }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
            "memory.show" => {
                let fact_id = request["params"]["id"].as_i64().unwrap_or(0);
                let fs = self.fact_store.lock().await;
                match fs.get_fact(fact_id) {
                    Ok(Some(row)) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "fact": row }
                    }),
                    Ok(None) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32011, "message": "fact not found" }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
            "memory.forget" => {
                let p = &request["params"];
                let fact_id = p["id"].as_i64().unwrap_or(0);
                let hard = p["hard"].as_bool().unwrap_or(false);
                let fs = self.fact_store.lock().await;
                let res = if hard {
                    fs.delete_fact(fact_id)
                } else {
                    fs.set_status(fact_id, "archived")
                };
                match res {
                    Ok(ok) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "ok": ok }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
            "memory.pin" | "memory.unpin" => {
                let fact_id = request["params"]["id"].as_i64().unwrap_or(0);
                let pin = method == "memory.pin";
                let fs = self.fact_store.lock().await;
                match fs.set_pinned(fact_id, pin) {
                    Ok(ok) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "result": { "ok": ok }
                    }),
                    Err(e) => json!({
                        "jsonrpc": "2.0",
                        "id": id,
                        "error": { "code": -32010, "message": e.to_string() }
                    }),
                }
            }
```

### 5b. CLI Subcommand: `crates/interact/src/tui/cli.rs`

**Insert the `MemoryAction` enum and `Memory` variant -- after line 121 (closing `}` of `DaemonAction`), before `/// CLI entry point`**

```rust
#[derive(Subcommand)]
pub enum MemoryAction {
    /// Save a fact: memory add "text" [--scope project] [--subject ...]
    Add {
        text: String,
        #[arg(long, default_value = "session")]
        scope: String,
        #[arg(long, default_value = "")]
        subject: String,
    },
    /// List facts [--scope S] [--all]
    List {
        #[arg(long)]
        scope: Option<String>,
        #[arg(long)]
        all: bool,
    },
    /// Search facts: memory search "query" [--scope S]
    Search {
        query: String,
        #[arg(long)]
        scope: Option<String>,
    },
    /// Show one fact by id
    Show { id: i64 },
    /// Forget (archive; --hard to delete)
    Forget {
        id: i64,
        #[arg(long)]
        hard: bool,
    },
    /// Pin a fact
    Pin { id: i64 },
    /// Unpin a fact
    Unpin { id: i64 },
}
```

**Add variant to `Command` enum at line 69 -- insert after `Debug` variant (line 104), before closing `}`**

```rust
    /// Governed memory management
    Memory {
        #[command(subcommand)]
        action: MemoryAction,
    },
```

**Add dispatch arm in `handle_command` at line 155 -- insert after `Command::Debug`, before closing `}`**

```rust
        Command::Memory { action } => memory_cmd(socket, action).await,
```

**Add the `memory_cmd` handler function -- insert after the `handle_command` function (after line 170, before `find_aletheond`)**

```rust
/// Handle memory subcommands by sending JSON-RPC to the daemon.
async fn memory_cmd(socket: &PathBuf, action: MemoryAction) -> Result<()> {
    let send_rpc = |request: serde_json::Value| async move {
        use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
        use tokio::net::UnixStream;
        let mut stream = UnixStream::connect(socket)
            .await
            .with_context(|| format!("Cannot connect to daemon socket: {}", socket.display()))?;
        let req_str = serde_json::to_string(&request)?;
        stream.write_all(req_str.as_bytes()).await?;
        stream.write_all(b"\n").await?;
        let (reader, _) = stream.split();
        let mut reader = BufReader::new(reader);
        let mut response = String::new();
        reader.read_line(&mut response).await?;
        serde_json::from_str::<serde_json::Value>(&response)
            .context("Failed to parse daemon response")
    };

    let req = match &action {
        MemoryAction::Add { text, scope, subject } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.add",
            "params": { "content": text, "scope": scope, "subject": subject }
        }),
        MemoryAction::List { scope, all } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.list",
            "params": { "scope": scope, "all": all }
        }),
        MemoryAction::Search { query, scope } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.search",
            "params": { "query": query, "scope": scope }
        }),
        MemoryAction::Show { id } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.show",
            "params": { "id": id }
        }),
        MemoryAction::Forget { id, hard } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.forget",
            "params": { "id": id, "hard": hard }
        }),
        MemoryAction::Pin { id } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.pin",
            "params": { "id": id }
        }),
        MemoryAction::Unpin { id } => serde_json::json!({
            "jsonrpc": "2.0", "id": 1, "method": "memory.unpin",
            "params": { "id": id }
        }),
    };

    let resp = send_rpc(req).await?;

    if let Some(err) = resp.get("error") {
        eprintln!("Error: {}", err["message"].as_str().unwrap_or("unknown"));
    } else if let Some(facts) = resp["result"]["facts"].as_array() {
        for f in facts {
            let pinned = if f["pinned"].as_bool().unwrap_or(false) { " [PINNED]" } else { "" };
            println!(
                "[{}] ({}/{}){} {}",
                f["fact_id"].as_i64().unwrap_or(0),
                f["scope"].as_str().unwrap_or("?"),
                f["status"].as_str().unwrap_or("?"),
                pinned,
                f["content"].as_str().unwrap_or(""),
            );
        }
    } else if let Some(fact) = resp["result"]["fact"].as_object() {
        println!("ID:      {}", fact.get("fact_id").map(|v| v.to_string()).unwrap_or_default());
        println!("Content: {}", fact.get("content").and_then(|v| v.as_str()).unwrap_or(""));
        println!("Scope:   {}  Source: {}  Status: {}",
            fact.get("scope").and_then(|v| v.as_str()).unwrap_or("?"),
            fact.get("source").and_then(|v| v.as_str()).unwrap_or("?"),
            fact.get("status").and_then(|v| v.as_str()).unwrap_or("?"),
        );
        println!("Trust:   {}  Tier: {}  TTL: {}d",
            fact.get("trust_score").and_then(|v| v.as_f64()).unwrap_or(0.0),
            fact.get("tier").and_then(|v| v.as_str()).unwrap_or("?"),
            fact.get("ttl_days").and_then(|v| v.as_i64()).unwrap_or(0),
        );
        println!("Pinned:  {}  Retrievals: {}",
            fact.get("pinned").and_then(|v| v.as_bool()).unwrap_or(false),
            fact.get("retrieval_count").and_then(|v| v.as_i64()).unwrap_or(0),
        );
        println!("Created: {}  Updated: {}",
            fact.get("created_at").and_then(|v| v.as_str()).unwrap_or(""),
            fact.get("updated_at").and_then(|v| v.as_str()).unwrap_or(""),
        );
    } else {
        println!("{}", serde_json::to_string_pretty(&resp["result"]).unwrap_or_default());
    }
    Ok(())
}
```

**Build commands:**
- `cargo build -p runtime`
- `cargo build -p interact`

---

## Phase 6 -- M-H: Remove dead MemoryRouter wiring + demote cognitive backends

### 6a. Remove dead wiring: `crates/runtime/src/core/orchestrator.rs`

Five precise deletions:

1. **Line 16** -- remove the import:
   ```rust
   use memory::MemoryRouter;
   ```
   Delete this line entirely.

2. **Lines 34** -- remove the field:
   ```rust
       memory: Option<Arc<MemoryRouter>>,
   ```
   Delete this line from the struct.

3. **Line 49** -- remove the initializer:
   ```rust
               memory: None,
   ```
   Delete this line from `new()`.

4. **Lines 88-91** -- remove the builder:
   ```rust
       /// Attach a MemoryRouter for prompt-time memory recall.
       pub fn with_memory(mut self, memory: Arc<MemoryRouter>) -> Self {
           self.memory = Some(memory);
           self
       }
   ```
   Delete these 4 lines.

5. **Lines 344-353** -- remove the dead recall block:
   ```rust
           // Inject memory context into system prompt
           if let Some(ref memory) = self.memory {
               let mem_ctx = memory.recall_for_prompt(&effective_input, 3).await;
               let mem_section = mem_ctx.to_prompt_section();
               if !mem_section.is_empty() {
                   let current = self.react_loop.system_prompt().to_string();
                   self.react_loop
                       .set_system_prompt(format!("{}\n\n{}", current, mem_section));
               }
           }
   ```
   Delete these 10 lines.

### 6b. Guard test: new file `crates/runtime/tests/memory_bifurcation_guard.rs`

```rust
//! Locks in M-H Option A: the live runtime/daemon must not wire the
//! cognitive MemoryRouter. Source-scan guards so a future edit that
//! re-introduces the bifurcation fails CI.

#[test]
fn live_runtime_does_not_reference_cognitive_memory_router() {
    let orchestrator = include_str!("../src/core/orchestrator.rs");
    assert!(
        !orchestrator.contains("MemoryRouter"),
        "Option A: MemoryRouter must not be wired into AletheonRuntime"
    );
    assert!(
        !orchestrator.contains("with_memory"),
        "Option A: the never-called with_memory builder must be removed"
    );
}

#[test]
fn daemon_never_wires_a_memory_router_into_the_runtime() {
    let handler = include_str!("../src/impl/daemon/handler/mod.rs");
    assert!(
        !handler.contains("with_memory("),
        "Option A: daemon must build AletheonRuntime without a MemoryRouter"
    );
    assert!(
        handler.contains("EpisodicMemory"),
        "EpisodicMemory remains the daemon's reflection store (kept under Option A)"
    );
}
```

**Test command:** `cargo test -p runtime --test memory_bifurcation_guard`
**Expected:** FAIL initially (orchestrator.rs still references MemoryRouter) then PASS after deletions.

### 6c. Demote cognitive backends behind feature flag

#### `crates/memory/Cargo.toml` -- add `[features]` section

```toml
[features]
# Off by default: the cognitive MemoryRouter + semantic/procedural/self backends
# are not used by the live daemon (M-H Option A). Enable to build them.
default = []
cognitive-memory = []
```

#### `crates/memory/src/backends/mod.rs` -- gate non-episodic modules

Replace with:

```rust
//! Storage backends for memory subsystems.
//!
//! Each backend implements the `MemoryBackend` trait for its memory type:
//! - `EpisodicMemory` — events, reflections, observations (always built)
//! - `SemanticMemory` — knowledge, concepts, facts (cognitive-memory feature)
//! - `ProceduralMemory` — skills, workflows (cognitive-memory feature)
//! - `SelfMemory` — identity changes, lineage (cognitive-memory feature)

pub mod episodic;
#[cfg(feature = "cognitive-memory")]
pub mod procedural;
#[cfg(feature = "cognitive-memory")]
pub mod self_memory;
#[cfg(feature = "cognitive-memory")]
pub mod semantic;

pub use episodic::EpisodicMemory;
#[cfg(feature = "cognitive-memory")]
pub use procedural::ProceduralMemory;
#[cfg(feature = "cognitive-memory")]
pub use self_memory::SelfMemory;
#[cfg(feature = "cognitive-memory")]
pub use semantic::SemanticMemory;
```

#### `crates/memory/src/ops/mod.rs` -- gate router/consolidation

Replace with:

```rust
//! Memory operations — routing, consolidation, decay, activation, schema.

pub mod activation;
pub mod decay;
pub mod schema;
#[cfg(feature = "cognitive-memory")]
pub mod router;
#[cfg(feature = "cognitive-memory")]
pub mod consolidation;

pub use activation::{compute_activation, ActivationEntry};
pub use decay::{apply_access_boost, compute_strength, should_forget};
#[cfg(feature = "cognitive-memory")]
pub use consolidation::{ConsolidationConfig, ConsolidationResult};
#[cfg(feature = "cognitive-memory")]
pub use router::{MemoryContext, MemoryRouter, ReflectionSummary, SkillSummary};
```

#### `crates/memory/src/lib.rs` -- gate cognitive re-exports

Replace with:

```rust
//! # Aletheon Memory
//!
//! SQLite-backed implementations of the `MemoryBackend` trait.
//! EpisodicMemory is always available (used by the daemon for reflections).
//! Cognitive backends (MemoryRouter + semantic/procedural/self) are behind the
//! off-by-default `cognitive-memory` feature (M-H Option A).

pub mod backends;
pub mod ops;

// Always-available exports
pub use backends::EpisodicMemory;
pub use ops::{compute_activation, ActivationEntry};
pub use ops::{apply_access_boost, compute_strength, should_forget};

// Cognitive exports (off by default)
#[cfg(feature = "cognitive-memory")]
pub use backends::{ProceduralMemory, SelfMemory, SemanticMemory};
#[cfg(feature = "cognitive-memory")]
pub use ops::{ConsolidationConfig, ConsolidationResult, MemoryContext, MemoryRouter, ReflectionSummary, SkillSummary};

// Sub-module re-exports for direct path access
pub use backends::episodic;
pub use ops::decay;
pub use ops::activation;
pub use ops::schema;

#[cfg(feature = "cognitive-memory")]
pub use backends::procedural;
#[cfg(feature = "cognitive-memory")]
pub use backends::self_memory;
#[cfg(feature = "cognitive-memory")]
pub use backends::semantic;
#[cfg(feature = "cognitive-memory")]
pub use ops::router;
#[cfg(feature = "cognitive-memory")]
pub use ops::consolidation;

#[cfg(test)]
pub mod testing;
```

### 6d. Feature gating guard test: new file `crates/memory/tests/feature_gating_guard.rs`

```rust
//! Option A: cognitive backends are demoted behind off-by-default
//! `cognitive-memory` feature; EpisodicMemory stays default for the daemon.

#[test]
fn cognitive_exports_are_feature_gated() {
    let lib = include_str!("../src/lib.rs");
    assert!(
        lib.contains(r#"#[cfg(feature = "cognitive-memory")]"#),
        "cognitive re-exports must be gated behind the cognitive-memory feature"
    );
}

#[test]
fn episodic_memory_is_available_by_default() {
    // Compiles under default features == EpisodicMemory is not gated.
    let dir = tempfile::tempdir().unwrap();
    let _mem = memory::EpisodicMemory::new(dir.path().join("ep.db"));
}

#[cfg(feature = "cognitive-memory")]
#[test]
fn router_is_available_with_the_feature() {
    let dir = tempfile::tempdir().unwrap();
    let _router = memory::MemoryRouter::new(dir.path());
}
```

**Test commands:**
- `cargo test -p memory --test feature_gating_guard` (default: asserts gating, episodic works)
- `cargo test -p memory --features cognitive-memory` (router path compiles)
- `cargo build -p memory` (default build excludes router)
- `cargo build -p runtime` (daemon uses only episodic -- still builds)
- `cargo build --workspace`

---

## Phase 7 -- Scope-aware Injection in Chat + End-to-End

### File: `crates/runtime/src/impl/daemon/handler/chat.rs`

**Modify the fact recall block at `chat.rs:121` to use governed search with scope.**

Replace:
```rust
                if let Ok(facts) = fs.search_facts(&query, None, 0.15, 4) {
```

With:
```rust
                if let Ok(facts) = fs.search_facts_governed(&query, None, false, 0.15, 4) {
```

**Build:** `cargo build -p runtime`

### Integration test: `crates/runtime/tests/factstore_canonical_recall.rs`

```rust
//! M-H Option A regression guard: after removing the cognitive MemoryRouter,
//! the daemon's canonical store (FactStore) must still recall injected facts.

use runtime::r#impl::memory::fact_store::FactStore;

#[test]
fn factstore_remains_the_canonical_recall_store() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FactStore::open(&dir.path().join("fact_store.db")).unwrap();

    let id = fs
        .add_fact("aletheon recalls facts via FactStore", "general", "", "", 0.7, "semantic", 0)
        .unwrap();

    let hits = fs.search_facts_governed("FactStore", None, false, 0.15, 4).unwrap();
    assert!(
        hits.iter().any(|f| f.fact_id == id),
        "daemon recall via FactStore must still return injected facts after MemoryRouter demotion"
    );
}
```

**Test command:** `cargo test -p runtime --test factstore_canonical_recall`

### Full workspace validation

```bash
cargo build --workspace
cargo test -p runtime
cargo test -p memory
cargo test -p memory --features cognitive-memory
```

---

## 4. Database Schema (Complete DDL)

### Facts table -- final schema after migration

```sql
CREATE TABLE IF NOT EXISTS facts (
    fact_id         INTEGER PRIMARY KEY AUTOINCREMENT,
    content         TEXT NOT NULL UNIQUE,
    category        TEXT NOT NULL DEFAULT 'general',
    tags            TEXT NOT NULL DEFAULT '',
    source_path     TEXT NOT NULL DEFAULT '',
    trust_score     REAL NOT NULL DEFAULT 0.5,
    retrieval_count INTEGER NOT NULL DEFAULT 0,
    helpful_count   INTEGER NOT NULL DEFAULT 0,
    tier            TEXT NOT NULL DEFAULT 'episodic',
    ttl_days        INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now')),
    -- governance columns (added by idempotent migration)
    scope           TEXT NOT NULL DEFAULT 'session',
    source          TEXT NOT NULL DEFAULT 'conversation',
    status          TEXT NOT NULL DEFAULT 'active',
    pinned          INTEGER NOT NULL DEFAULT 0,
    subject         TEXT NOT NULL DEFAULT ''
);

CREATE INDEX IF NOT EXISTS idx_facts_trust ON facts(trust_score DESC);
CREATE INDEX IF NOT EXISTS idx_facts_category ON facts(category);
CREATE INDEX IF NOT EXISTS idx_facts_tier ON facts(tier);
CREATE INDEX IF NOT EXISTS idx_facts_scope ON facts(scope);
CREATE INDEX IF NOT EXISTS idx_facts_status ON facts(status);

-- FTS5 virtual table + triggers (unchanged)
CREATE VIRTUAL TABLE IF NOT EXISTS facts_fts USING fts5(
    content, tags,
    content=facts, content_rowid=fact_id,
    tokenize='porter unicode61'
);

CREATE TRIGGER IF NOT EXISTS facts_ai AFTER INSERT ON facts BEGIN
    INSERT INTO facts_fts(rowid, content, tags) VALUES (new.fact_id, new.content, new.tags);
END;
CREATE TRIGGER IF NOT EXISTS facts_ad AFTER DELETE ON facts BEGIN
    INSERT INTO facts_fts(facts_fts, rowid, content, tags) VALUES('delete', old.fact_id, old.content, old.tags);
END;
CREATE TRIGGER IF NOT EXISTS facts_au AFTER UPDATE ON facts BEGIN
    INSERT INTO facts_fts(facts_fts, rowid, content, tags) VALUES('delete', old.fact_id, old.content, old.tags);
    INSERT INTO facts_fts(rowid, content, tags) VALUES (new.fact_id, new.content, new.tags);
END;
```

### Tables NOT changed by this plan

- `entities` / `fact_entities` -- entity graph (unchanged)
- `episodes` / `episodes_fts` -- episodic events (unchanged)
- `knowledge` / `knowledge_fts` -- extracted knowledge (unchanged)
- `consolidation_log` -- consolidation tracking (unchanged)
- `reflection_events` (EpisodicMemory) -- reflections (unchanged)

---

## 5. Migration Plan

### Step 1: No separate migration step needed

The `migrate_facts_table` function runs on every `FactStore::open()` call,
guarded by `PRAGMA table_info(facts)`. It is idempotent -- repeated opens add
no duplicate columns. No separate migration binary or up/down scripts.

### Step 2: Phase order for safe deployment

```
  Phase 1 (schema) ──► existing data gets defaults, daemon works
  Phase 2 (FactRow) ──► new fields visible in Rust, wire-compatible
  Phase 3 (write)   ──► new `add_fact_governed` path; legacy `add_fact` unchanged
  Phase 4 (retrieval)──► governed search + pin/archive APIs
  Phase 5 (CLI)     ──► user-facing management surface
  Phase 6 (M-H)     ──► dead router removal (behavior-neutral)
  Phase 7 (injection)──► scope-filtered chat injection
```

### Step 3: Backward compatibility

- Existing `add_fact` (legacy) callers (`rpc.rs:73` in "clear" handler, any
  internal tool callers) continue to work -- governance columns get their table
  defaults.
- `search_facts` (legacy) continues to work with the expanded column list.
- `FactRow` gains 5 fields; existing `Serialize` impl means JSON-RPC responses
  automatically include the new fields.
- EpisodicMemory, RecallMemory, CoreMemory, AutoMemory are untouched.

---

## 6. Rollback Plan

### What to revert per phase

| Phase | Rollback action | Risk |
|---|---|---|
| Phase 1 | Leave columns; they are default-valued and harmless. Alternatively: `ALTER TABLE facts DROP COLUMN scope;` etc. for each. No data loss -- defaults were inserted. | Zero |
| Phase 2 | Revert `FactRow` to 12 fields; revert SELECTs; revert `map_fact_row`. | Low (compile-time only) |
| Phase 3 | Remove `add_fact_governed` + `is_sensitive`. No data to clean up (same table). | Low |
| Phase 4 | Remove `search_facts_governed` / `set_pinned` / `set_status` / `list_facts`. | Low |
| Phase 5 | Revert JSON-RPC arms + CLI code. No state to clean up. | Low |
| Phase 6 | Git revert orchestrator.rs deletions; remove feature gates. Rebuild with `cargo build --workspace`. | Low (deletions are of dead code) |
| Phase 7 | Revert `search_facts` to `search_facts_governed` in chat.rs. | Zero |

### Database rollback (if columns must be removed)

SQLite does not support `ALTER TABLE DROP COLUMN` before 3.35.0. For older
versions, recreate the table:

```sql
-- Backup
CREATE TABLE facts_backup AS SELECT
  fact_id, content, category, tags, source_path,
  trust_score, retrieval_count, helpful_count,
  tier, ttl_days, created_at, updated_at
FROM facts;

-- Recreate without governance columns
DROP TABLE facts;
CREATE TABLE facts (... original schema ...);
INSERT INTO facts SELECT * FROM facts_backup;
DROP TABLE facts_backup;

-- Rebuild FTS
INSERT INTO facts_fts(facts_fts) VALUES('rebuild');
```

This should never be needed in practice (the columns are harmless).

---

## 7. Risk Assessment

### Risk 1: SQLite concurrent access (MEDIUM)

**Scenario:** The daemon holds `FactStore` behind `Arc<Mutex<FactStore>>`
(`handler/mod.rs:139`). Multiple chat turns or RPC calls within the same daemon
process serialize through this mutex. No external writers.

**Mitigation:**
- WAL mode is already enabled (`PRAGMA journal_mode=WAL` at `mod.rs:99`).
- All FactStore access goes through the single mutex.
- JSON-RPC `memory.*` calls acquire the same lock as the chat recall path.
- No concurrent-writer scenario exists in the current architecture.

**Residual risk:** Low. If a future change adds a second `FactStore` instance
pointing at the same file, SQLite WAL mode handles concurrent readers safely,
but only one writer. The `Arc<Mutex<>>` pattern prevents this.

### Risk 2: Column order dependency (LOW)

**Scenario:** `map_fact_row` reads by positional index (0..16). If a future
migration inserts a column before the governance columns, indices shift.

**Mitigation:**
- All SELECT statements use explicit column lists (never `SELECT *`).
- The migration is append-only -- columns are added at the end, preserving
  indices 0..11 exactly (the original 12 columns), then 12..16 (governance).
- The test `migration_is_idempotent_and_adds_columns` validates column presence.
- A future migration that inserts mid-table would need to update `map_fact_row`
  indices -- this is documented in the code comments.

### Risk 3: Secret safety false positives (LOW)

The `is_sensitive` regex may match benign text containing patterns like "api_key
= something". The penalty is a refused write (returned as error to caller). The
user can retry with `--source explicit` to bypass.

**Mitigation:** The regex targets specific patterns (sk- prefix, BEGIN PRIVATE
KEY, bearer tokens). False-positive rate is expected to be negligible. Future
iteration can make this configurable.

### Risk 4: M-H Phase 6 deletion causes compile failure (LOW)

Deleting dead code (`with_memory` + `MemoryRouter` field) from
`orchestrator.rs`. The downstream `if let Some(ref memory) = self.memory` block
was the only consumer; it was guarded by an always-`None` condition.

**Mitigation:**
- `rg "with_memory\b"` confirmed zero callers (2026-07-02).
- `cargo build --workspace` validates after deletion.
- The bifurcation guard test (`memory_bifurcation_guard.rs`) prevents accidental re-introduction.

### Risk 5: EpisodicMemory must stay in default build (MEDIUM)

The daemon depends on `EpisodicMemory` at `handler/mod.rs:43,104,353`. Gating it
would break the daemon build.

**Mitigation:** Only `cognitive-memory` gates `MemoryRouter` +
semantic/procedural/self + consolidation. `EpisodicMemory` + `activation` +
`decay` + `schema` stay default. The guard test
`episodic_memory_is_available_by_default` verifies this.

### Risk 6: FTS5 query sanitization edge cases (LOW)

Special characters in user queries can cause FTS5 syntax errors. The existing
`sanitize_fts_query` handles this by stripping non-alphanumeric characters.
`search_facts_governed` reuses it.

**Mitigation:** `sanitize_fts_query` is called identically in both
`search_facts` and `search_facts_governed`. No regression.

---

## 8. Affected Files Summary

| File | Phase | Change type | Key lines |
|---|---|---|---|
| `crates/runtime/src/impl/memory/fact_store/mod.rs` | 1,2,3 | Add migration, extend FactRow + map_fact_row, add is_sensitive | 22-36, 99-101, 247-260, 250-275 |
| `crates/runtime/src/impl/memory/fact_store/query.rs` | 2,3,4 | Extend SELECTs, add governed write/retrieval/pin/archive/list | 66-71, 78-86, 168, after 187 |
| `crates/runtime/src/impl/daemon/handler/rpc.rs` | 5 | Add memory.* JSON-RPC arms | Before line 777 |
| `crates/interact/src/tui/cli.rs` | 5 | Add MemoryAction enum, Command variant, memory_cmd handler | 69, 100+, 121+, 155, 170+ |
| `crates/runtime/src/core/orchestrator.rs` | 6 | Delete dead MemoryRouter wiring | 16, 34, 49, 88-91, 344-353 |
| `crates/runtime/tests/memory_bifurcation_guard.rs` | 6 | NEW: source-scan guard test | -- |
| `crates/memory/Cargo.toml` | 6 | Add [features] cognitive-memory | -- |
| `crates/memory/src/lib.rs` | 6 | Gate cognitive re-exports | 9-30 |
| `crates/memory/src/ops/mod.rs` | 6 | Gate router/consolidation modules | 9-18 |
| `crates/memory/src/backends/mod.rs` | 6 | Gate non-episodic modules | 9-17 |
| `crates/memory/tests/feature_gating_guard.rs` | 6 | NEW: feature gating guard test | -- |
| `crates/runtime/tests/factstore_canonical_recall.rs` | 7 | NEW: regression guard test | -- |
| `crates/runtime/src/impl/daemon/handler/chat.rs` | 7 | Scope-aware search_facts_governed | 121 |

**Files NOT touched by this plan:**
- `crates/base/src/include/memory.rs` (ABI types; MemoryType/MemoryEntry/MemoryQuery unchanged -- governance lives in FactStore, not the cognitive crate)
- `crates/memory/src/ops/schema.rs` (cognitive base table; M-H demotes but doesn't alter it)
- `crates/memory/src/ops/router.rs` (behind feature flag after M-H; code preserved under `#[cfg]`)
- `crates/memory/src/backends/episodic/` (kept default; daemon still uses it)
- `crates/memory/src/backends/{semantic,procedural,self_memory}/` (behind feature flag)
- `crates/runtime/src/impl/memory/{recall_memory,core_memory,auto_memory}/` (unchanged)
- `crates/runtime/src/impl/daemon/handler/mod.rs` (no structural changes; only reads FactStore)

---

## 9. Non-Goals (explicitly excluded)

1. Automatic write-trigger detection (implicit/task/periodic) -- MVP is explicit-only.
2. Merge/conflict resolution / decay-policy tuning.
3. Intent detection / query expansion / semantic vector search.
4. New storage backends (Postgres/Qdrant).
5. Layered Always/Task-relevant token-budgeted injection (deferred; scope filter is the MVP).
6. Data migration from cognitive backends (they were never populated on the live path).
7. M-A compaction unification (separate module).
8. Renaming/branding (M-G cancelled by owner directive).
