# M-H — Unify the Bifurcated Memory Subsystem — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Design-only handoff — do not execute product changes until the design-only gate is lifted.**

**Goal:** Collapse the two parallel memory subsystems into one. Today the daemon runs entirely on the runtime store (`FactStore` + `RecallMemory` + `CoreMemory` + `AutoMemory` + `EpisodicMemory`), while the cognitive `memory::MemoryRouter` (+ semantic/procedural/self backends) is wired only as a never-populated `Option` on `AletheonRuntime` and is dead on the live path. This plan makes `FactStore` the canonical governed store, deletes the dead `MemoryRouter` wiring, and demotes the unused cognitive backends behind an off-by-default feature flag — a staged, build-guarded deprecation with no daemon behavior change.

**Architecture:** This is the same "two divergent implementations" smell as M-A (compaction). It is a **follow-up to the Tier 1 Governed-Memory MVP** (`docs/plans/2026-07-01-governed-memory-mvp-plan.md`), which governs `FactStore` but does *not* resolve the bifurcation. Every claim below was re-verified against the repo on 2026-07-01 (anchors inline).

**Tech Stack:** Rust (Cargo workspace), `cargo` feature flags, `rusqlite`, existing `FactStore`/`EpisodicMemory`.

**Spec:** `docs/plans/2026-07-01-modules-roadmap-design.md` § "M-H. Unify the bifurcated memory subsystem"

**Branch:** `auro/feat/20260701-aletheon-unify-memory` (own branch per repo policy).

---

## Decision gate (PREREQUISITE — record before any code)

This plan implements **Option A** (the recommended survivor). The owner MUST record the A-vs-B decision (e.g. in the PR description or an ADR) before Phase 1.

| Option | Description | Verdict |
|---|---|---|
| **A (chosen)** | Keep `FactStore` + friends as the canonical governed daemon store (it runs, and has trust/ttl/FTS5/entities). Delete the dead `MemoryRouter` wiring from `AletheonRuntime`; demote the cognitive `MemoryRouter`/semantic/procedural/self backends behind an off-by-default `cognitive-memory` feature in the `memory` crate. Keep `EpisodicMemory` (still used by the daemon). | **RECOMMENDED** — least work; matches reality (the daemon already runs 100% on `FactStore`, and `with_memory` has zero callers). |
| B (rejected) | Invest in the cognitive `memory` crate as canonical: route the daemon through `MemoryRouter`, migrate `FactStore` data into the semantic/episodic backends. | **REJECTED** — requires data migration and re-plumbing the live per-turn path for no functional gain; the cognitive router is not even reachable today. |

**Non-goals:** Not resolved by the Tier 1 MVP. Does not delete the cognitive crate or migrate data (that would be Option B). Does not touch the ReAct-loop or compaction paths (M-A).

---

## Ground truth (verified 2026-07-01)

