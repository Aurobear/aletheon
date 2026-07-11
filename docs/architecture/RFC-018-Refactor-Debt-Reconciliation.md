# RFC-018 Refactor-Debt Reconciliation

> **Status:** Active roadmap. Captures the design debt left by the Executive Refactor
> (RFC-010~013) and the Agora+Primitives absorption (RFC-014/017), and stages the
> work to pay it down plus the gpt.md future extensions.
>
> **Source:** architecture review 2026-07-10 (4 parallel researchers, findings
> cross-verified against code — several subagent claims were corrected; see §2 notes).

---

## 1. Meta-finding: two architectures coexist

The Executive Refactor did the **structural** moves — crates renamed
(base→fabric, memory→mnemosyne, runtime→executive), modules relocated, `CoreSystems`
bundle created, `fabric::ops` traits + `fabric::primitives` defined. It stopped before
the **semantic** migration — actually routing subsystem calls through the new ops
traits and speaking the primitive vocabulary.

Result: the scaffolding for the target architecture exists but is **not load-bearing**.
The system runs on the *old* layer (concrete types, `include/*Ops`, `ModuleId` old
names) while the *new* layer (`ops.rs`, `primitives`, renamed crates) sits mostly
unused beside it. Most findings below are facets of this one gap.

---

## 2. Findings (ranked, verified)

### ✅ D1 — Two parallel trait vocabularies *(resolved 2026-07-11)*
`fabric::ops` defined `CognitOps / DaseinOps / MnemosyneOps / CorpusOps` (Group A).
`fabric::include/*` defines `BrainCoreOps / SelfFieldOps / MemoryBackend / BodyRuntime`
— the ones actually implemented and used.
- Original evidence: `cognit/src/core/brain_core_ops.rs:19` impls `BrainCoreOps`;
  `fabric/src/ops.rs:16` `CognitOps` had no implementor.
- **Re-audit finding:** this was never a live duplication. Of the seven traits in
  `ops.rs`, six had **zero implementors and zero consumers** — `CognitOps`,
  `MnemosyneOps`, `CorpusOps`, `ops::DaseinOps` (a same-named-but-distinct trait from
  the live phenomenological `fabric::dasein::DaseinOps`), plus the unused harness pair
  `CognitiveHarness` / `ToolExecutor`. Only `AgoraOps` was wired (`agora/src/ops.rs`,
  called in `executive/.../chat.rs`). The `include/*` set was always the sole live
  contract layer.
- **Resolution:** canonical home is `fabric::include`. Deleted the six dead traits and
  removed `crates/fabric/src/ops.rs` entirely; moved the one live trait to
  `fabric::include::agora`. Deleting `ops::DaseinOps` also removed the `DaseinOps`
  name collision (a partial D3 fix). Zero behaviour change; `cargo build/test/clippy
  --workspace` green.

### 🟡 D2 — Primitive vocabulary was decorative *(first boundary typed 2026-07-11)*
`fabric::primitives::cognitive` (Hypothesis/Evidence/Narrative/Commitment) and
`::comm` (Command/Query/Event/Stream/Mailbox) originally had **zero consumers**
outside fabric — the dictionary existed; nothing spoke it.
- **Resolved on the hottest live boundary:** the Agora tool-result trace. A tool result
  is now constructed as a typed `Evidence` (`Evidence::from_tool_result`) and recorded
  via the new `AgoraOps::record_evidence` default method (lowers to `trace(_, "evidence",
  _)`); `executive/.../chat.rs` uses it in place of the hand-rolled JSON blob. To make the
  consumer half real, `Workspace::snapshot()` now serializes full trace entries (not just
  `trace_len`), so the persisted snapshot carries the typed object round-trip
  (`agora::ops` test `record_evidence_survives_snapshot_as_typed`).
- **Deliberately left as `Value`:** the generic blackboard (`publish`/`recall`/`update`)
  is schema-flexible by design, and Cognit does not yet produce `Hypothesis`/`Plan` as
  first-class runtime objects. Remaining boundaries get typed as real producers appear —
  not preemptively (that would re-create the "unused type" problem). The pattern
  (`record_*` default method + typed producer + snapshot round-trip) is now established.

### 🟡 D3 — Naming drift *(ModuleId done 2026-07-11; module/type renames pending)*
`ModuleId` predated the 7-subsystem model. **Re-audit corrected two assumptions:** it is
used in ~22 routing sites (not 39), and it is **never persisted to disk** — envelopes only
cross the in-process bus / same-build unix socket, so a rename is compiler-checked with no
protocol-version migration needed in practice.
- **Done — `ModuleId`:** variants renamed to the crate/subsystem names —
  `Brain→Cognit`, `SelfField→Dasein`, `Memory→Mnemosyne`, `Body→Corpus`, `Meta→Metacog`,
  `Runtime→Executive`. `Perception` kept (a live perception-event routing endpoint, not a
  crate); `Agora` **not** added (accessed directly as `Arc<dyn AgoraOps>`, never routed to
  — YAGNI).
