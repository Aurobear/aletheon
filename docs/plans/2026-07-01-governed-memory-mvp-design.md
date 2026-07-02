# Governed Memory MVP — Design

**Date:** 2026-07-01
**Status:** Approved design (design-only; no implementation yet)
**Source docs:** `docs/guide/gpt-suggestion3.md` (governed memory spec), §21 (MVP), §22 (CLI)
**Scope decision:** Doc 3 MVP slice — schema + explicit save/forget + task-relevant retrieval + layered injection + `memory` CLI. Automatic write-triggers, merge/conflict resolution, intent detection, and new vector backends are explicit non-goals (deferred to follow-up specs).

---

## 0. Reality correction (post-audit, 2026-07-01) — SUPERSEDES router-specific details below

An audit of the *live* daemon chat path found this spec's original target was
wrong. There are **two parallel memory subsystems**:

- **Cognitive `memory` crate** — `MemoryRouter` + 4 backends
  (episodic/semantic/procedural/self). `MemoryRouter` is referenced **only** by
  `runtime/src/core/orchestrator.rs:34` as an `Option`, and the daemon chat path
  does **not** use it (only `EpisodicMemory` is wired in directly).
- **Runtime daemon memory** (`crates/runtime/src/impl/memory/`) — the store the
  daemon actually uses per turn: **`FactStore`** (`fact_store/mod.rs:91`),
  `RecallMemory`, `CoreMemory`, `AutoMemory`. Held on `RequestHandler`
  (`handler/mod.rs:90-167`).

**Therefore the Governed Memory target is `FactStore`, not `MemoryRouter`.** Good
news: `FactStore` already implements most of the governed schema. Spec-vs-code diff:

| Doc-3 governance field | FactStore reality (`fact_store/mod.rs`) | Action |
|---|---|---|
| confidence | `trust_score REAL DEFAULT 0.5` (mod.rs:113) + `record_feedback`/`decay_stale` | **reuse** (rename concept to confidence in CLI) |
| ttl | `ttl_days INTEGER DEFAULT 0` (mod.rs:117) | **reuse** |
| type/domain | `category` + `tier` (mod.rs:110,116) + `tags` (mod.rs:111) | **reuse** (domain = tag/category) |
| source | — | **add** `source TEXT` |
| scope | — | **add** `scope TEXT DEFAULT 'session'` |
| status (archive) | — | **add** `status TEXT DEFAULT 'active'` |
| pinned | — | **add** `pinned INTEGER DEFAULT 0` |
| subject | `content` is UNIQUE; no separate subject | **add** `subject TEXT` (optional; dedup already via UNIQUE content) |
| FTS search | `facts_fts` FTS5 + triggers (mod.rs:127-145) | **reuse** |
| retrieval/access | `retrieval_count` (mod.rs:114) | **reuse** |
| user CLI | absent | **add** (via daemon JSON-RPC) |

**Net effect on this MVP:** the schema work shrinks to adding
`scope/source/status/pinned/subject` columns to the `facts` table (idempotent
`ALTER TABLE` guarded by `PRAGMA table_info(facts)`); trust/ttl/tags/FTS/decay
are already there. The **CLI is the largest remaining piece**. The daemon
protocol is untyped **JSON-RPC over the Unix socket dispatched by a `method`
string** (`handler/mod.rs:883`, arms in `handler/rpc.rs`), so `memory.*` methods
are new match arms + a new `interact` subcommand — no shared enum to extend.

The router/`MemoryEntry`/`MemoryType` details in §2–§9 below are retained for
context but are **superseded by this section** wherever they conflict. The
bifurcation itself (which subsystem should survive long-term) is tracked as
roadmap item **M-H** — this MVP does not resolve it; it governs the live store
(`FactStore`) and leaves the cognitive crate alone.

---

## 1. Problem & motivation

Aletheon already has a working memory subsystem (SQLite-backed, four cognitive backends behind a `MemoryBackend` trait), but it is **passively accumulated and ungoverned**:

- No **scope** — every memory is global-ish; cannot distinguish "this session only" from "long-term project fact" (`memory/src/ops/schema.rs:9`).
- No **provenance / trust** — no `source` or `confidence`; cannot rank or filter by how the memory was acquired.
- No **expiry** — no `ttl`; temporary facts live forever.
- No **user control** — there is no way to list, inspect, forget, or pin memories (`memory_search` exists as a tool, but no management CLI).
- No **layered injection** — retrieval feeds context without an always/task-relevant tiering or a token budget.

