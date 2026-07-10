# Memory System

> Migrated from `docs/design/core/memory-system.md` — code paths updated to match actual crate names (fabric, cognit, corpus, dasein, mnemosyne, metacog, interact, executive)

> Inspired by Letta (MemGPT)'s three-tier self-editing memory architecture, enabling agents to manage their own memory like an OS manages virtual memory. Self-learning loop.

**Module:** 02
**Crates:** `executive` (executive-level memory: CoreMemory, RecallMemory, ArchivalMemory, compressor, pipeline), `mnemosyne` (backend storage: episodic, semantic, procedural, self_memory, router)
**Related modules:** [cognitive-engine](../executive/react-loop.md), [tool-system](../executive/orchestration.md)
**Last Updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| CoreMemory (L1) | Implemented | `runtime/src/impl/memory/core_memory.rs` | Block-based in-context memory with self-edit tools |
| RecallMemory (L2) | Implemented | `runtime/src/impl/memory/recall_memory.rs` | SQLite-backed conversation history |
| ArchivalMemory (L3) | Implemented | `runtime/src/impl/memory/archival_memory.rs` | `InMemoryArchival` (keyword search) + `VectorArchival` (vector-backed via VectorStore) |
| Memory tools | Implemented | `runtime/src/impl/memory/tools.rs` | core_memory_append/replace/recall_search etc. |
| ContextBudget | Implemented | `runtime/src/impl/memory/budget.rs` | Token budget tracking |
| AdvancedCompressor | Implemented | `runtime/src/impl/memory/compressor/mod.rs` | Token-budget tail protection with iterative summary updates |
| Tail Protection | Implemented | `runtime/src/impl/memory/compressor/tail.rs` | `TailProtectionConfig`, `find_tail_cut()` — soft ceiling + hard minimum + boundary alignment |
| Summary Template | Implemented | `runtime/src/impl/memory/compressor/template.rs` | `SummaryTemplate` with `render()` and `render_iterative()` for iterative summary updates |
| MemoryScope | Implemented | `runtime/src/impl/memory/scope.rs` | 3-tier isolation (Global/Session/Agent) with `ScopedCoreMemory`, `PendingWrite` approval, `Scratchpad` |
| Scoped Recall | Implemented | `runtime/src/impl/memory/scope.rs` | `ScopeFilter`, `ScopedRecallFilter` — scope-aware recall queries via metadata JSON |
| MemoryPipeline | Implemented | `runtime/src/impl/memory/pipeline/mod.rs` | Two-phase pipeline: Phase1 extraction + Phase2 consolidation |
| Phase1Extractor | Implemented | `runtime/src/impl/memory/pipeline/phase1.rs` | Parallel session extraction with lease-based claiming |
| Phase2Consolidator | Implemented | `runtime/src/impl/memory/pipeline/phase2.rs` | Global lock, rollout summaries, raw_memories.md output |
| StateDatabase | Implemented | `runtime/src/impl/memory/pipeline/state_db.rs` | In-memory session tracking with lease/watermark |
| Vector DB | Implemented | `runtime/src/impl/memory/vector_store.rs` | QdrantVectorStore, LanceVectorStore, OpenAIEmbedder |
| EpisodicMemory | Implemented | `memory/src/episodic.rs` | Episodic memory backend |
| SemanticMemory | Implemented | `memory/src/semantic.rs` | Semantic memory backend |
| ProceduralMemory | Implemented | `memory/src/procedural.rs` | Procedural memory backend |
| SelfMemory | Implemented | `memory/src/self_memory.rs` | Self-memory backend |
| MemoryRouter | Implemented | `memory/src/router.rs` | Routes queries to appropriate backends |
| MemorySchema | Implemented | `memory/src/schema.rs` | Schema definitions for memory storage |

---

## 1. Overview

The memory system is the foundation of Aletheon's persistent cognition. Inspired by Letta (MemGPT)'s three-tier memory architecture, it divides memory into three levels:

- **L1 Core Memory** — Editable blocks within the context window, agent self-managed
- **L2 Recall Memory** — SQLite-stored complete conversation history and tool call records
- **L3 Archival Memory** — Vector database stored long-term knowledge and patterns

