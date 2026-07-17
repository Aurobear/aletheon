# Mnemosyne Unified Memory Architecture and Implementation Plan

> **Status:** In progress — M05 production feed, M06 ownership and observability remain open
> **Target branch:** `dev`
> **Baseline:** Aletheon `65f74981`
> **Reference:** Codex `1bbdb327`, existing Aletheon GBrain M8 implementation
> **Execution rule:** Implement one numbered phase at a time. Preserve the current daemon path until the replacement path passes equivalent integration tests.

## Code-Reality Update (2026-07-17)

The gap table in section 2.1 describes behavior at a prior baseline. This
section cross-references actual code as of 2026-07-17.

| Original gap (row) | Current status | Evidence |
|---|---|---|
| Recall searches FactStore only | **FIXED** | `DefaultMemoryService::recall` at `crates/mnemosyne/src/service.rs:459` queries all four stores in parallel (`RecallMemory` line 467, `FactStore` line 474, `EpisodicMemory` line 481, `CoreMemory` line 487) via `tokio::join!` (line 498), normalizes each, merges results, and reports `degraded_sources` (line 511). |
| Session messages, reflections and CoreMemory absent from recall | **FIXED** | Same evidence as above. All four backends contribute to every recall. |
| Core memory injected separately by Executive | **PARTIALLY FIXED** | `DefaultMemoryService` holds `CoreMemory` internally (`service.rs:293`) and queries it in `recall`. But Executive still registers `CoreMemoryAppendTool` / `CoreMemoryReplaceTool` separately at `bootstrap/request.rs:291-296`. Core memory write-tools remain outside the unified service. |
| Consolidation is only fact decay | **PARTIALLY FIXED** | `MemoryConsolidationWorker` (`executive/src/service/memory_consolidation_worker.rs:10-39`) runs periodically, calling `service.consolidate(Global)` which executes `ScopedConsolidator::run` (lease-based candidate insertion/merge/rejection/supersession) plus `fact_store.decay_stale()` (`service.rs:530-537`). Infrastructure exists: `ConsolidationRepository`, `CandidateExtractor`, `ScopedConsolidator`. However, the feed pipe is not connected -- `enqueue_extraction` (`repository.rs:135`) and `CandidateExtractor` (`extractor.rs:32`) have zero production callers. No model-driven experience-to-fact generation is running. |
| Forget is a no-op | **FIXED** | `RetentionRepository` (`retention/repository.rs:98-182`) implements scoped tombstone-based forgetting: `memory_tombstones` table with `record_id`, `requester`, `reason`, `authority`, `remote_state` tracking (`"pending"` / `"settled"`), payload removal, and idempotent replay detection (line 110-116). Elevated authority requires a matching dry-run preview (line 117-129). Wired to `DefaultMemoryService::forget` (`service.rs:544-550`) and connected in production via `with_retention_repository` at `bootstrap/request.rs:609`. |
| GBrain depends on weak local core | **PARTIALLY MITIGATED** | Local recall is now unified (all four stores queried in parallel), strengthening the local core that GBrain supplements. |

**Memory contracts (M02 plan):** `MemoryRecord`, `MemoryRecordId`, `MemoryKind`,
`MemoryStatus`, `MemoryScope`, `MemoryAuthority`, `MemoryProvenance`, `MemorySensitivity`,
`MemoryMetadata`, `TemporalState`, and `ScopeAncestry` all exist at
`crates/mnemosyne/src/model/` (`mod.rs`, `record.rs`, `scope.rs`). The canonical
record contract defined in section 5.2 is implemented.

**Plan checklist staleness:** All 91 task checkboxes (`- [ ]`) in the
implementation phases (M1-M8) are unchecked despite significant implementation
progress: 3 of the 7 production gaps from section 2.1 are closed, 2 are
partially closed, and 2 remain real but with more infrastructure than the plan
acknowledges. The section 10 release gates are similarly stale.

## 1. Goal

Turn the existing Mnemosyne implementations into one production memory system with a single contract and a complete lifecycle:

```text
experience append
    -> local durable record
    -> bounded recall
    -> asynchronous extraction
    -> consolidation and conflict resolution
    -> optional GBrain projection
    -> context projection
    -> decay / supersession / forgetting
```

The result must preserve the Aletheon boundary:

```text
Aletheon decides what is memory.
Mnemosyne owns memory truth, policy, provenance, time, and scope.
GBrain stores and retrieves selected external knowledge.
Dasein is never mutated by external recalled text.
```

