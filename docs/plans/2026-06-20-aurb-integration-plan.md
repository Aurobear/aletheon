# aurb → aletheon Integration Plan

**Date:** 2026-06-20
**Goal:** Port aurb's most valuable patterns into aletheon's Rust runtime as native components.

---

## Context

aurb is a Python-based agent infrastructure with sophisticated memory (FactStore + trust scoring + entity graph + FTS5), lifecycle hooks, and skill routing. aletheon is a Rust agent runtime with L1/L2/L3 memory (CoreMemory/RecallMemory/ArchivalMemory), but lacks structured fact storage, trust scoring, entity resolution, and lifecycle automation.

**Strategy:** Port high-value patterns as Rust native modules. No Python dependency at runtime.

---

## Phase 1: FactStore — Structured Fact Storage + Trust Scoring

### Problem

aletheon's `memory_pipeline.rs` extracts facts but appends them as free-text to CoreMemory blocks. No individual fact records, no confidence tracking, no dedup at storage level, no retrieval by query/trust/category.

### Design

New module: `crates/aletheon-runtime/src/impl/memory/fact_store.rs`

```
┌─────────────────────────────────────────────┐
│ FactStore (SQLite + FTS5)                   │
├─────────────────────────────────────────────┤
│ facts table:                                │
│   fact_id INTEGER PK AUTOINCREMENT          │
│   content TEXT NOT NULL UNIQUE               │
│   category TEXT DEFAULT 'general'           │
│   tags TEXT DEFAULT ''                       │
│   source_path TEXT DEFAULT ''               │
│   trust_score REAL DEFAULT 0.5              │
│   retrieval_count INTEGER DEFAULT 0         │
│   helpful_count INTEGER DEFAULT 0           │
│   tier TEXT DEFAULT 'episodic'              │
│   ttl_days INTEGER DEFAULT 0                │
│   created_at TIMESTAMP                       │
│   updated_at TIMESTAMP                       │
│                                             │
│ facts_fts (FTS5 on content, tags)           │
│   with sync triggers (INSERT/DELETE/UPDATE)  │
│                                             │
│ Trust scoring:                              │
│   helpful:   +0.05 (asymmetric)             │
│   unhelpful: -0.10 (2x penalty)             │
│   range: [0.0, 1.0]                         │
│   search gate: min_trust = 0.3              │
│   recall boost: +0.005 per retrieval        │
└─────────────────────────────────────────────┘
```

### Tasks

| # | Task | Files | Tests |
|---|---|---|---|
| 1.1 | Schema: facts table + FTS5 + triggers + indexes | `fact_store.rs` | +5 (schema, insert, dedup, FTS sync) |
| 1.2 | add_fact / search_facts / record_feedback | `fact_store.rs` | +8 (CRUD, FTS search, trust feedback, min_trust gate) |
| 1.3 | Trust decay: time-based decay for unused facts | `fact_store.rs` | +3 (decay, clamp, no-decay-recent) |
| 1.4 | Integration: wire FactStore into MemoryPipeline | `memory_pipeline.rs` | +3 (extract→store, dedup, search recall) |

**Estimated:** +19 tests, ~400 lines

### Key Decisions