The three layers interact through compression/eviction mechanisms, similar to the OS's CPU cache -> RAM -> disk hierarchy. The memory system also maintains context budgets to ensure the reasoning loop does not fail due to token limits.

---

## 2. Current Design

### 2.1 Three-Tier Memory Architecture

```
+-------------------------------------------------------------+
|                    Memory System                              |
|                                                               |
|  +---------------------------------------------------------+ |
|  |  L1: Core Memory — within context window                 | |
|  |                                                         | |
|  |  Block structure: label + value + limit + read_only     | |
|  |                                                         | |
|  |  Example blocks:                                        | |
|  |  - system_state: "Current focus: coding, CPU: 45%, ..." | |
|  |  - user_prefs: "Prefers Arch, uses vim, English first..."| |
|  |  - safety_rules: "No rm -rf /" (read_only)              | |
|  |                                                         | |
|  |  Agent self-edit tools:                                 | |
|  |  - core_memory_append(label, content)                   | |
|  |  - core_memory_replace(label, old, new)                 | |
|  |  - core_memory_rethink(label, new_content)              | |
|  +---------------------------------------------------------+ |
|                         | periodic compression/eviction      |
|                         v                                      |
|  +---------------------------------------------------------+ |
|  |  L2: Recall Memory — SQLite                              | |
|  |                                                         | |
|  |  Storage: full conversation history + tool calls + events| |
|  |  Index: timestamp + sessionID + event type              | |
|  |  Query: conversation_search, event_search,              | |
|  |         tool_call_search                                | |
|  |  Capacity: GB level, retain last 7 days + older summary | |
|  +---------------------------------------------------------+ |
|                         | vectorize                            |
|                         v                                      |
|  +---------------------------------------------------------+ |
|  |  L3: Archival Memory — vector database                   | |
|  |                                                         | |
|  |  Storage: long-term knowledge + user habit patterns +    | |
|  |           historical decisions                          | |
|  |  Retrieval: archival_memory_insert/search, pattern_match| |
|  |  Capacity: TB level, persistent                          | |
|  +---------------------------------------------------------+ |
+-------------------------------------------------------------+
```

**L1 Core Memory design notes:**
- Block is the editable unit within the context window, each block has `label` (identifier), `value` (content), `limit` (character limit), `read_only` (permission flag)
- Agent self-edits memory through `core_memory_append`, `core_memory_replace`, `core_memory_rethink` three tools (inspired by Letta `letta/functions/function_sets/base.py:246-280`)
- `read_only` blocks (e.g., `safety_rules`) are system-injected, agent cannot modify
- Core Memory content is directly injected into the LLM system prompt, consuming context window

**L2 Recall Memory design notes:**
- SQLite storage, retains last 7 days of complete records
- Supports multi-dimensional queries by time, session, event type, tool name
- Conversation history older than 7 days auto-compressed to summaries

**L3 Archival Memory design notes:**
- Vector database (Qdrant/LanceDB), stores long-term knowledge
- Supports semantic retrieval + tag filtering
- `pattern_match` can retrieve historically similar situations, aiding decisions

### 2.2 Context Budget Tracking

Inspired by Letta's `ContextWindowOverview` (`letta/schemas/memory.py:23-65`):

```
+-------------------------------------------------------+
|  Context Budget Tracking (ContextWindowOverview)        |
|                                                       |
|  System prompt:     1200 tokens  XXXXXXXX              |
|  Core Memory:        800 tokens  XXXXX                 |
|  Tool definitions:   600 tokens  XXXX                  |
|  Conversation:      4000 tokens  XXXXXXXXXXXXXXXXXXXXXX|
|  -----------------------------------------------      |
|  Total:             6600 / 8192 tokens                 |
|  Remaining:         1592 tokens                        |
|  Compress trigger:  >7000 tokens -> auto compress      |
+-------------------------------------------------------+
```

**Budget management rules:**
- Before each LLM call, tally current token consumption by category
- When total exceeds threshold (default 70%), trigger context compression
- Each Core Memory block's `limit` field prevents individual block bloat
- Compression preferentially evicts low-priority conversation messages, preserving Core Memory

### 2.3 Integration with OS Perception