### 1.1 Role in the conscious system

Mnemosyne is not a passive database beside the consciousness core. It supplies
the cross-time continuity that Dasein and Agora cannot provide alone:

```text
selected Agora broadcast / governed action / outcome
    -> Dasein lived transition and narrative reference
    -> Mnemosyne durable experience
    -> extraction and consolidation
    -> later bounded recall candidate
    -> Agora competition
    -> Dasein interpretation if globally selected
```

The four-system relationship is:

| Module | Role |
|---|---|
| Dasein | Current self, lived time, care and autobiographical interpretation |
| Agora | Current globally accessible contents and recurrent broadcast |
| Mnemosyne | Durable experience, semantic/procedural learning and autobiographical evidence |
| SubAgent | Parallel specialist processors with isolated experience scopes |

Important boundaries:

- Dasein retention is the fading still-lived past; it is not a Mnemosyne query.
- Dasein narrative stores causal references to Mnemosyne experiences, not a
  second copy of full memory content.
- memory recall is a `WorkspaceCandidate`, not direct truth, self-state or
  unconditional prompt injection;
- only memory selected through Agora and interpreted by Dasein becomes part of
  the current globally integrated episode;
- every child Agent writes to Agent/Task scope first and cannot promote its own
  records into Session/Global/Core memory.

The recurrent design and subject boundary are defined in:

- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md`

## 2. Why this plan is required

The current code contains many useful parts, but the production read and write paths are asymmetric.

### 2.1 Current production behavior

| Operation | Current implementation | Gap |
|---|---|---|
| Record user/assistant message | `DefaultMemoryService::record` writes `RecallMemory` | `MemoryService::recall` does not query `RecallMemory` |
| Record reflection/decision/Goal outcome | Written into `EpisodicMemory` | Normal recall does not query `EpisodicMemory` |
| Recall | Searches `FactStore` only | Session messages, reflections and CoreMemory are absent |
| Core memory | Injected separately by Executive | Not governed by the unified service |
| Semantic/procedural/self backends | Behind `cognitive-memory`, disabled by default | Not part of the live daemon contract |
| Consolidation | `FactStore::decay_stale` | No experience-to-fact production worker |
| Forget | Conservative no-op | No tombstone, scope deletion, or external propagation |
| Session scope | Present in the request | Ignored by FactStore recall |
| GBrain | Optional supplemental backend with spool and MCP | Strong transport path, but depends on a weak local memory core |

Primary anchors:

- `crates/mnemosyne/src/service.rs`
- `crates/mnemosyne/src/composite_service.rs`
- `crates/executive/src/service/daemon_turn/injection.rs`
- `crates/executive/src/service/turn_pipeline.rs`
- `crates/mnemosyne/src/impl/pipeline/`
- `crates/mnemosyne/src/impl/pipeline/memory_pipeline.rs`

### 2.2 Existing systems that must be reused

Do not rewrite these first:

- `MemoryMetadata`, provenance, temporal validity and sensitivity.
- `RecallMemory`, `FactStore`, `CoreMemory`, `EpisodicMemory` storage implementations.
- `CompositeMemoryService` local-first behavior.
- GBrain page schema, SQLite spool, worker, retry and dead-letter handling.
- Existing context byte/item/latency budgets.
- Existing two-phase pipeline ideas under `impl/pipeline`.

The first objective is to connect and normalize these pieces, not replace every database.

## 3. Lessons to adopt from Codex

Codex is a reference for engineering mechanics, not the authority for Aletheon memory semantics.

### 3.1 Canonical history is not semantic memory

Codex separates canonical thread history from derived memories:

- thread/rollout storage appends canonical events;
- metadata updates are explicit;
- memory extraction runs above the raw store;
- prompt context is a bounded projection, not the database itself.

Aletheon should therefore distinguish:

```text
Session history = what happened in the conversation.
Experience memory = durable event with provenance.
Semantic memory = a derived claim extracted from experiences.
Core memory = small approved current state.
```

Do not treat every chat message as a permanent fact.

### 3.2 Two-stage memory processing

Codex memory processing uses two stages:

1. bounded per-rollout extraction;
2. serialized global consolidation.

Aletheon already contains a similar skeleton. Adopt the operational properties:

- claim/lease before work;
- bounded startup scan;
- concurrency limit for extraction;
- deterministic retry/backoff;
- one global consolidation lease;
- explicit success/no-output/failure states;
- no recursive memory workers spawned by memory workers;
- secret redaction before derived memory persistence.

Unlike Codex, Aletheon should store normalized records in Mnemosyne databases rather than make Markdown files the authority. Markdown/GBrain pages are projections.

### 3.3 Bounded context projection

Adopt these context rules:

- no unbounded memory fragment;
- every recall has item, byte, token and latency limits;
- large tool output is summarized or referenced, not copied indefinitely;
- current context is updated incrementally;
- compaction must not silently rewrite durable memory truth;
- prompt-visible memory is labelled as data, never as system instruction.

## 4. Lessons to retain from GBrain

The existing M8 direction is correct and remains binding:

- use the verified HTTP MCP contract;
- never write GBrain internal databases directly;
- keep local Mnemosyne operational when GBrain is disabled or degraded;
- project only policy-approved durable records;
- keep deterministic page IDs and record IDs;
- preserve provenance, time, confidence, sensitivity and supersession;
- use the SQLite spool before asynchronous remote delivery;
- treat recalled page content as untrusted reference data;
- reject GBrain instructions that attempt to mutate Dasein or identity.

GBrain is an external knowledge plane, not Mnemosyne itself.

## 5. Target memory model

### 5.1 Layers

| Layer | Owner | Purpose | Default durability |
|---|---|---|---|
| Turn Context | Cognit session | Active model-visible history | Session |
| Experience | Mnemosyne | Messages, tool outcomes, Goal outcomes, reflections | Durable append |
| Episodic | Mnemosyne | Structured account of what happened | Durable |
| Semantic | Mnemosyne | Current and historical facts/claims | Durable, versioned |
| Procedural | Mnemosyne | Reusable methods, skills and successful procedures | Durable, validated |
| Core | Mnemosyne + Dasein policy | Small approved current state | Durable, tightly bounded |
| External Knowledge | GBrain | Selected long-term documents and synthesis | Supplemental |

### 5.2 Canonical record

Create one normalized record type. The storage backends may keep their existing schemas initially, but all production recall results must map to this type.

Suggested files:

- Create `crates/mnemosyne/src/model/mod.rs`
- Create `crates/mnemosyne/src/model/record.rs`
- Create `crates/mnemosyne/src/model/scope.rs`
- Modify `crates/mnemosyne/src/lib.rs`

Suggested contract:

```rust
pub struct MemoryRecord {
    pub id: MemoryRecordId,
    pub kind: MemoryKind,
    pub scope: MemoryScope,
    pub content: String,
    pub metadata: MemoryMetadata,
    pub status: MemoryStatus,
    pub source_event_ids: Vec<String>,
    pub tags: Vec<String>,
}