| Claim | Evidence |
|---|---|
| Workspace `[package]` names are `base` / `runtime` / `memory` (used in `cargo -p`) | `crates/base/Cargo.toml:2`, `crates/runtime/Cargo.toml:2`, `crates/memory/Cargo.toml:2` |
| `runtime` depends on the `memory` crate | `crates/runtime/Cargo.toml:20` `memory = { path = "../memory" }` |
| `AletheonRuntime` holds `memory: Option<Arc<MemoryRouter>>` | `crates/runtime/src/core/orchestrator.rs:34` (`use` at `:16`) |
| The field is initialized to `None` and only set by `with_memory` | `orchestrator.rs:49` (`memory: None`), `orchestrator.rs:88-91` (`with_memory` builder) |
| **`with_memory` has ZERO callers** — the router is never populated | `rg "with_memory\b"` → only the definition at `orchestrator.rs:89`; no call site anywhere |
| The only use of `self.memory` (`recall_for_prompt`) is therefore dead on the live path | `orchestrator.rs:345-353` (`if let Some(ref memory) = self.memory { … recall_for_prompt … }`) |
| The daemon builds `AletheonRuntime::new(...)` WITHOUT `.with_memory(...)` | `crates/runtime/src/impl/daemon/handler/mod.rs:335` |
| **Outside the `memory` crate, `MemoryRouter` is referenced ONLY by** `orchestrator.rs:16` | `rg "MemoryRouter"` → all other hits are inside `crates/memory/**` |
| The daemon's canonical per-turn store is `FactStore` | `handler/mod.rs:139` (field), `:223` (`FactStore::open`), `:249`/`:667`; recall/injection at `chat.rs:121` (`fs.search_facts(&query, None, 0.15, 4)`) |
| `EpisodicMemory` (from the `memory` crate) IS still used by the daemon (reflections/awareness) | `handler/mod.rs:43` (`use memory::episodic::EpisodicMemory`), `:104` (field), `:353` (`EpisodicMemory::new`); `memory_pipeline.rs:18,84` |
| `runtime` imports from the `memory` crate = only `MemoryRouter` (dead) + `episodic::EpisodicMemory` | `rg "use memory::"` → `orchestrator.rs:16`, `memory_pipeline.rs:18`, `handler/mod.rs:43` |
| Cognitive backends live behind `MemoryRouter` and are used nowhere outside the `memory` crate | `SemanticMemory` `backends/semantic/schema.rs:166`, `ProceduralMemory` `backends/procedural.rs:19`, `SelfMemory` `backends/self_memory.rs:22`; `router.rs:14-17` consumes all four |
| `MemoryRouter` impls the `MemoryBackend` trait | `memory/src/ops/router.rs:276`; trait def at `crates/base/src/include/memory.rs:141` |
| Runtime store components (unchanged canonical set) | `FactStore` `fact_store/mod.rs:91`, `RecallMemory` `recall_memory.rs:17`, `CoreMemory` `core_memory.rs:44`, `AutoMemory` `auto_memory.rs:40` |
| `memory` crate re-exports (flat API) mix episodic + cognitive | `memory/src/lib.rs:13-14`; `ops/mod.rs:12,18`; `backends/mod.rs:9-16` |
| The `memory` crate has no `[features]` section yet | `crates/memory/Cargo.toml` (only per-dep `features = [...]`) |

> **Conclusion on the roadmap's claim:** `MemoryRouter` is not merely "referenced only as an `Option`" — it is **strictly dead on the live path**: `with_memory` (the only setter) has zero callers, so the field is always `None`, and the sole consumer (`orchestrator.rs:345`) never fires in the daemon. Option A is therefore a low-risk removal + demotion, not a migration.

---

## File map

| File | Change |
|---|---|
| `crates/runtime/src/core/orchestrator.rs` | Remove the dead cognitive wiring: `use memory::MemoryRouter` (`:16`), the `memory: Option<Arc<MemoryRouter>>` field (`:34`), its `None` init (`:49`), the `with_memory` builder (`:88-91`), and the `if let Some(ref memory) = self.memory { … }` recall block (`:345-353`). |
| `crates/runtime/tests/memory_bifurcation_guard.rs` | **New** guard test: source-scan asserting the live runtime no longer references `MemoryRouter` and the daemon never calls `with_memory`. |
| `crates/memory/Cargo.toml` | Add `[features]` with an off-by-default `cognitive-memory = []`. |
| `crates/memory/src/lib.rs` | Feature-gate the cognitive re-exports (`MemoryRouter`, `Semantic/Procedural/Self`, `router`, `consolidation`); keep `EpisodicMemory` + `episodic` in the default build. |
| `crates/memory/src/ops/mod.rs` | Feature-gate `pub mod router;` / `pub mod consolidation;` and their re-exports. |
| `crates/memory/src/backends/mod.rs` | Feature-gate `pub mod {procedural, self_memory, semantic};` and their re-exports; keep `episodic` default. |
| `crates/memory/tests/feature_gating_guard.rs` | **New** guard test: cognitive backends are gated, `EpisodicMemory` stays default. |
| `crates/runtime/tests/factstore_canonical_recall.rs` | **New** regression test: daemon recall via `FactStore` still returns injected facts. |

Each phase ends with build + commit. Default checks:
`cargo build --workspace` and (per phase) the named `cargo test` command.

---

## Phase 1 — Prove `MemoryRouter` is dead, then delete its wiring

### Task 1: Remove the never-populated `MemoryRouter` from `AletheonRuntime`

**Files:**
- Add: `crates/runtime/tests/memory_bifurcation_guard.rs`
- Modify: `crates/runtime/src/core/orchestrator.rs`

- [ ] **Step 1: Write the failing guard test**

