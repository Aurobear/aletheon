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

### 🔴 D1 — Two parallel trait vocabularies
`fabric::ops` defines `CognitOps / DaseinOps / MnemosyneOps / CorpusOps` (Group A,
mostly unused). `fabric::include/*` defines `BrainCoreOps / SelfFieldOps /
MemoryBackend / BodyRuntime` — the ones actually implemented and used.
- Evidence: `cognit/src/core/brain_core_ops.rs:19` impls `BrainCoreOps`; `fabric/src/ops.rs:16` `CognitOps` has no implementor. `AgoraOps` is the one `ops.rs` trait that *is* wired (`agora/src/ops.rs`, called in `executive/.../chat.rs:11`).
- Problem: every feature must be reconciled against two abstractions. Pick one.

### 🔴 D2 — Primitive vocabulary is decorative
`fabric::primitives::cognitive` (Hypothesis/Evidence/Narrative/Commitment) and
`::comm` (Command/Query/Event/Stream/Mailbox) have **zero consumers** outside fabric.
Ops-trait methods cross boundaries as `serde_json::Value`, not typed primitives
(13 of 16 methods in `fabric/src/ops.rs`).
- Problem: RFC-017's contract ("every subsystem communicates using these primitives")
  is unenforced. The dictionary exists; nothing speaks it.

### 🟠 D3 — Naming drift, live in the wire format
`ModuleId { Brain, SelfField, Memory, Body, Meta, Runtime, Perception }`
(`fabric/src/ipc/envelope.rs:11-19`) is used in **39 routing sites**. It predates the
7-subsystem model: old names, includes a non-subsystem (`Perception`), lacks
`Agora`/`Cognit`. `include/` modules (`brain/memory/body/runtime/self_field`) and
`AletheonRuntime`/`RuntimeConfig` in executive likewise lag the rename.
- Problem: readers can't tell whether "runtime"/"memory"/"brain" means the concept or
  the renamed subsystem. Changing the wire enum needs a protocol-version bump.

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
(`executive/src/core/core_systems.rs:46-99`); `chat.rs:225-380` orchestrates
fact/memory/skill/hook inline. This was explicitly the documented "intermediate step";
the refactor renamed but did not finish the God-object decomposition.

### ⚪ D6 — Placement debates (not clearly wrong)
`orchestration/`, `coordinator.rs`, `goal/ObjectiveStore` live in executive. Verdict:
multi-agent **orchestration** is a legitimate Executive/Supervisor concern — leave it.
`goal/ObjectiveStore` (goal/plan state) is more arguably Cognit's — revisit later.

---

## 3. Future-readiness gaps (gpt.md)

| Area | Current reality | Gap | Priority |
|------|-----------------|-----|----------|
| **Harness graphs** | `CognitiveHarness` trait exists (`fabric/src/ops.rs`), but only `linear` ReAct impl; executive `orchestrator.rs:44` hardcodes construction | 2nd harness (Research/Coding/Robot) requires editing executive core | 🔴 highest future value |
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
`MnemosyneOps::store()` instead of only logging. Closes the RFC-014 §5b deferral.

> **Mnemosyne background scheduler (was 1b) — moved to Phase 3.5 (deferred).**
> Scheduling `consolidate`/`decay` is not the low-risk additive change first assumed:
> `consolidate` (`mnemosyne/src/ops/consolidation.rs:50`) is behind the off-by-default
> `cognitive-memory` feature, so a naive scheduler either does nothing in the live
> daemon or requires enabling untested feature-gated paths. Deserves its own design
> (which services run in the default build, on what trigger, with what concurrency).

### Phase 2 — Harness factory *(extension; doing now)*
Make executive select a `CognitiveHarness` via a registry/factory keyed by config, so a
second harness can be registered without touching executive core. Uses the existing
trait; move the hardcoded `ReActLoop` construction behind the factory.

### Phase 3.5 — Mnemosyne background services *(deferred)*
A `BackgroundTaskScheduler` running the memory-maintenance services that are available
in the default build, on turn-completion or timer events, wired at daemon startup.
Design must first settle which of consolidate/decay/activation/replay run without the
`cognitive-memory` feature and their trigger/concurrency model.

### Phase 3 — Trait-vocabulary reconciliation *(deferred — large, staged separately)*
Resolve D1+D2: converge on **one** boundary vocabulary. Recommended direction: adopt
the RFC-017-aligned `fabric::ops` traits as canonical, implement them as thin adapters
over the existing subsystem cores (as `AgoraOps` already demonstrates), and begin
typing the hottest boundary payloads with `fabric::primitives` instead of
`serde_json::Value`. High blast radius — deserves its own RFC and incremental PRs; do
NOT big-bang.

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