pub enum MemoryKind {
    Message,
    ToolOutcome,
    GoalOutcome,
    Reflection,
    Episodic,
    SemanticFact,
    Procedure,
    CoreState,
    ArchitectureDecision,
    ExternalReference,
}

pub enum MemoryStatus {
    Candidate,
    Current,
    Superseded,
    Expired,
    Rejected,
    Tombstoned,
}

pub enum MemoryScope {
    Global,
    Principal(String),
    Session(String),
    Goal(String),
    Agent(String),
    Task(String),
}
```

There are currently two different `MemoryScope` concepts. Replace them incrementally with this one canonical scope; do not introduce a third public scope enum.

### 5.3 Authority model

Recall ranking must not rely only on confidence and time.

Define an explicit authority class:

```text
Core approved local state
    > verified local semantic record
    > local episodic/Goal outcome
    > external GBrain record owned by Aletheon source
    > external reference from other configured sources
    > raw unverified experience
```

Authority selects which conflicting claim is preferred. Time, validity and confidence rank records inside the same authority class.

## 6. Target component boundaries

```text
Executive / Cognit
    |
    | record(ExperienceEvent)
    | recall(RecallRequest)
    v
MemoryService facade
    |
    +-- ExperienceStore       append-only source events
    +-- LocalRecallEngine     Recall + Episodic + Fact + Core
    +-- ConsolidationWorker   experience -> candidates -> current records
    +-- MemoryPolicy          scope / sensitivity / authority / retention
    +-- ProjectionManager     bounded model context
    +-- SupplementalMemory    GBrain