```rust
// crates/runtime/tests/memory_bifurcation_guard.rs
//! Locks in Option A: the live runtime/daemon must not wire the cognitive
//! MemoryRouter. Source-scan guards so a future edit that re-introduces the
//! bifurcation fails CI. (Search strings live here, never in the scanned files.)

#[test]
fn live_runtime_does_not_reference_cognitive_memory_router() {
    let orchestrator = include_str!("../src/core/orchestrator.rs");
    assert!(
        !orchestrator.contains("MemoryRouter"),
        "Option A: the cognitive MemoryRouter must not be wired into AletheonRuntime"
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
        "Option A: the daemon must build AletheonRuntime without a MemoryRouter"
    );
    // The daemon still uses EpisodicMemory directly — that is the kept path.
    assert!(
        handler.contains("EpisodicMemory"),
        "EpisodicMemory remains the daemon's reflection store (kept under Option A)"
    );
}
```

- [ ] **Step 2: Run — expected FAIL**

Run: `cargo test -p runtime --test memory_bifurcation_guard`
Expected: FAIL — `orchestrator.rs` still contains `MemoryRouter` / `with_memory` (`:16,:34,:88-91`). (The second test already passes; the first fails until Step 3.)

- [ ] **Step 3: Delete the dead wiring from `orchestrator.rs`**

Remove exactly these, verified dead by the ground-truth grep (no external caller of `with_memory`, so `self.memory` is always `None`):

1. The import at `:16`:
```rust
use memory::MemoryRouter;
```
2. The struct field at `:34`:
```rust
    memory: Option<Arc<MemoryRouter>>,
```
3. The field initializer at `:49` (inside `new()`):
```rust
            memory: None,
```
4. The builder at `:88-91`:
```rust
    /// Attach a MemoryRouter for prompt-time memory recall.
    pub fn with_memory(mut self, memory: Arc<MemoryRouter>) -> Self {
        self.memory = Some(memory);
        self
    }
```
5. The dead recall block at `:345-353`:
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

> If `Arc` becomes unused after removing the field, drop it from the `use` line too (the compiler's `unused_imports` warning will flag it). Do NOT touch `take_awareness_signals` (`orchestrator.rs:369`) or the `EpisodicMemory` doc-comments — those describe the kept daemon path.

- [ ] **Step 4: Run — expected PASS + workspace build**

Run: `cargo test -p runtime --test memory_bifurcation_guard`
Then: `cargo build --workspace`
Expected: both green. Removing an always-`None` branch cannot change runtime behavior; the workspace still builds because the only external `MemoryRouter` reference was the line just deleted.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/core/orchestrator.rs crates/runtime/tests/memory_bifurcation_guard.rs
git commit -m "refactor(runtime): remove dead MemoryRouter wiring from AletheonRuntime (M-H Option A)"
```

---

## Phase 2 — Demote the cognitive backends behind an off-by-default feature

### Task 2: Gate `MemoryRouter` + semantic/procedural/self behind `cognitive-memory`

**Files:**
- Modify: `crates/memory/Cargo.toml`, `crates/memory/src/lib.rs`, `crates/memory/src/ops/mod.rs`, `crates/memory/src/backends/mod.rs`
- Add: `crates/memory/tests/feature_gating_guard.rs`

Rationale (verified): outside the `memory` crate, nothing consumes the cognitive backends — `runtime` uses only `memory::episodic::EpisodicMemory` (`handler/mod.rs:43`, `memory_pipeline.rs:18`) after Phase 1. So the router + semantic/procedural/self modules can move off the default build. `EpisodicMemory` and the generic `activation`/`decay`/`schema` helpers stay default (the router, under the feature, still references `activation` — `router.rs:13`).

- [ ] **Step 1: Write the failing guard test**

```rust
// crates/memory/tests/feature_gating_guard.rs
//! Option A: cognitive backends (MemoryRouter/semantic/procedural/self) are
//! demoted behind the off-by-default `cognitive-memory` feature, while
//! EpisodicMemory stays in the default build for the daemon.

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
    // Compiles/links under default features == EpisodicMemory is not gated.
    let dir = tempfile::tempdir().unwrap();
    let _mem = memory::EpisodicMemory::new(dir.path().join("episodic.db"));
}

#[cfg(feature = "cognitive-memory")]
#[test]
fn router_is_available_with_the_feature() {
    let dir = tempfile::tempdir().unwrap();
    let _router = memory::MemoryRouter::new(dir.path());
}
```

> `EpisodicMemory::new(PathBuf)` and `MemoryRouter::new(&Path)` match the real
> constructors (`backends/episodic/schema.rs:21`, `ops/router.rs:103,423`).

- [ ] **Step 2: Run — expected FAIL**

Run: `cargo test -p memory --test feature_gating_guard`
Expected: FAIL — `lib.rs` has no `#[cfg(feature = "cognitive-memory")]` yet (no `[features]` section exists, `Cargo.toml`).