The `system_state` block in Core Memory is automatically updated by the perception engine. The `system_state` block has `read_only: false`, periodically overwritten by the perception engine. Update frequency is determined by perception event `Priority` (Critical -> immediate, High -> next cycle, Normal -> on demand).

### 2.4 Rust Structure Definitions

- **MemorySystem** — Contains CoreMemory (L1), RecallDatabase (L2 SQLite), ArchivalDatabase (L3 vector DB), ContextBudget, Summarizer
- **CoreMemory** — Block array, each block has label/value/limit/read_only
- Code location: `runtime/src/impl/memory/core_memory.rs`, `runtime/src/impl/memory/recall_memory.rs`, `runtime/src/impl/memory/budget.rs`

---

## 3. Identified Defects

### 3.1 Memory Corruption on Crash (Extension of Session Persistence Issue)

**Severity:** P0

If aletheon daemon process crashes during Core Memory update (`append`/`replace`/`rethink`), block values may be partially written, or Recall Memory and Core Memory states may be inconsistent.

**Mitigation:** Unified by the Session persistence module. Memory system side needs:
- Core Memory update operations should be atomic (write WAL first, then apply)
- Recall Memory's SQLite natively supports transactions
- On recovery, rebuild Core Memory state from latest checkpoint

### 3.2 Vector Database Selection Undecided

**Severity:** P2

L3 Archival Memory's vector database has not been finalized between ChromaDB, Qdrant, LanceDB. POC comparison needed before Phase 2 implementation, focusing on: embedded deployment friendliness, Rust binding maturity, query latency.

### 3.3 P1: Multi-Agent Memory Isolation Missing

**Severity:** P1

**Problem:** The three-layer memory architecture works well in single-agent scenarios, but completely lacks scope isolation in multi-agent scenarios:

- **L1 Core Memory globally shared and writable** — all sub-agents see the same content, sub-agents can modify affecting all other agents
- **L2 Recall Memory has no agent identifier** — all agents' history in the same table, sub-agent intermediate reasoning pollutes global recall space
- **L3 Archival Memory has no archive tags** — vector similarity search cannot distinguish knowledge source
- **No task-level working memory** — sub-agents have no temporary storage for intermediate results

---

## 4. Improved Design

### 4.1 Atomic Core Memory Updates

Adopt WAL (Write-Ahead Log) pattern for atomic updates: write WAL entry first, then apply to memory, finally mark committed. Updates to read_only blocks are rejected.

### 4.2 Memory Recovery Flow

Recover from checkpoint + WAL: load latest checkpoint, replay uncommitted WAL entries to rebuild memory state.

### 4.3 MemoryScope — Three-Tier Memory Scope

Solves the multi-agent memory isolation deficiency in section 3.3.

#### 4.3.1 MemoryScope Definition

```rust
enum MemoryScope {
    /// Global scope — safety rules, user preferences, shared knowledge
    /// All agents readable, only parent agent writable
    Global,
    /// Session scope — parent agent's working memory + current session context
    /// Parent agent read/write, sub-agents read, sub-agent writes need approval
    Session,
    /// Agent scope — single agent's private working memory
    /// Only owner read/write, optionally preserved or discarded after task completion
    Agent(String), // agent_id
}
```

#### 4.3.2 Core Memory Scoping

Partition Core Memory into different scope memory blocks. `ScopedMemoryBlock` contains scope, label, content, read_only.

Sub-agent system prompts only inject Global + Session scope memory blocks, not other sub-agents' AgentScope.

#### 4.3.3 Recall Memory Scope Filtering

`RecallQuery` contains scope_filter field, sub-agents default to querying only Global + their own AgentScope. Querying SessionScope requires parent agent authorization.

SQLite tables add `scope_type` and `scope_id` columns with indexes.

#### 4.3.4 Archival Memory Scope Tags

`ArchivalEntry` metadata contains scope, agent_id, task_id, created_at. Retrieval optionally filters by scope, sub-agents auto-filter to Global + their own AgentScope.

#### 4.3.5 Task-Level Scratchpad

Provides temporary working memory for each sub-agent task:

`Scratchpad` contains agent_id, task_id, entries, retention. `RetentionPolicy` enum: Discard (discard after task), ArchiveToAgent (archive to AgentScope), ArchiveToSession (archive to SessionScope, needs approval).

**Write control policy:**
- Sub-agent write to GlobalScope is rejected
- Write to SessionScope needs parent agent approval
- Write to AgentScope is allowed by default
- Existing single-agent scenario behavior unchanged, scope defaults to Global (backward compatible)

---

## 5. Implementation Notes

| Item | Description |
|------|-------------|
| **Core Memory** | `runtime/src/impl/memory/core_memory.rs` — MemoryBlock structure + self-edit tools |
| **Recall Memory** | `runtime/src/impl/memory/recall_memory.rs` — SQLite schema + query interface |
| **Archival Memory** | `runtime/src/impl/memory/archival_memory.rs` — Vector DB wrapper |
| **Budget tracking** | `runtime/src/impl/memory/budget.rs` — ContextBudget |
| **Compressor** | `runtime/src/impl/memory/compressor/mod.rs` — AdvancedCompressor with tail protection |
| **Pipeline** | `runtime/src/impl/memory/pipeline/` — Two-phase consolidation (extraction + merge) |
| **Backend storage** | `memory/src/` — episodic, semantic, procedural, self_memory, router |
| **WAL** | Planned — with Session persistence shared WAL |

---

## 6. References

| Source | Key File | Borrowed Content |
|--------|----------|-----------------|
| Letta (MemGPT) | `letta/schemas/block.py:67-68` | MemoryBlock definition |
| Letta (MemGPT) | `letta/schemas/memory.py:68-77` | Memory class (three-tier memory container) |
| Letta (MemGPT) | `letta/schemas/memory.py:23-65` | ContextWindowOverview (budget tracking) |
| Letta (MemGPT) | `letta/functions/function_sets/base.py:246-280` | `core_memory_append` / `core_memory_replace` |
| Letta (MemGPT) | `letta/services/summarizer/compact.py` | Cheap model compression strategy |
| Anthropic SDK | `lib/tools/_beta_runner.py:177` | `_check_and_compact` context compression trigger |

---

## Implementation Summary

**Runtime-level memory (all under `runtime/src/impl/memory/`):**
- `core_memory.rs` — CoreMemory (L1) with block-based in-context memory and self-edit tools
- `recall_memory.rs` — RecallMemory (L2) with SQLite-backed conversation history
- `archival_memory.rs` — ArchivalMemory (L3) with `InMemoryArchival` (keyword search) and `VectorArchival` (vector-backed semantic search)
- `tools.rs` — Memory tools (core_memory_append/replace/recall_search etc.)
- `budget.rs` — ContextBudget (token budget tracking)
- `compressor/mod.rs` — `AdvancedCompressor` with token-budget tail protection and iterative summary generation
- `compressor/tail.rs` — `TailProtectionConfig` and `find_tail_cut()`
- `compressor/template.rs` — `SummaryTemplate` with `render()` and `render_iterative()`
- `scope.rs` — `MemoryScope` (Global/Session/Agent), `ScopedCoreMemory` with `PendingWrite` approval flow, `Scratchpad` with `RetentionPolicy`
- `pipeline/mod.rs` — `MemoryPipeline` orchestrating Phase 1 then Phase 2
- `pipeline/phase1.rs` — `Phase1Extractor` for parallel session extraction
- `pipeline/phase2.rs` — `Phase2Consolidator` for global consolidation
- `pipeline/state_db.rs` — `StateDatabase` for in-memory session tracking
- `vector_store.rs` — `VectorStore` trait with QdrantVectorStore, LanceVectorStore; `Embedder` trait with OpenAIEmbedder

**Backend storage (all under `memory/src/`):**
- `episodic.rs` — Episodic memory backend
- `semantic.rs` — Semantic memory backend
- `procedural.rs` — Procedural memory backend
- `self_memory.rs` — Self-memory backend
- `router.rs` — Memory router (routes queries to appropriate backends)
- `schema.rs` — Schema definitions

**Planned (not started):**
- WAL-based atomic Core Memory updates
- Checkpoint-based memory recovery (WAL + checkpoint replay)