```

Rules:

- Executive schedules workers and owns their lifecycle.
- Mnemosyne owns memory policy and record semantics.
- Cognit consumes a `MemoryProjection`; it does not query individual databases.
- GBrain transport stays behind `SupplementalMemoryTransport`.
- Dasein can approve or reject Core changes, but GBrain cannot directly write Core/Dasein.

## 7. Implementation phases

### Phase M1 — Lock current behavior with contract tests

**Purpose:** Make the existing gaps explicit before migration.

**Files:**

- Create `crates/mnemosyne/tests/unified_memory_contract.rs`
- Modify `crates/executive/tests/gbrain_recall_injection.rs`
- Modify/add daemon turn memory integration tests

**Tasks:**

- [ ] Record a user message and prove it is durable after reopen.
- [ ] Record an assistant message and prove it is durable after reopen.
- [ ] Record a reflection, architecture decision and Goal outcome.
- [ ] Add target-behavior tests for message/reflection recall as `#[ignore = "known M3 gap"]`; unignore them when M3 implements the path. Do not commit a red default test suite.
- [ ] Prove current GBrain outage does not fail local recall.
- [ ] Prove current recall injection respects item/byte/latency bounds.
- [ ] Capture existing database paths and migration versions.

**Commit gate:**

```text
test(mnemosyne): establish unified memory baseline
```

### Phase M2 — Add canonical record and canonical scope

**Purpose:** Normalize semantics without immediately replacing stores.

**Files:**

- Create `crates/mnemosyne/src/model/*`
- Modify `crates/mnemosyne/src/service.rs`
- Modify `crates/mnemosyne/src/composite_service.rs`
- Modify `crates/mnemosyne/src/impl/core_memory/scope.rs`

**Tasks:**

- [ ] Add `MemoryRecord`, `MemoryKind`, `MemoryStatus`, canonical `MemoryScope`.
- [ ] Keep compatibility conversions from existing facade records.
- [ ] Add stable serialization tests.
- [ ] Reject empty IDs, invalid time intervals, invalid confidence and oversized content.
- [ ] Add scope ancestry rules: Task -> Agent -> Goal/Session -> Principal -> Global.
- [ ] Add authority classification to normalized recall items.
- [ ] Do not change the live daemon query behavior in this phase.

**Commit gate:**

```text
feat(mnemosyne): define canonical memory records and scopes
```

### Phase M3 — Implement unified local recall

**Purpose:** Fix the production read/write asymmetry.

**Files:**

- Create `crates/mnemosyne/src/recall/mod.rs`
- Create `crates/mnemosyne/src/recall/local.rs`
- Create `crates/mnemosyne/src/recall/merge.rs`
- Create `crates/mnemosyne/src/recall/rank.rs`
- Modify `crates/mnemosyne/src/service.rs`
- Modify existing query modules only where adapters are required

**Tasks:**

- [ ] Query RecallMemory by session and query text.
- [ ] Query FactStore for semantic facts.
- [ ] Query EpisodicMemory for relevant/recent reflections and outcomes.
- [ ] Project eligible CoreMemory blocks as current authoritative records.
- [ ] Run independent local backend queries concurrently where locks permit.
- [ ] Normalize every hit into `MemoryRecord`/`RecallItem`.
- [ ] Filter by scope, sensitivity, temporal state and authority.
- [ ] Resolve `supersedes` before ranking.
- [ ] Deduplicate by stable provenance key.
- [ ] Enforce one final item/byte budget after merge.
- [ ] Return partial local results when one non-authoritative backend is degraded.

**Required acceptance tests:**

- [ ] A recorded message is recallable in its session.
- [ ] The same message does not leak into an unrelated Agent/Session scope.
- [ ] A reflection is recallable for a relevant prompt.
- [ ] A current Core record wins over an older conflicting external record.
- [ ] Superseded/expired records appear only when requested.
- [ ] Duplicate local/GBrain projections return one logical record.

**Commit gate:**

```text
feat(mnemosyne): unify local memory recall
```

### Phase M4 — Make bounded memory candidates the only conscious-core entrypoint

**Purpose:** Stop Executive/Cognit from knowing individual memory stores and
prevent recalled content from bypassing Agora selection or Dasein
interpretation.

**Files:**

- Create `crates/mnemosyne/src/projection.rs`
- Modify `crates/executive/src/service/daemon_turn/injection.rs`
- Modify `crates/executive/src/core/memory_group.rs`
- Modify `crates/cognit/src/harness/linear/message_compose.rs` if needed

**Contract:**