- **Done — `cognit` rename:** the one clean module/trait pair — `include/brain.rs` →
  `include/cognit.rs`, trait `BrainCoreOps` → `CognitOps`, and the cognit-crate internals
  (`BrainCore` → `CognitCore`, `BrainCoreConfig` → `CognitCoreConfig`,
  `brain_core_ops.rs` → `cognit_ops.rs`).
- **Pending:** `AletheonRuntime`/`RuntimeConfig` type renames in executive — mechanical,
  compiler-checked.
- **Blocked / won't rename:** the other module+trait pairs can't align cleanly:
  `include/self_field` → `dasein` **collides** with the existing `fabric::dasein`
  (phenomenological) module — same collision as `SelfFieldOps→DaseinOps`; and renaming
  `memory`/`body`/`runtime`/`meta` modules while their traits (`MemoryBackend` — a
  descriptive backend with 6 impls; `BodyRuntime`; `RuntimeOps`/`MetaRuntimeOps` which are
  metacog's, not executive's) can't rename would only create module/trait mismatches.
  The provenance enums `IntentSource::Brain` / `ConflictSource::Brain` keep "Brain" — a
  separate concept (reasoning-as-source), not the renamed trait.

### 🟠 D4 — Cross-layer coupling (memory/self depend on reasoning)
`mnemosyne → cognit` (`compressor/mod.rs:7-8`: `CompactorTrait`, `LlmProvider`) and
`→ corpus` (`pruner`); `dasein → cognit` (`llm_bridge.rs:6`) and `→ corpus`
(`security/runner.rs`).
- Root cause: `LlmProvider` lives in `cognit`, so anyone needing an LLM client must
  depend on the reasoner. The `CompactorTrait` broke one cycle; the LLM coupling
  remains.
- Problem: memory and self shouldn't depend on the reasoner. `LlmProvider` belongs in
  `fabric` (or a standalone `llm` crate) so all subsystems can use it neutrally.

### 🟡 D5 — Executive not yet minimal (known intermediate state)
`CoreSystems` still bundles ~28 concrete subsystem fields
(`executive/src/core/core_systems.rs:46-99`); `handle_chat` was a single ~1080-line
method orchestrating fact/memory/skill/hook/loop/evolution inline. This was explicitly
the documented "intermediate step"; the refactor renamed but did not finish the
God-object decomposition.
- **In progress (2026-07-11):** three seams extracted, `handle_chat` **1080→658 lines**:
  - *seam 1* — pre-turn injection cluster (keyword-skill / fact-recall / core-memory /
    skill-suggestion / stale-decay), private methods on `RequestHandler`.
  - *seam 2* — post-turn phases (PostTurn hooks / auto-memory / reflection scoring +
    storage / post-evolution / Agora snapshot commit), private methods on `RequestHandler`.
  - *seam 3* — the per-tool execution pipeline (PreTool → OnMemoryRecall → approval →
    SelfField → guarded runner → PerfCounter → StormBreaker → PostTool) extracted from the
    ~150-line inline `execute_tool` closure into `TurnToolExecutor`
    (`handler/tool_executor.rs`), adapted to the harness's `Fn(&str,&str,&Value)->Fut`
    executor param via a thin `Arc<Self>` wrapper.
  Extraction proceeds one seam at a time. What remains inline in `handle_chat`: the
  control-flow-bearing pre-turn gate/hook parts, the session-manager/turn-count
  bookkeeping, the `tokio::spawn` react task, and the `tokio::select!` event/approval loop
  — the genuinely tangled orchestration, which is legitimately `handle_chat`'s runner role.
- **Issue #3 first field (2026-07-11):** `CoreSystems.agora` is now `Arc<dyn AgoraOps>`
  (was `Arc<AgoraRegistry>`) — the first concrete subsystem field moved behind a trait
  object, proving the God-object can be incrementally dyn-ified and letting agora be
  mocked/swapped. The remaining ~27 fields do **not** convert cleanly yet: their concrete
  types expose rich APIs the narrow `include/*` traits don't (e.g.
  `FactStore::search_facts_governed` is not on `MemoryBackend`), so each needs a
  per-subsystem trait-widening pass before it can go behind `dyn`. That is the long tail of
  issue #3, done one subsystem at a time — not a big-bang.

### ⚪ D6 — Placement debates (not clearly wrong)
`orchestration/`, `coordinator.rs`, `goal/ObjectiveStore` live in executive. Verdict:
multi-agent **orchestration** is a legitimate Executive/Supervisor concern — leave it.
`goal/ObjectiveStore` (goal/plan state) is more arguably Cognit's — revisit later.