Doc 3's thesis: *Memory is governed runtime state — explicit, inspectable, editable, scoped, ranked, evolvable.* This spec delivers the MVP of that.

## 2. Core principle: two orthogonal axes (additive, not a rewrite)

The existing `MemoryType` (`base/src/include/memory.rs:16`) — **Episodic / Semantic / Procedural / Self** — is a **cognitive/storage axis**: it decides *which backend* stores a memory and *how it decays*. We keep it unchanged.

Doc 3's "types" (User / Project / Workflow / Robot / Trace / Knowledge) and "scopes" (global / project / session / workflow / robot / user / temporary) are a **governance axis**. We add this axis on top:

- **Scope** becomes a first-class enum column.
- **Domain** (User/Project/Robot/…) is expressed as a **tag**, not a second enum — this avoids a two-headed taxonomy while still allowing domain filtering.

Result: a **schema migration + a thin governance layer**, not a rewrite. The neuroscience decay/consolidation model (`memory/src/ops/decay.rs`, `consolidation.rs`) is left intact.

## 3. Schema extension

Add the following to `MemoryEntry` (`base/src/include/memory.rs:29`) and the `memory` base table (`memory/src/ops/schema.rs:9`). Migration uses idempotent `ALTER TABLE ADD COLUMN` so existing rows get defaults (backward-compatible; new installs get the full schema via `CREATE TABLE`).

| Field | Type | Default | Purpose |
|---|---|---|---|
| `scope` | enum: Global / Project / Session / Workflow / Robot / User / Temporary | `Session` | governance boundary |
| `subject` | `Option<String>` | `NULL` | what the memory is about (ranking + dedup key) |
| `confidence` | `f64` (0.0–1.0) | `1.0` | trust weight; feeds ranking |
| `source` | enum: Conversation / Explicit / Tool / Import / System | `Conversation` | provenance |
| `ttl` | `Option<DateTime<Utc>>` | `NULL` | absolute expiry; Temporary memories set this |
| `last_used_at` | `Option<DateTime<Utc>>` | `NULL` | freshness signal; updated on recall |
| `status` | enum: Active / Archived | `Active` | soft-delete (forget archives before hard delete) |
| `pinned` | `bool` | `false` | user-protected; excluded from decay/compaction, ranking boost |

New enums (`MemoryScope`, `MemorySource`, `MemoryStatus`) live in `base/src/include/memory.rs` next to `MemoryType`. All serialize as TEXT.

**Migration approach:** a `migrate_base_table(conn)` helper runs each `ALTER TABLE ADD COLUMN IF NOT EXISTS`-equivalent (SQLite lacks `IF NOT EXISTS` for columns, so guard by reading `PRAGMA table_info(memory)` and adding only missing columns). Idempotent; safe to run on every open.

## 4. Explicit save / forget (write path)

New `MemoryWriteRequest { source_text, scope, source, confidence, subject, ttl }`. Write pipeline (doc §17):

```
WriteRequest
  → SafetyCheck   (reject obvious secrets — API keys, passwords, tokens — via regex,
                   per §19, unless source == Explicit AND user-confirmed)
  → DedupCheck    (match on (subject, scope); if a near-duplicate exists, skip or bump
                   access_count instead of inserting)
  → Store         (route to the correct cognitive backend by MemoryType; persist governance columns)
```

**MVP is explicit-only:** memories are written when the user asks ("remember X") or via `memory add`. Automatic write-trigger detection (implicit/task/periodic) is a **non-goal** here.

**Forget** is two-stage: `forget` sets `status = Archived` (recoverable); a `--hard` flag deletes the row. Pinned memories require `--force` to archive.

## 5. Task-relevant retrieval

Extend `MemoryQuery` (`base/src/include/memory.rs:60`) with a `scope: Option<Vec<MemoryScope>>` filter and a `min_confidence: Option<f64>`. Ranking (in `memory/src/ops/router.rs` recall path) becomes:

```
score = activation                     (existing: recency × frequency × importance)
        × confidence
        × scope_match_boost             (current project/session scope ranks higher)
        + pinned_boost                  (pinned always floated to top tier)
```

On recall, update `last_used_at = now` for returned entries (feeds freshness/decay). Expired entries (`ttl < now`) are filtered out at query time and swept during compaction.

## 6. Layered context injection

`MemoryContext` (`memory/src/ops/router.rs:40`) gains tiers under a configurable **token budget** (doc §11):

- **Always** — Global-scope + pinned memories (small, high-value core).
- **Task-relevant** — memories whose scope/subject matches the current project/session/task.
- *(Optional tier — low-confidence/historical — deferred to a later spec.)*