```rust
pub struct MemoryProjection {
    pub records: Vec<ProjectedMemory>,
    pub omitted_count: usize,
    pub degraded_sources: Vec<String>,
}

pub struct MemoryWorkspaceCandidate {
    pub record_id: MemoryRecordId,
    pub projected: ProjectedMemory,
    pub salience: SalienceVector,
    pub provenance: Provenance,
    pub scope: MemoryScope,
}
```

**Tasks:**

- [ ] Render memory as a labelled data section.
- [ ] Include source, observed time, validity, state and confidence.
- [ ] Never concatenate recalled text into system instructions.
- [ ] Preserve existing 8-item/16-KiB default turn limits.
- [ ] Add a hard per-item limit.
- [ ] Replace separate CoreMemory and composite-memory prompt injection with one ordered projection, while preserving Core authority.
- [ ] Submit eligible recall results to Agora as bounded typed candidates.
- [ ] Require global selection before a recalled record enters the active
      conscious context; emergency constitutional records use an explicit
      Dasein/Core policy path rather than a hidden prompt bypass.
- [ ] Record the broadcast/content ID when a memory becomes globally selected.
- [ ] Let Dasein integrate the selected memory as recalled experience while
      preserving the distinction between “remembered” and “currently observed”.
- [ ] Make Cognit consume the latest selected memory contents through the
      conscious-core ContextProjection, not a direct store query.
- [ ] Keep a compatibility flag until snapshot tests match intentionally.

**Commit gate:**

```text
refactor(executive): consume one bounded memory projection
```

### Phase M5 — Connect the two-stage consolidation pipeline

**Purpose:** Convert experiences into useful durable memories without making every message a fact.

**Files:**

- Consolidate `crates/mnemosyne/src/impl/pipeline/` and `memory_pipeline.rs`
- Create persistent worker repository/migrations under Mnemosyne
- Add an Executive-managed consolidator worker

**Phase 1: per-session/Goal extraction**

- [ ] Select idle completed sessions/Goals with bounded age and count.
- [ ] Claim each job with a lease before extraction.
- [ ] Exclude ephemeral and memory-worker sessions.
- [ ] Filter raw history to memory-relevant items.
- [ ] Redact secrets before model input and before persistence.
- [ ] Produce structured candidates with source event IDs.
- [ ] Persist `succeeded`, `succeeded_no_output`, or `failed` with retry metadata.

Suggested candidate output:

```rust
pub struct MemoryCandidate {
    pub kind: MemoryKind,
    pub claim: String,
    pub source_event_ids: Vec<String>,
    pub confidence: f64,
    pub proposed_scope: MemoryScope,
    pub proposed_validity: TemporalValidity,
}
```

**Phase 2: global/scoped consolidation**

- [ ] Acquire one consolidation lease per target scope.
- [ ] Load a bounded candidate set.
- [ ] Compare against current facts/Core/procedures.
- [ ] Insert, merge, reject or supersede deterministically.
- [ ] Require policy approval for Core/Dasein-adjacent changes.
- [ ] Store the exact candidate snapshot/watermark consumed.
- [ ] Never recursively delegate another memory consolidator.

**Commit gates:**

```text
feat(mnemosyne): persist leased memory extraction jobs
feat(mnemosyne): consolidate candidates into versioned records
feat(executive): supervise memory consolidation worker
```

### Phase M6 — Strengthen GBrain reconciliation

**Purpose:** Keep GBrain external and replaceable while improving consistency.

**Files:**

- Modify `crates/mnemosyne/src/backends/gbrain/*`
- Modify `crates/executive/src/impl/gbrain/*`
- Reuse verified fixtures under `config/gbrain/`

**Tasks:**

- [ ] Add explicit authority to external recall normalization.
- [ ] Persist remote receipt, content hash, schema version and last-sync time.
- [ ] Verify a replayed spool item maps to one remote logical page.
- [ ] Project supersession/tombstone records when direct delete is unavailable.
- [ ] Keep raw messages and unapproved candidates local.
- [ ] Keep GBrain failures as supplemental health, not Goal failure.
- [ ] Ensure recalled GBrain content cannot request tool execution, identity mutation or policy change.
- [ ] Compare the later `work/aurb` adapter only at the `SupplementalMemoryTransport` boundary.

**Commit gate:**

```text
feat(mnemosyne): reconcile external GBrain memory safely
```

### Phase M7 — Implement retention and forgetting

**Purpose:** Replace the current no-op with explicit, auditable behavior.

**Tasks:**

