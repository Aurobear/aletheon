# Agora + Primitives Absorption — Design Spec

**Date:** 2026-07-10
**Source RFCs:** RFC-014 (Agora), RFC-015 (Mnemosyne), RFC-016 (Cognit), RFC-017 (Primitives)
**Scope decision:** Approach B — absorb only RFC-017 (Primitives) + RFC-014 (Agora). RFC-015/016 are
refinements of already-implemented code and are explicitly deferred (YAGNI).

---

## 1. Goal

Fill the one genuine gap these RFCs reveal: **working-memory state currently has no home**. Today it is
scattered — `Scratchpad` lives in `mnemosyne` (long-term memory), shared context lives in
`executive/session_gateway`. Absorb:

1. **RFC-017 Primitives** → a canonical shared vocabulary in `fabric` (cognitive objects + communication
   primitives), so subsystems communicate via primitives rather than concrete implementations.
2. **RFC-014 Agora** → a new `agora` crate: the session-isolated shared cognitive workspace positioned
   between Cognit and Corpus.

**Non-goals (explicitly deferred):**
- RFC-015 Mnemosyne background-service rewrite — `mnemosyne/src/ops` already has consolidation/decay/activation.
- RFC-016 Cognit harness-graph refactor — `cognit/harness/linear` is sufficient.
- Rewriting the working IPC layer.

---

## 2. Boundary Decisions

| Decision | Choice | Rationale |
|----------|--------|-----------|
| Scratchpad ownership | Move `mnemosyne/scope.rs::Scratchpad` → `agora` | RFC-014 assigns scratchpad to the workspace; Mnemosyne owns only long-term memory. (Note: this type is currently **dead code** — only its own unit tests reference it — so the move is clean.) |
| `executive/kernel/ipc.rs::SharedScratchpad` | **Keep as-is** | Different responsibility — a kernel-level IPC task-KV store for multi-agent concurrency, not a cognitive workspace. Coexists with Agora's scratchpad. |
| Primitives location | New `fabric/src/primitives/` | fabric is the ABI layer every crate depends on — the natural home for shared vocabulary. |
| Existing cognitive types | **Do not refactor** — re-export + add only the missing ones | Avoid large-scale breakage; YAGNI. |
| Comm primitives | Keep existing `Pattern`/`Envelope`; **add** typed `Command`/`Query`/`Event`/`Stream` wrappers + `Mailbox` | Existing IPC works — add a type-safe layer, don't overturn it. |
| Agora persistence | None — session-scoped, in-memory; persists only via `commit()` → Mnemosyne | RFC-014 principle: "Never persistent by itself". |
| Agora dependencies | `agora → fabric` only | Orchestration (recall/commit) lives in executive; avoids circular deps. |

---

## 3. RFC-017 Primitives (`fabric/src/primitives/`)

Pure type layer, no business logic. All crates reference via `fabric::primitives::*`.

```
fabric/src/primitives/
├── mod.rs          # re-export everything
├── cognitive.rs    # 9 cognitive objects (5 re-export + 4 new)
└── comm.rs         # Envelope re-export + Command/Query/Event/Stream/Mailbox
```

### 3a. Cognitive objects

| Object | Current state | Action |
|--------|---------------|--------|
| Intent | `include/self_field.rs` | re-export |
| Observation | `include/brain.rs` | re-export |
| Plan | `include/brain.rs` | re-export |
| Experience | `include/brain.rs` | re-export |
| Decision | `policy` | re-export |
| **Hypothesis** | missing | new: `{ id, statement, confidence: f64, evidence_ids: Vec<String> }` |
| **Evidence** | missing | new: `{ id, source: String, content: String, weight: f64 }` |
| **Narrative** | scattered in dasein | new canonical: `{ id, summary: String, entries: Vec<String> }` |
| **Commitment** | missing | new: `{ id, statement: String, created_at, status: CommitmentStatus }` |

New types are simple `serde` structs with `#[derive(Debug, Clone, Serialize, Deserialize)]`.

### 3b. Communication primitives

| Primitive | Current state | Action |
|-----------|---------------|--------|
| Envelope | `ipc/envelope.rs` | re-export |
| Command / Query / Event / Stream | only `Pattern` enum variants | new typed wrappers, each `impl Into<Envelope>` |
| **Mailbox** | missing | new trait: `async fn send(&self, e: Envelope)`, `async fn recv(&self) -> Option<Envelope>` — backed by existing `CommunicationBus` |

Typed wrappers make "sending a command" vs "sending a query" distinct at the type level while lowering to
the same `Envelope` wire format.

---

## 4. RFC-014 Agora (`crates/agora/`)

The 8th subsystem. Session-isolated shared cognitive workspace between Cognit and Corpus.

### 4a. Module layout