Injection fills Always first, then Task-relevant until the budget is exhausted, emitting a structured block (doc §12 format). Budget default is a fraction of the provider context window (reuse `llm.max_context_length()` wiring already present in the daemon handler).

## 7. `memory` CLI (Interface layer)

Lives in the `interact` crate (CLI = Interface, per doc 4). Commands (doc §22):

| Command | Action |
|---|---|
| `memory add "<text>" [--scope S] [--source explicit] [--subject …]` | explicit save |
| `memory list [--scope S] [--type T] [--limit N]` | list entries |
| `memory search "<query>" [--scope S]` | ranked search |
| `memory show <id>` | full entry incl. governance fields + `reason` for retrieval |
| `memory forget <id> [--hard] [--force]` | archive / hard-delete |
| `memory pin <id>` / `memory unpin <id>` | protect / unprotect |

**Transport:** all commands route through the **existing daemon Unix socket** (Runtime owns memory state; single writer). This requires new request/response variants in the daemon protocol (`runtime/src/impl/daemon/`) and matching handlers. Requires the daemon to be running (documented precondition); a future spec may add a daemon-less read path.

## 8. Data flow (summary)

```
SAVE:   user "remember X" / `memory add`
          → MemoryWriteRequest → SafetyCheck → DedupCheck → Store(scope, backend)

RECALL: turn start / `memory search`
          → MemoryQuery(scope-filtered, min_confidence)
          → Rank(activation · confidence · scope_match + pinned)
          → Inject(Always → Task-relevant, token-budgeted)
          → touch last_used_at
```

## 9. Affected files (CORRECTED target — `FactStore`; not touched by this spec)

Per §0, the target is the live `FactStore`, not the cognitive `memory` crate.

- `crates/runtime/src/impl/memory/fact_store/mod.rs` — add `scope/source/status/pinned/subject`
  columns to the `facts` table + an idempotent `migrate_facts_table` (guarded by
  `PRAGMA table_info(facts)`); extend `FactRow` with the new fields; add
  `pin/unpin/archive` methods; add scope-aware safety check on the insert path.
- `crates/runtime/src/impl/memory/fact_store/query.rs` — scope/status filters in
  `search_facts`; exclude archived/expired; pinned ranking boost.
- `crates/runtime/src/impl/daemon/handler/rpc.rs` — new `memory.add/list/search/show/forget/pin/unpin`
  match arms (JSON-RPC, following the existing `reflect`/`compact` arm pattern).
- `crates/runtime/src/impl/daemon/handler/chat.rs` — safety/scope on the auto-memory
  write path (`chat.rs:632`); scope-filtered injection where FactStore is recalled
  (`chat.rs:113-146`).
- `crates/interact/src/tui/cli.rs` — new `Memory { action: MemoryAction }` subcommand
  (variant in `Command` enum `cli.rs:69`, arm in `handle_command` `cli.rs:155`), using
  the `send_rpc` helper pattern (`debug.rs:1194`).

Deliberately **not** touched: `crates/memory/` (cognitive crate) — see M-H.

## 10. Testing strategy

**Unit**
- Migration idempotency (`migrate_base_table` run twice = no error, no dup columns).
- Scope filter returns only in-scope entries; `min_confidence` threshold respected.
- Safety regex rejects API-key/password patterns; allows normal text.
- Dedup collapses `(subject, scope)` duplicates.
- `ttl` expiry: expired entries excluded from recall and swept by compaction.

**Integration**
- `add → list → search → show → forget → pin` round-trip through the daemon.
- Injection respects token budget and tier order (Always before Task-relevant).
- Backward compat: an old DB (pre-migration) opens, migrates, and serves queries.

## 11. Non-goals (this spec)

- Automatic write-trigger detection (implicit/task/periodic) — doc §7, §18.
- Merge / conflict resolution / decay-policy changes / archival policy — doc §13 (beyond the soft-archive we add).
- Intent detection, query expansion, ranking-by-reason explanations beyond a basic `reason` string — doc §9, §10.
- New storage backends (Postgres/Qdrant/…) — doc §15; current SQLite router stays.

## 12. Follow-up specs (roadmap pointers)

1. **Write-triggers & candidate classification** (doc §7–§8, §18).
2. **Memory evolution** — merge / conflict / decay tuning / archival (doc §13).
3. **Retrieval pipeline** — intent detection, query expansion, conflict check (doc §9–§10).
4. **Pluggable stores** — `MemoryStore` backends beyond SQLite (doc §15).