---

## 3. Future-readiness gaps (gpt.md)

| Area | Current reality | Gap | Priority |
|------|-----------------|-----|----------|
| **Harness graphs** | `HarnessKind` enum + `build_harness` factory select the harness by config (`cognit/src/harness/mod.rs`; no `dyn` trait — see D1), but only the `linear` ReAct harness is implemented | 2nd harness (Research/Coding/Robot) is now additive: add an enum variant + factory arm, no executive-core edits | 🟠 high |
| **Mnemosyne background services** | consolidation/decay/activation are pure fns, **never scheduled**; Replay/Dream/Association/Forgetting absent | memory never consolidates or ages → bloat; no long-term continuity | 🟠 high |
| **Agora shared workspace** | only `turn_input` published; tool outputs / sub-agent results **not** written; snapshot **logged, not persisted** (`chat.rs:1144`) | reasoning trace lost on restart; blackboard near-empty | 🟡 medium-high |
| **Corpus capability layering** | flat Tools+Skills+Hooks; no Capability composition | can't compose/recompose tools | ⚪ low (YAGNI until needed) |

---

## 4. Staged roadmap

Ordered by value/risk. Each phase is independently shippable; every change goes via PR
(direct push to dev/main is ruleset-blocked) with full CI.

### Phase 1 — Agora persistence *(bugfix; doing now)*
Additive and low-risk; fixes real incomplete behavior (reasoning trace lost on
restart). In the tool-execution path, publish each tool result (and sub-agent result)
to the Agora trace; at turn end persist `agora.snapshot()` to Mnemosyne via
`MemoryBackend::store()` instead of only logging. Closes the RFC-014 §5b deferral.

> **Mnemosyne background scheduler (was 1b) — moved to Phase 3.5 (deferred).**
> Scheduling `consolidate`/`decay` is not the low-risk additive change first assumed:
> `consolidate` (`mnemosyne/src/ops/consolidation.rs:50`) is behind the off-by-default
> `cognitive-memory` feature, so a naive scheduler either does nothing in the live
> daemon or requires enabling untested feature-gated paths. Deserves its own design
> (which services run in the default build, on what trigger, with what concurrency).

### Phase 2 — Harness factory *(done)*
Executive selects the harness via the `HarnessKind` enum + `build_harness` factory
(`cognit/src/harness/mod.rs`), keyed by the `harness_kind` config field; the hardcoded
`ReActLoop` construction now runs through the factory. A `dyn CognitiveHarness` trait
was *not* used — `run()` is generic over its executor and so not object-safe (see D1) —
so a second harness is added as a new enum variant + factory arm, not a trait impl.

### Phase 3.5 — Mnemosyne background services *(deferred)*
A `BackgroundTaskScheduler` running the memory-maintenance services that are available
in the default build, on turn-completion or timer events, wired at daemon startup.
Design must first settle which of consolidate/decay/activation/replay run without the
`cognitive-memory` feature and their trigger/concurrency model.

### Phase 3 — Trait-vocabulary reconciliation *(D1 done 2026-07-11; D2 still open)*
**D1 resolved** — but in the *opposite* direction to what this section originally
recommended. The re-audit (see D1 above) showed the `fabric::ops` traits were dead
scaffolding, not a live competing vocabulary, so the correct move was to **keep
`fabric::include` as canonical and delete `ops.rs`** rather than adopt the ops traits.
No big-bang was needed; it was a single dead-code-deletion + one-trait-relocation PR.

**D2 first boundary typed (2026-07-11):** the Agora tool-result trace now speaks the
RFC-017 vocabulary via `Evidence` + `AgoraOps::record_evidence` (see D2 above). The
`record_*`-default-method pattern is established; remaining boundaries get typed as real
producers appear, not preemptively. The generic blackboard stays `Value` by design.

### Phase 4 — Decouple LlmProvider *(deferred)*
Resolve D4: move `LlmProvider` out of `cognit` into `fabric` (or a standalone `llm`
crate) so mnemosyne/dasein no longer depend on the reasoner for an LLM client.

### Phase 5 — Naming alignment *(deferred, low priority)*
Resolve D3: rename `ModuleId` variants + `include/` modules + `AletheonRuntime` to the
7-subsystem vocabulary. Wire-format change → protocol-version bump + migration.

### Not planned — Corpus capability decomposition
YAGNI until a concrete composition need appears.

---

## 5. Doing now vs deferred

- **This effort (via /workflow):** Phase 1 (Agora persistence) and Phase 2 (Harness factory).
- **Deferred to future RFCs:** Phase 3 (vocabulary reconciliation), Phase 3.5
  (Mnemosyne background services), Phase 4 (LLM decouple), Phase 5 (naming). Larger
  and/or higher-blast-radius; documented here so they are not lost.