- [ ] **Step 3a: Add the feature to `crates/memory/Cargo.toml`**

```toml
[features]
# Off by default: the cognitive MemoryRouter + semantic/procedural/self backends
# are not used by the live daemon (M-H Option A). Enable to build them.
default = []
cognitive-memory = []
```

- [ ] **Step 3b: Gate the re-exports in `crates/memory/src/lib.rs`**

Replace the flat re-export block (`lib.rs:13-28`) so episodic stays default and the cognitive surface is gated:

```rust
// Backward-compatible re-exports (flat API)
pub use backends::EpisodicMemory;
pub use ops::{compute_activation, ActivationEntry};
pub use ops::{apply_access_boost, compute_strength, should_forget};

#[cfg(feature = "cognitive-memory")]
pub use backends::{ProceduralMemory, SelfMemory, SemanticMemory};
#[cfg(feature = "cognitive-memory")]
pub use ops::{ConsolidationConfig, ConsolidationResult, MemoryContext, MemoryRouter, ReflectionSummary, SkillSummary};

// Sub-module re-exports for direct path access (e.g. `memory::episodic::EpisodicMemory`)
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
```

- [ ] **Step 3c: Gate the modules in `ops/mod.rs` and `backends/mod.rs`**

```rust
// crates/memory/src/ops/mod.rs — keep activation/decay/schema default; gate the rest
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

```rust
// crates/memory/src/backends/mod.rs — keep episodic default; gate the rest
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

> If any *default-built* module (e.g. `consolidation` tests, or `testing` at
> `lib.rs:30`) fails to compile because it references a now-gated backend, gate
> that reference too (add `#[cfg(feature = "cognitive-memory")]`). `consolidation`
> consumes `EpisodicMemory` (default) but is itself cognitive → it is gated above,
> so its own references resolve only when the feature is on. Let the compiler
> enumerate any stragglers; do not add new logic.

- [ ] **Step 4: Run — expected PASS + both build modes green**

```bash
cargo test -p memory --test feature_gating_guard                 # default: gating asserted, episodic works
cargo test -p memory --features cognitive-memory                 # router path compiles + its tests run
cargo build -p memory                                            # default build excludes router
cargo build -p runtime                                           # daemon uses only episodic — still builds
cargo build --workspace
```
Expected: all green. `cargo build -p runtime` proves the daemon compiles with the cognitive backends off the default path.

- [ ] **Step 5: Commit**

```bash
git add crates/memory/Cargo.toml crates/memory/src/lib.rs crates/memory/src/ops/mod.rs crates/memory/src/backends/mod.rs crates/memory/tests/feature_gating_guard.rs
git commit -m "refactor(memory): demote cognitive MemoryRouter/backends behind off-by-default cognitive-memory feature (M-H)"
```

---

## Phase 3 — `FactStore` is the sole daemon store: recall regression guard

### Task 3: Lock in FactStore-based recall so the demotion caused no regression

**Files:**
- Add: `crates/runtime/tests/factstore_canonical_recall.rs`