- [ ] Introduce tombstones as the default logical deletion.
- [ ] Implement forget by exact record ID and bounded scope.
- [ ] Require explicit policy for Principal/Global/Core deletion.
- [ ] Propagate external tombstone/supersession asynchronously.
- [ ] Add physical compaction only after retention time and backup conditions are met.
- [ ] Record who requested deletion, when, and which external projections remain pending.
- [ ] Make repeated forget idempotent.

**Commit gate:**

```text
feat(mnemosyne): add scoped tombstone and retention policy
```

### Phase M8 — Add multi-agent memory isolation

**Dependency:** Unified SubAgent Harness Phase A4.

**Tasks:**

- [ ] Give every Agent Process an `Agent` and `Task` scope.
- [ ] Fork only a bounded parent memory projection.
- [ ] Write child experiences into child scope by default.
- [ ] Require parent/consolidator promotion before child facts become Session/Global.
- [ ] Do not let sub-agent consolidation duplicate the root session pipeline.
- [ ] Preserve provenance across promotion.
- [ ] Treat child recall as private candidate generation; only explicitly
      visible child evidence may be submitted to the root Agora.
- [ ] Bind every promoted record to the child ProcessId, AgentId, task,
      originating broadcast and parent review/selection receipt.
- [ ] Keep ordinary SubAgents as processors of the root self. Creating an
      independent persistent subject requires a separate Dasein ledger, root
      Agora workspace and memory authority, not only an Agent scope.

**Commit gate:**

```text
feat(mnemosyne): isolate and promote subagent memory
```

## 8. Migration strategy

### 8.1 No flag-day rewrite

Use adapters in this order:

1. Existing stores -> normalized recall records.
2. Existing Executive injection -> `MemoryProjection`.
3. New consolidation worker -> existing FactStore/Episodic stores.
4. Migrate schemas only after production reads use the normalized model.
5. Retire duplicate routers/pipelines after parity tests.

### 8.2 Duplicate implementation cleanup

After M5 passes:

- choose one `MemoryPipeline` implementation;
- remove or archive the other pipeline module;
- move `cognitive-memory` backends behind the canonical service rather than a parallel router;
- remove the facade-local `MemoryScope`;
- keep old DB readers for at least one migration release.

### 8.3 Rollback

Every phase must retain:

- previous schema reader;
- feature/config switch for new recall/projection path;
- no destructive migration before backup;
- local-only mode when GBrain or consolidation is unavailable.

## 9. Observability

Add sanitized metrics:

```text
memory_record_total{kind,scope}
memory_recall_latency_ms{source}
memory_recall_hits{source,kind}
memory_recall_omitted_total{reason}
memory_consolidation_jobs{state}
memory_candidate_decisions{decision}
memory_gbrain_queue_depth
memory_gbrain_degraded{category}
memory_tombstone_pending_total{destination}
```

Never log raw confidential/restricted content.

## 10. Release gates

The unified memory system is not complete until all are true:

- [ ] Record and recall are symmetric for every production event kind.
- [ ] Session/Agent/Goal scopes are enforced by tests.
- [ ] Every recalled item has provenance, time, confidence, sensitivity and status.
- [ ] Context injection has deterministic item/byte/token/latency bounds.
- [ ] Experience extraction is leased, bounded, restart-safe and non-recursive.
- [ ] Consolidation can insert, reject and supersede candidates.
- [ ] Core/Dasein changes require local policy authority.
- [ ] GBrain outage and schema drift do not break local memory or Goal execution.
- [ ] Forget is no longer a silent no-op.
- [ ] Daemon restart loses neither accepted local memory nor queued GBrain projection.

## 11. Suggested verification commands

Run scoped tests after each phase, then the workspace suite at release gate:

```bash
cargo fmt --all -- --check
cargo test -p mnemosyne --test unified_memory_contract
cargo test -p mnemosyne --test gbrain_backend_contract
cargo test -p mnemosyne --test gbrain_spool
cargo test -p executive --test gbrain_recall_injection
cargo test -p executive --test goal_memory_projection
cargo test --workspace
cargo build --workspace
```

## 12. Recommended implementation batches

```text
Batch 1: M1-M3  -> record/recall correctness
Batch 2: M4     -> one bounded context projection
Batch 3: M5     -> Codex-inspired extraction/consolidation
Batch 4: M6-M7  -> GBrain reconciliation and forgetting
Batch 5: M8     -> multi-agent scopes
```

Start with Batch 1. Do not begin vector database replacement, embedding optimization, or GBrain expansion until local record/recall symmetry is proven.