- **UNIQUE on content** for dedup (same as aurb) — `INSERT OR IGNORE` + fetch existing on conflict
- **Trust decay** is time-based (aurb's recall-inject does -0.002 per 7 days unused) — we implement as a method `decay_stale()` callable periodically
- **FTS5 query safety**: wrap in double quotes, escape embedded quotes (same as aurb)
- **Retrieval count**: passive popularity signal, incremented on search results returned
- **No HRR in Phase 1** — defer to Phase 3

---

## Phase 2: Entity Graph — Resolution + Neighbors + Path Finding

### Problem

Facts are isolated strings. No way to find "all facts related to Kuavo" or "path from EtherCAT to motor control". aurb extracts entities from capitalized words, quoted strings, and "aka" patterns, then builds a co-occurrence graph.

### Design

Extend `fact_store.rs` with entity tables:

```
┌─────────────────────────────────────────────┐
│ Entity Graph                                │
├─────────────────────────────────────────────┤
│ entities table:                             │
│   entity_id INTEGER PK AUTOINCREMENT        │
│   name TEXT NOT NULL UNIQUE                  │
│   aliases TEXT DEFAULT ''                    │
│                                             │
│ fact_entities table:                        │
│   (fact_id, entity_id) COMPOSITE PK         │
│                                             │
│ API:                                        │
│   extract_entities(content) → Vec<String>   │
│   resolve_entity(name) → entity_id          │
│   get_entity_neighbors(id) → Vec<Neighbor>  │
│   find_entity_path(from, to, max_depth)     │
│   get_entity_facts(entity_id) → Vec<Fact>   │
│                                             │
│ Extraction patterns:                        │
│   - Multi-word capitalized: "John Smith"    │
│   - Double-quoted: "some term"              │
│   - Single-quoted: 'some term'              │
│   - AKA aliases: "X aka Y"                  │
└─────────────────────────────────────────────┘
```

### Tasks

| # | Task | Files | Tests |
|---|---|---|---|
| 2.1 | Schema: entities + fact_entities tables | `fact_store.rs` | +3 (schema, entity CRUD) |
| 2.2 | Entity extraction: regex patterns + dedup | `fact_store.rs` | +5 (capitalized, quoted, aka, dedup, empty) |
| 2.3 | Entity resolution: exact match → alias → create | `fact_store.rs` | +4 (exact, alias, new, case-insensitive) |
| 2.4 | Entity graph: neighbors (1-hop), path (BFS), facts | `fact_store.rs` | +5 (neighbors, path found, path not found, max_depth, entity facts) |
| 2.5 | Integration: auto-extract on add_fact | `fact_store.rs` | +2 (entities linked on add, re-extract on update) |

**Estimated:** +19 tests, ~350 lines

---

## Phase 3: Lifecycle Hooks — Event System + Recall Injection + Session Distillation

### Problem

aletheon has no lifecycle hooks. Memory recall is manual (LLM calls `memory_search` tool). No automatic session-end distillation. No automatic fact injection on user prompt.

### Design

New module: `crates/aletheon-runtime/src/impl/hooks/mod.rs`

```
┌──────────────────────────────────────────────┐
│ Hook System                                  │
├──────────────────────────────────────────────┤
│ HookEvent enum:                              │
│   UserPromptSubmit { prompt: String }        │
│   PreToolUse { tool: String, args: Value }   │
│   PostToolWrite { path: PathBuf }            │
│   SessionStop { transcript_path: PathBuf }   │
│                                              │
│ HookResult enum:                             │
│   Allow                                      │
│   Block { reason: String }                   │
│   Inject { context: String }                 │
│   Noop                                       │
│                                              │
│ Hook trait:                                  │
│   fn name(&self) -> &str                     │
│   fn events(&self) -> &[HookEvent]           │
│   fn handle(&self, event: &HookEvent)        │
│     -> Result<HookResult>                    │
│                                              │
│ Built-in hooks:                              │
│   1. RecallInjector (UserPromptSubmit)       │
│      - FTS5 search on FactStore              │
│      - Entity graph boost                    │
│      - Trust recall boost (+0.005)           │
│      - Inject as additionalContext           │
│                                              │
│   2. SessionDistiller (SessionStop)          │
│      - Read transcript JSONL                 │
│      - LLM extracts facts (cheap model)      │
│      - Write to FactStore                    │
│      - Extract episodes                      │
│                                              │
│   3. AutoFormatter (PostToolWrite)           │
│      - .rs: rustfmt                         │
│      - .py: black + py_compile              │
│      - .sh: shfmt + bash -n                 │
│      - .json: pretty-print                  │
└──────────────────────────────────────────────┘
```

### Tasks

| # | Task | Files | Tests |
|---|---|---|---|
| 3.1 | HookEvent + HookResult + Hook trait | `hooks/mod.rs` | +3 (enum variants, trait impl) |
| 3.2 | RecallInjector: FTS5 recall + entity boost + trust bump | `hooks/recall_inject.rs` | +6 (trivial gate, FTS recall, entity boost, trust bump, no results, inject format) |
| 3.3 | SessionDistiller: transcript parsing + LLM extraction + FactStore write | `hooks/session_distiller.rs` | +5 (parse JSONL, extract facts, write to store, skip short, skip disabled) |
| 3.4 | AutoFormatter: file extension dispatch | `hooks/auto_format.rs` | +4 (rustfmt, json pretty, unknown ext noop, missing file) |
| 3.5 | HookRunner: event dispatch + result aggregation | `hooks/mod.rs` | +3 (dispatch, block propagation, inject merge) |
| 3.6 | Integration: wire hooks into Controller | `controller.rs` | +2 (prompt submit → recall, session stop → distill) |

**Estimated:** +23 tests, ~600 lines

### Key Decisions

- **RecallInjector uses sqlite3 CLI** for sub-20ms (aurb pattern) — but in Rust we use rusqlite directly, which is equally fast
- **SessionDistiller** needs LLM API — use existing `ModelAdapter` or provider abstraction
- **Trivial prompt gate**: skip <8 chars, `/*` commands, greetings
- **Trust feedback loop**: recall boost +0.005, stale decay -0.002 per 7 days

---

## Phase 4: Skill Router — Keyword + Semantic Matching

### Problem

No automatic skill/workflow suggestion. User must manually invoke `/workflow` or `/skill-name`.

### Design

New module: `crates/aletheon-runtime/src/impl/skill_router.rs`

```
┌──────────────────────────────────────────────┐
│ SkillRouter                                  │
├──────────────────────────────────────────────┤
│ SkillEntry:                                  │
│   name: String                               │
│   triggers: Vec<String> (bilingual)          │
│   tags: Vec<String>                          │
│   description: String                        │
│   path: PathBuf                              │
│                                              │
│ Two-layer scoring:                           │
│   Layer 1: Keyword (substring match)         │
│     triggers: +2.0 per match                │
│     name:     +1.5 per match                │
│     tags:     +0.5 per match                │
│                                              │
│   Layer 2: FTS5 semantic                     │
│     search FactStore category="skill"        │
│     cap at min(3.0 * raw, 1.5)              │
│                                              │
│   Final: keyword + semantic                  │
│   Confidence: clamp(raw / 3.0, 0.0, 0.99)   │
│   Threshold: min_confidence = 0.6            │
│                                              │
│ Suggest-only mode: never auto-execute        │
└──────────────────────────────────────────────┘
```

### Tasks

| # | Task | Files | Tests |
|---|---|---|---|
| 4.1 | SkillEntry + skill loading from SKILL.md frontmatter | `skill_router.rs` | +4 (parse frontmatter, triggers, tags, missing) |
| 4.2 | Keyword scoring: substring match with weights | `skill_router.rs` | +4 (trigger match, name match, tag match, no match) |
| 4.3 | FTS5 semantic scoring from FactStore | `skill_router.rs` | +3 (semantic hit, cap, no results) |
| 4.4 | Final ranking + confidence threshold | `skill_router.rs` | +3 (sorted, threshold filter, empty) |
| 4.5 | Integration: suggest on long prompts via RecallInjector | `hooks/recall_inject.rs` | +2 (suggest, skip short) |

**Estimated:** +16 tests, ~350 lines

---

## Phase 5: Episodes + Knowledge — Consolidation Pipeline

### Problem

No episode tracking (task outcomes) and no knowledge consolidation (episodes → reusable knowledge). aletheon's pipeline extracts facts but doesn't track task success/failure patterns.

### Design

Extend `fact_store.rs` with episodes + knowledge tables:

```
┌──────────────────────────────────────────────┐
│ Episodes + Knowledge                         │
├──────────────────────────────────────────────┤
│ episodes table:                              │
│   episode_id INTEGER PK AUTOINCREMENT        │
│   session_id TEXT NOT NULL                    │
│   task TEXT NOT NULL                          │
│   context_json TEXT DEFAULT '{}'             │
│   actions_json TEXT DEFAULT '[]'             │
│   outcome TEXT CHECK(success|failure|        │
│                      partial|abandoned)      │
│   outcome_detail TEXT DEFAULT ''             │
│   importance REAL DEFAULT 0.5                │
│   consolidated INTEGER DEFAULT 0             │
│   timestamp TIMESTAMP                        │
│                                              │
│ episodes_fts (FTS5 on task, context_json)    │
│                                              │
│ knowledge table:                             │
│   knowledge_id INTEGER PK AUTOINCREMENT      │
│   topic TEXT NOT NULL                         │
│   content TEXT NOT NULL                       │
│   source_episodes TEXT DEFAULT '[]'          │
│   confidence REAL DEFAULT 0.5                │
│   access_count INTEGER DEFAULT 0             │
│   created_at TIMESTAMP                        │
│                                              │
│ knowledge_fts (FTS5 on topic, content)       │
│                                              │
│ consolidation_log table:                     │
│   log_id, run_at, episodes_processed,        │
│   knowledge_extracted, errors                │
│                                              │
│ API:                                         │
│   add_episode(session, task, outcome, ...)   │
│   get_unconsolidated(limit) → Vec<Episode>   │
│   mark_consolidated(ids)                     │
│   add_knowledge(topic, content, confidence)  │
│   search_knowledge(query) → Vec<Knowledge>   │
│   log_consolidation(stats)                   │
└──────────────────────────────────────────────┘
```

### Tasks

| # | Task | Files | Tests |
|---|---|---|---|
| 5.1 | Episodes schema + CRUD | `fact_store.rs` | +5 (add, search, count, consolidated filter, outcome constraint) |
| 5.2 | Knowledge schema + CRUD + FTS5 | `fact_store.rs` | +4 (add, FTS search, access_count bump, confidence) |
| 5.3 | Consolidation log | `fact_store.rs` | +2 (log, get_last) |
| 5.4 | Integration: auto-record episodes on tool results | `react_loop.rs` | +3 (success episode, failure episode, skip trivial) |

**Estimated:** +14 tests, ~300 lines

---

## Summary

| Phase | Component | New Tests | New Lines | Priority |
|---|---|---|---|---|
| **1** | FactStore + Trust Scoring | +19 | ~400 | P0 |
| **2** | Entity Graph | +19 | ~350 | P0 |
| **3** | Lifecycle Hooks | +23 | ~600 | P1 |
| **4** | Skill Router | +16 | ~350 | P1 |
| **5** | Episodes + Knowledge | +14 | ~300 | P2 |
| **Total** | | **+91** | **~2000** | |

### Dependency Graph

```
Phase 1 (FactStore)
  ├── Phase 2 (Entity Graph) — extends FactStore schema
  ├── Phase 3 (Hooks) — uses FactStore for recall/distillation
  │     └── Phase 4 (Skill Router) — uses FactStore + Hooks
  └── Phase 5 (Episodes + Knowledge) — extends FactStore schema
```

### What We're NOT Porting (and Why)

| aurb Feature | Why Skip |
|---|---|
| HRR (Holographic Reduced Representations) | Research-grade, numpy dependency, diminishing returns vs FTS5 |
| Embedding Worker (model2vec) | Heavy dependency, FTS5 is sufficient for Phase 1-5 |
| Memory Banks (category superposition) | Only useful with HRR |
| Vector similarity fallback | FTS5 + trust scoring covers 90% of use cases |
| Full RAGEngine facade | We compose directly from FactStore |

### Integration Points with Existing Code

| Existing Module | Integration |
|---|---|
| `memory_pipeline.rs` | Phase 1: route ExtractedFact → FactStore instead of CoreMemory append |
| `recall_memory.rs` | Phase 3: RecallInjector can also search RecallMemory |
| `controller.rs` | Phase 3: fire HookEvents at lifecycle boundaries |
| `react_loop.rs` | Phase 5: auto-record episodes on tool execution |
| `scope.rs` | Phase 1: FactStore respects MemoryScope for multi-agent isolation |