RFC-014 suggests 9 modules; consolidated to 6 (observation/artifact/context merged into blackboard — all
key-value shared data; api merged into lib.rs).

```
crates/agora/src/
├── lib.rs          # re-export public API
├── workspace/      # per-session container aggregating all below
│   └── mod.rs      #   Workspace { session_id, blackboard, attention, task_graph, scratchpad, trace }
├── blackboard/     # key-value shared area (hypotheses, evidence, intermediate conclusions)
├── attention/      # attention state (current focus, priorities)
├── scratchpad/     # migrated from mnemosyne/scope.rs (incl. RetentionPolicy)
├── task_graph/     # sub-task dependencies + status
├── trace/          # reasoning trace + tool outputs + sub-agent results
└── ops.rs          # AgoraOps implementation
```

### 4b. AgoraOps trait (defined in `fabric/src/ops.rs`, alongside existing ops traits)

Aligned with existing ops style (`#[async_trait]`, `serde_json::Value` for inter-subsystem data):

```rust
#[async_trait]
pub trait AgoraOps: Send + Sync {
    async fn publish(&self, session: &str, key: &str, value: Value) -> Result<()>;  // write blackboard
    async fn recall(&self, session: &str, key: &str) -> Result<Option<Value>>;      // read blackboard
    async fn update(&self, session: &str, patch: Value) -> Result<()>;              // update workspace
    async fn snapshot(&self, session: &str) -> Result<Value>;                       // snapshot (debug/commit)
    async fn clear(&self, session: &str) -> Result<()>;                             // clear session workspace
}
```

### 4c. Lifecycle & Mnemosyne relationship

```
Input → Context Build → Recall Injection → Reasoning → ... → Reflection → Commit to Mnemosyne
                ↑ Mnemosyne.recall() → Agora.publish()      Agora.snapshot() → Mnemosyne.store()
```

- **Recall injection:** when Cognit builds context, `MnemosyneOps::recall()` results are `publish()`-ed into
  the Agora blackboard.
- **Commit:** at turn end, Agora's key outputs (experience/decision) are `snapshot()`-ed and stored via
  `MnemosyneOps::store()`.
- Agora itself is pure in-memory; the session workspace is cleared at session end (unless a scratchpad's
  `RetentionPolicy` specifies archival).

### 4d. Dependency direction

```
agora → fabric   (primitives, AgoraOps trait; Workspace uses cognitive objects)
```

Agora depends only on fabric — not mnemosyne/cognit — to avoid cycles. recall/commit orchestration lives in
the executive layer; Agora only provides workspace read/write.

---

## 5. Integration

### 5a. CoreSystems wiring

Add one field to `executive/core/core_systems.rs`:

```rust
pub agora: Arc<AgoraRegistry>,   // manages per-session Workspace instances
```

Constructed in `handler/init.rs`. Accessed via `self.subsystems.agora` (following the Group A CoreSystems
pattern).

### 5b. Orchestration hooks (executive layer, minimal intrusion)

Two hook points added to the existing turn flow (**without changing cognit/mnemosyne internals**):
- **Turn start:** `mnemosyne.recall()` → `agora.publish()` injects into blackboard.
- **Turn end:** `agora.snapshot()` → `mnemosyne.store()` commits.

**Minimal-viable scope:** Agora exists, supports read/write/snapshot/clear. Deep integration of reasoning
trace / task graph is deferred to a later increment.

---

## 6. Delivery — 3 PRs

Follows the Executive Refactor grouped-PR pattern: each PR independently verifiable and mergeable, via
feature branch → PR → CI → merge.

| PR | Content | Verification |
|----|---------|--------------|
| **C1** | RFC-017 Primitives: `fabric/src/primitives/` (cognitive + comm; add 4 cognitive objects + Command/Query/Event/Stream/Mailbox) | build + existing tests unbroken |
| **C2** | RFC-014 Agora crate: create `crates/agora/`, migrate Scratchpad, implement AgoraOps + Workspace/blackboard/attention/task_graph/trace | new-crate unit tests + workspace tests |
| **C3** | Integration: wire into CoreSystems + init, add recall/commit hook points (minimal-viable) | full workspace test + clippy + fmt |

### Verification standard (each PR)

```bash
cargo build --workspace && \
cargo test --workspace && \
cargo clippy --workspace -- -D warnings && \
cargo fmt --all --check
```

---

## 7. Post-absorption architecture

```
crates/
├── fabric/       (+ primitives/ — canonical vocabulary)
├── mnemosyne/
├── executive/    (+ agora wiring + recall/commit hooks)
├── cognit/
├── dasein/
├── corpus/
├── metacog/
├── agora/        🆕 8th subsystem — shared cognitive workspace
├── interact/
└── bin/
```