This phase adds no product change — it proves the kept path (the daemon's `FactStore` recall at `chat.rs:121`) still returns injected facts after Phases 1–2, so the removal/demotion is regression-free.

- [ ] **Step 1: Write the regression test**

```rust
// crates/runtime/tests/factstore_canonical_recall.rs
//! M-H Option A regression guard: after removing the cognitive MemoryRouter,
//! the daemon's canonical store (FactStore) must still recall injected facts —
//! this is the exact call the chat handler makes (chat.rs:121).

use runtime::r#impl::memory::fact_store::FactStore;

#[test]
fn factstore_remains_the_canonical_recall_store() {
    let dir = tempfile::tempdir().unwrap();
    let fs = FactStore::open(&dir.path().join("fact_store.db")).unwrap();

    // add_fact(content, category, tags, source_path, trust, tier, ttl_days)
    let id = fs
        .add_fact("aletheon recalls facts via FactStore", "general", "", "", 0.7, "semantic", 0)
        .unwrap();

    // Same signature the daemon uses: search_facts(query, category, min_trust, limit)
    let hits = fs.search_facts("FactStore", None, 0.15, 4).unwrap();
    assert!(
        hits.iter().any(|f| f.fact_id == id),
        "daemon recall via FactStore must still return injected facts after MemoryRouter demotion"
    );
}
```

> Path check: the crate root re-exports `FactStore` under `impl::memory::fact_store`
> (module tree from `handler/mod.rs:63` `use crate::r#impl::memory::fact_store::FactStore`).
> If the module is not `pub` at the crate boundary for integration tests, instead
> place this test inside `crates/runtime/src/impl/memory/fact_store/mod.rs`'s
> `#[cfg(test)]` module (same pattern as the governed-memory MVP tests) and run
> `cargo test -p runtime fact_store::`. Prefer the in-crate location if the path
> re-export is not public.

- [ ] **Step 2: Run — expected PASS**

Run: `cargo test -p runtime --test factstore_canonical_recall`
(or `cargo test -p runtime fact_store::` if placed in-crate)
Expected: PASS — `FactStore` is untouched by this plan; the test just pins its role as the sole recall store.

- [ ] **Step 3: Full workspace regression sweep**

```bash
cargo build --workspace
cargo test -p runtime
cargo test -p memory
cargo test -p memory --features cognitive-memory
```
Expected: green. `cargo test -p runtime` covers the daemon/session/fact_store suites; no recall or injection test regresses.

- [ ] **Step 4: Commit**

```bash
git add crates/runtime/tests/factstore_canonical_recall.rs
git commit -m "test(runtime): guard FactStore as the sole canonical daemon recall store (M-H)"
```

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** decision gate ↔ spec "requires explicit owner A-vs-B decision recorded before implementation"; Phase 1 (prove-then-delete dead router) ↔ "prove MemoryRouter is unused on the live path / retire or DEMOTE"; Phase 2 (feature flag) ↔ "demote the cognitive backends behind a feature flag or move them out of the default daemon wiring"; Phase 3 ↔ "leave FactStore as the sole daemon store" + "no regression in daemon recall/injection".
- **Every removal is build-guarded:** each phase ends with `cargo build --workspace` and a test; Phase 1's deletion is safe because the branch is provably dead (`with_memory` zero callers).
- **Placeholder scan:** none — exact line targets to delete, exact `#[cfg]` edits, exact `[features]` block, real constructors/signatures for tests.
- **Type/signature consistency:** `EpisodicMemory::new(PathBuf)` (`schema.rs:21`), `MemoryRouter::new(&Path)` (`router.rs:103`), `FactStore::open(&Path)` (`mod.rs:97`), `add_fact(content,category,tags,source_path,trust,tier,ttl_days)` (`query.rs:14`), `search_facts(query,Option<&str>,f64,usize)` (`query.rs:47`) all verified against source.
- **Roadmap correction recorded:** `MemoryRouter` is *stronger-than-claimed* dead — not just an unused `Option` but one with no setter caller; the daemon uses `FactStore` (not "only `EpisodicMemory`") for per-turn recall, while `EpisodicMemory` is kept for reflections. Ground-truth table reflects this.

## Risks / notes for the implementer

- **Decision gate is mandatory** — do not start Phase 1 until the owner records "Option A" (PR description or ADR). If the owner picks B, this plan does not apply.
- **Phase 1 is behavior-neutral by construction** — you are deleting an `if let Some(...)` whose condition is always `false` on the live path (`with_memory` has zero callers). If `cargo build --workspace` breaks after the deletion, something *did* populate the field — stop and re-audit `rg "with_memory"` before proceeding.
- **`EpisodicMemory` must stay in the default build** — the daemon depends on it (`handler/mod.rs:43,353`, `memory_pipeline.rs`). Do NOT gate `backends::episodic` or `EpisodicMemory`. Only the router + semantic/procedural/self + consolidation are gated.
- **Compiler-driven gating** — after adding `#[cfg]`, the default build may surface a straggler reference into a gated module (e.g. from `testing` at `lib.rs:30` or a default-built helper). Gate that reference; never add logic to make it compile.
- **`cognitive-memory` feature is preserved, not deleted** — Option A demotes, it does not remove the cognitive crate. The backends still compile and test under `--features cognitive-memory`, keeping the door open for a future Option-B revisit without data loss.
- **No data migration** — `FactStore` data is untouched; the cognitive backends were never populated on the live path, so there is nothing to migrate (that risk only exists under the rejected Option B).
- **Does not resolve M-A** (compaction bifurcation) — separate module; and this plan assumes the Tier 1 Governed-Memory MVP has landed or is independent (it only reads `FactStore`, never conflicts).
