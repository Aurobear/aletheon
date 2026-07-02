# M-D — Self-Evolution Loop Wiring — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking. **Design-only handoff — do not execute product changes until the design-only gate is lifted.**

**Goal:** Make the self-evolution loop **safe and bounded** rather than remove-or-add it. Contrary to the roadmap's original "runtime never calls metacog" framing, the loop is *already wired* — but it runs **unconditionally (default-ON)**, it **never rolls back** a rejected/failed candidate, and the roadmap's named decision symbols (`MetaCognition::decide()` / `EvolutionAction`) are **dead code**. This plan (1) puts the whole loop behind a **default-OFF config flag**, (2) guarantees a **rollback on every sandbox/eval failure**, and (3) wires the dead `EvolutionAction::TriggerEvolution` signal in as the trigger gate.

**Architecture:** The loop today: `chat.rs` post-turn → `AletheonRuntime::post_evolution` → `EvolutionCoordinator::post_turn` → (turn-count/failure heuristic) → `MorphogenesisPipeline::run(intent)` → `generate_candidate → sandbox_test → evaluate → migrate` over the **declarative `Genome`** (care/boundary/memory specs — *not* Rust code). This plan makes that loop no-op unless a config flag is on, makes the pipeline call `rollback()` whenever a candidate is generated but not adopted, and lets an `EvolutionAction` decide whether the pipeline runs. Migration stays limited to declarative genome fields via the existing `MutationSpec.allowed_targets`; **no code/topology self-mutation** (non-goal).

**Tech Stack:** Rust, `tokio`, `async-trait`, `anyhow`, `serde`; existing `EvolutionCoordinator`, `MorphogenesisPipeline`, `DefaultMetaRuntime`, `MetaCognition`.

**Spec:** `docs/plans/2026-07-01-modules-roadmap-design.md` § "M-D. Self-Evolution loop wiring".

**Branch:** `auro/feat/20260701-aletheon-self-evolution` (own branch per repo policy).

---

## Ground truth (verified 2026-07-01)

| Claim | Anchor |
|---|---|
| Cargo package names are `base` / `runtime` / `metacog` (concept map ABI/Runtime/Meta) | `crates/base/Cargo.toml` `name = "base"`; `crates/runtime/Cargo.toml` `name = "runtime"`; `crates/metacog/Cargo.toml` `name = "metacog"` |
| `MetaCognition::decide()` returns an `EvolutionAction` from mood/turn heuristics | `crates/metacog/src/core/meta_cognition.rs:58` (`pub fn decide(&self, ctx: &DaseinContext, turn: usize) -> EvolutionAction`) |
| `EvolutionAction` variants: `Observe`, `TriggerEvolution { intents: Vec<MutationIntent> }`, `AdjustDasein`, `InjectReflection` | `crates/metacog/src/core/meta_cognition.rs:28-33` |
| **`MetaCognition` / `decide()` are dead** — referenced only by their own unit test | `rg "MetaCognition|\.decide\("` → only `crates/metacog/src/core/meta_cognition.rs` + `core/mod.rs:5` re-export |
| The self-modification trait is **`MetaRuntimeOps`** (roadmap said `MetaRuntime`), on `base` | `crates/base/src/include/meta.rs:67` `pub trait MetaRuntimeOps: Subsystem` |
| Its methods are all `async fn(&self)`: `generate_candidate(&MutationIntent)->RuntimeCandidate`, `sandbox_test(&RuntimeCandidate)->TestResult`, `evaluate(&RuntimeCandidate,&TestResult)->Evaluation`, `migrate(&RuntimeCandidate)->MigrationResult`, `rollback()->()` | `crates/base/src/include/meta.rs:69-88` |
| `DefaultMetaRuntime` implements `MetaRuntimeOps`; `generate_candidate` saves a rollback snapshot, `rollback()` restores it | `crates/metacog/src/core/traits.rs:39` (struct), `:120` (impl), `:151-162` (generate+snapshot), `:195-205` (rollback) |
| `RollbackManager::rollback` pops the last snapshot; **errors if none** | `crates/metacog/src/impl/meta_runtime/rollback.rs:50-63` |
| **Pipeline never calls `rollback()`** — non-`Adopt` just returns `success:false`; sandbox/eval errors `?`-propagate with no rollback | `crates/metacog/src/impl/morphogenesis/pipeline.rs:24-100` (no `rollback` token in file) |
| Loop is **already wired**: orchestrator exposes `post_evolution(...)` | `crates/runtime/src/core/orchestrator.rs:97-133` |
| Called **unconditionally** post-turn (no flag) | `crates/runtime/src/impl/daemon/handler/chat.rs:799` (`state.runtime.post_evolution(...)`) |
| Coordinator attached **unconditionally** (no flag) | `crates/runtime/src/impl/daemon/handler/mod.rs:337-348` (`with_evolution(evo_config)` + `MorphogenesisPipeline::new`) |
| Coordinator triggers on turn-count / failure and builds intents via `MutationIntentGenerator::from_reflections` — **`decide()` is bypassed** | `crates/runtime/src/core/evolution_coordinator.rs:176-188` (trigger), `:303-335` (`run_evolution`) |
| `EvolutionConfig` has **no `enabled`/gate field** | `crates/runtime/src/core/evolution_coordinator.rs:26-47` |
| Migration operates on the declarative `Genome`; allowed targets are declared, not code | `crates/runtime/tests/evolution_integration.rs:43-47` (`MutationSpec.allowed_targets = ["care.priorities","mutation.config"]`, `require_sandbox`) |
| **No `PermissionManager` exists** anywhere (Tier 2a not yet present) | `rg "PermissionManager" crates` → no matches |
| Reusable test harness (`MockMetaRuntime` impl of `MetaRuntimeOps`, `minimal_genome()`) already exists | `crates/runtime/tests/evolution_integration.rs:28-158` |
| `MutationIntent` fields: `target: String`, `change: serde_json::Value`, `reason: String`, `reversible: bool` | `crates/base/src/include/self_field.rs:117-122` |

> **Anchor drift corrected vs. the roadmap brief:** (a) trait is `MetaRuntimeOps`, not `MetaRuntime`; (b) files live under `crates/metacog/…` and `crates/runtime/…`, not `metacog/…`/`runtime/…`; (c) the loop is *already* wired end-to-end (`post_evolution` at `chat.rs:799`) — the real defect is that it is **default-ON, rollback-less, and bypasses `decide()`**, so this plan is a **safety-gating** plan, not a "connect the wire" plan.

---

## Design decisions (made for this plan)

1. **Gate at the coordinator, default-OFF.** Add `enabled: bool` to `EvolutionConfig` (defaults `false`). `post_turn` early-returns an inert `EvolutionSummary` (`evolution_triggered = false`, no pipeline call) when disabled. This is the single choke point every trigger path (`post_turn`, `post_turn_with_stimmung`, `run_evolution`) already flows through — lowest blast radius. The daemon reads the flag from `AppConfig` (default false) so a fresh checkout evolves nothing.
2. **Rollback is the pipeline's responsibility.** Because `generate_candidate` snapshots *before* returning (`traits.rs:155-159`), the pipeline can safely call `rollback()` whenever a candidate was generated but **not** adopted (sandbox failed, evaluation rejected, or a later step errored). Add `rolled_back: bool` to `PipelineResult`. Rollback is best-effort (logged, never masks the original error).
3. **Wire the dead `EvolutionAction` in as the trigger gate.** Add `EvolutionCoordinator::run_action(action, meta)` that runs the pipeline **only** on `EvolutionAction::TriggerEvolution { intents }` (using those intents), and is a no-op for `Observe`/`AdjustDasein`/`InjectReflection`. This makes `EvolutionAction::TriggerEvolution` a live signal. Feeding a real `DaseinContext` into `MetaCognition::decide()` from the dasein subsystem is deferred (see Risks) — `decide()`'s heuristics are exercised by unit test, and the *consumption* side is what this plan lands.
4. **Permission gate deferred to Tier 2a.** No `PermissionManager` exists yet, so the loop is gated on the config flag **alone**. A `// TODO(Tier 2a): also gate on PermissionManager` marker documents the follow-up.

---

## File map

| File | Change |
|---|---|
| `crates/runtime/src/core/evolution_coordinator.rs` | add `enabled: bool` to `EvolutionConfig` (default `false`); early-return in `post_turn` when disabled; add `run_action(&EvolutionAction, &MorphogenesisPipeline<M>)` |
| `crates/metacog/src/impl/morphogenesis/pipeline.rs` | call `rollback()` on non-adopt / failure; add `rolled_back: bool` to `PipelineResult` |
| `crates/runtime/src/core/config/agent.rs` (or `config/infra.rs`) | add a serde-defaulted `EvolutionSettings { enabled: bool }` (default false) surfaced on `AppConfig` |
| `crates/runtime/src/impl/daemon/handler/mod.rs` | map the config flag into `EvolutionConfig.enabled` at `:337-344`; Tier-2a TODO |
| `crates/runtime/tests/evolution_integration.rs` | new tests reusing `MockMetaRuntime` (disabled no-op; rollback-on-failure; `run_action` gating) |

Each phase ends with build + commit. Default checks: `cargo test -p runtime` and `cargo test -p metacog`; `cargo build -p runtime`.

---

## Phase 1 — Default-OFF config gate (the critical safety fix)

### Task 1: `EvolutionConfig.enabled` + `post_turn` no-op when disabled

**Files:** Modify `crates/runtime/src/core/evolution_coordinator.rs` and `crates/runtime/tests/evolution_integration.rs`.

- [ ] **Step 1: Write the failing test** (disabled coordinator never touches the pipeline)

Add to `crates/runtime/tests/evolution_integration.rs` (reuses `MockMetaRuntime`, `MorphogenesisPipeline`, `EvolutionConfig`, already imported at `:11-18`):

```rust
#[tokio::test]
async fn disabled_coordinator_is_a_noop() {
    let tmp = tempfile::tempdir().unwrap();
    let config = EvolutionConfig {
        enabled: false,          // NEW: default-off gate
        trigger_every_n_turns: 1, // would trigger every turn if enabled
        trigger_on_failure: true,
        window_size: 20,
        lineage_dir: tmp.path().to_path_buf(),
    };
    let coordinator = EvolutionCoordinator::new(config).unwrap();
    let (mock, gen_calls, mig_calls) = MockMetaRuntime::new();
    let pipeline = MorphogenesisPipeline::new(mock);

    let summary = coordinator
        .post_turn("task", "error output", false, 5, 2, 1000, 1, &pipeline, vec![])
        .await
        .unwrap();

    assert!(!summary.evolution_triggered, "disabled loop must not trigger");
    assert_eq!(gen_calls.load(std::sync::atomic::Ordering::SeqCst), 0, "no candidate generated");
    assert_eq!(mig_calls.load(std::sync::atomic::Ordering::SeqCst), 0, "no migration");
    // still reflects (observation is safe and free of side effects)
    assert!(summary.reflected);
}
```

- [ ] **Step 2: Run — expected FAIL** (`EvolutionConfig` has no `enabled` field → does not compile).

Run: `cargo test -p runtime --test evolution_integration disabled_coordinator_is_a_noop`
Expected: compile error `no field 'enabled' on type 'EvolutionConfig'`.

- [ ] **Step 3: Add the field + gate**

In `evolution_coordinator.rs`, add the field to `EvolutionConfig` (struct at `:26`) and default it OFF (`Default` at `:38`):

```rust
pub struct EvolutionConfig {
    /// Master switch. When false, the whole loop is inert (default).
    pub enabled: bool,
    pub trigger_every_n_turns: usize,
    pub trigger_on_failure: bool,
    pub window_size: usize,
    pub lineage_dir: PathBuf,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            enabled: false, // HIGH-risk autonomy: OFF unless explicitly enabled
            trigger_every_n_turns: 5,
            trigger_on_failure: true,
            window_size: 20,
            lineage_dir: PathBuf::from("/var/lib/aletheon/lineage"),
        }
    }
}
```

Early-return in `post_turn` (before the reflect/window/trigger block at `:157`):

```rust
// HIGH-risk autonomy gate. TODO(Tier 2a): also require PermissionManager approval.
if !self.config.enabled {
    return Ok(EvolutionSummary {
        reflected: false,
        reflection_id: None,
        evolution_triggered: false,
        pipeline_results: Vec::new(),
        lineage_entries_added: 0,
        awareness_entries: signals_to_awareness(&awareness_signals),
    });
}
```

> `post_turn_with_stimmung` delegates to `post_turn` (`:230-242`), so it inherits the gate automatically — no separate change.

- [ ] **Step 4: Run — expected PASS.** Then the full suite to prove the *enabled* paths still pass unchanged (they construct `EvolutionConfig { enabled: true, .. }`):

Run: `cargo test -p runtime --test evolution_integration`
Expected: the three existing tests (`failure_triggers_evolution`, periodic, window) must be updated to set `enabled: true` and still pass; `disabled_coordinator_is_a_noop` passes.

> **Fix-forward note:** the three existing tests at `:167`, and their siblings, construct `EvolutionConfig { .. }` positionally-by-field; add `enabled: true` to each so the *enabled* behavior remains covered.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/core/evolution_coordinator.rs crates/runtime/tests/evolution_integration.rs
git commit -m "feat(evolution): gate self-evolution loop behind default-off EvolutionConfig.enabled"
```

### Task 2: Surface the flag on `AppConfig` and map it in the daemon

**Files:** Modify `crates/runtime/src/core/config/agent.rs` (new `EvolutionSettings`), `crates/runtime/src/core/config/mod.rs` (add field to `AppConfig`), `crates/runtime/src/impl/daemon/handler/mod.rs`.

- [ ] **Step 1: Add a serde-defaulted settings struct** (in `agent.rs`, mirroring `PerceptionConfig` at `:139-160`):

```rust
/// Self-evolution loop settings. Default OFF (HIGH-risk autonomy).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionSettings {
    #[serde(default)] // bool default = false
    pub enabled: bool,
}

impl Default for EvolutionSettings {
    fn default() -> Self { Self { enabled: false } }
}
```

Export it alongside the others in `config/mod.rs:8`, and add to `AppConfig` (`config/mod.rs:26`):

```rust
#[serde(default)]
pub evolution: EvolutionSettings,
```

- [ ] **Step 2: Map the flag into `EvolutionConfig` in the daemon**

In `handler/mod.rs` at the `evo_config` construction (`:337-344`), thread the flag through and mark the Tier 2a follow-up:

```rust
// Wire EvolutionCoordinator for post-turn self-evolution.
// HIGH-risk autonomy: OFF unless config.evolution.enabled is true.
// TODO(Tier 2a): additionally gate migrations behind PermissionManager.
let evo_config = EvolutionConfig {
    enabled: config.evolution.enabled,
    trigger_every_n_turns: 10,
    trigger_on_failure: true,
    window_size: 20,
    lineage_dir: data_dir.join("lineage"),
};
runtime = runtime.with_evolution(evo_config)?;
```

- [ ] **Step 3: Build** `cargo build -p runtime` — expected: compiles (`config.evolution.enabled` resolves; `AppConfig` gains a defaulted field so existing configs still parse).

- [ ] **Step 4: Config-parse guard** — confirm the shipped `config/default.toml` still parses **and** leaves evolution off by default (no `[evolution]` section ⇒ `#[serde(default)]` ⇒ `enabled = false`). If the repo has a config-parse test (see the Tier-0 plan's `shipped_default_config_is_startable_shaped`), re-run it:

Run: `cargo test -p runtime config`
Expected: PASS, evolution defaults off.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/core/config/agent.rs crates/runtime/src/core/config/mod.rs crates/runtime/src/impl/daemon/handler/mod.rs
git commit -m "feat(config): add default-off [evolution] enabled flag wired into the daemon"
```

---

## Phase 2 — Guaranteed rollback on sandbox/eval failure

### Task 3: `MorphogenesisPipeline::run` rolls back every non-adopted candidate

**Files:** Modify `crates/metacog/src/impl/morphogenesis/pipeline.rs`; test in the same file.

- [ ] **Step 1: Write the failing test** (rejected candidate ⇒ rollback fires)

Add a `#[cfg(test)]` module to `pipeline.rs` with a local mock that **rejects** and counts rollbacks (self-contained; does not depend on the runtime test crate):

```rust
#[cfg(test)]
mod tests {
    use super::*;
    use base::genome::*;
    use base::meta::{Recommendation, Subsystem, SubsystemHealth};
    use base::Version;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use std::sync::Arc;
    use async_trait::async_trait;

    fn genome() -> Genome {
        Genome {
            topology: Topology { subsystems: vec![] },
            identity: IdentitySpec { name: "t".into(), description: "t".into(), self_model: "t".into() },
            boundary: BoundarySpec { rules: vec![] },
            care: CareSpec { priorities: vec![] },
            memory: MemorySpec { backends: vec![], compaction_strategy: "none".into() },
            mutation: MutationSpec { allowed_targets: vec!["care.priorities".into()], require_sandbox: false, require_self_field_approval: false },
            lifecycle: LifecycleSpec { auto_compact: false, health_check_interval_secs: 60, max_idle_time_secs: 3600 },
        }
    }

    struct RejectingMeta { rollbacks: Arc<AtomicUsize> }

    #[async_trait]
    impl Subsystem for RejectingMeta {
        fn name(&self) -> &str { "reject-meta" }
        fn version(&self) -> Version { Version::new(0, 1, 0) }
        async fn init(&mut self, _c: &base::SubsystemContext) -> anyhow::Result<()> { Ok(()) }
        async fn shutdown(&mut self) -> anyhow::Result<()> { Ok(()) }
        async fn health(&self) -> SubsystemHealth { SubsystemHealth::Healthy }
    }

    #[async_trait]
    impl MetaRuntimeOps for RejectingMeta {
        async fn read_genome(&self) -> anyhow::Result<Genome> { Ok(genome()) }
        async fn generate_candidate(&self, _i: &MutationIntent) -> anyhow::Result<RuntimeCandidate> {
            Ok(RuntimeCandidate { id: uuid::Uuid::new_v4(), genome: genome(), changes: vec!["c".into()], generated_at: chrono::Utc::now() })
        }
        async fn sandbox_test(&self, _c: &RuntimeCandidate) -> anyhow::Result<TestResult> {
            Ok(TestResult { passed: false, tests_run: 1, tests_passed: 0, tests_failed: 1, failures: vec!["boom".into()], elapsed_ms: 1 })
        }
        async fn evaluate(&self, _c: &RuntimeCandidate, _t: &TestResult) -> anyhow::Result<Evaluation> {
            Ok(Evaluation { score: 0.0, strengths: vec![], weaknesses: vec!["failed".into()], recommendation: Recommendation::Reject })
        }
        async fn migrate(&self, _c: &RuntimeCandidate) -> anyhow::Result<MigrationResult> {
            panic!("migrate must not be called on a rejected candidate")
        }
        async fn rollback(&self) -> anyhow::Result<()> { self.rollbacks.fetch_add(1, Ordering::SeqCst); Ok(()) }
        fn current_version(&self) -> Version { Version::new(0, 1, 0) }
    }

    #[tokio::test]
    async fn rejected_candidate_is_rolled_back() {
        let rollbacks = Arc::new(AtomicUsize::new(0));
        let meta = RejectingMeta { rollbacks: rollbacks.clone() };
        let pipeline = MorphogenesisPipeline::new(meta);
        let intent = MutationIntent {
            target: "care.priorities".into(),
            change: serde_json::json!({ "action": "adjust" }),
            reason: "test".into(),
            reversible: true,
        };
        let result = pipeline.run(&intent).await.unwrap();
        assert!(!result.success, "rejected candidate must not count as success");
        assert!(result.rolled_back, "rejected candidate must be rolled back");
        assert_eq!(rollbacks.load(Ordering::SeqCst), 1, "rollback() must fire exactly once");
    }
}
```

- [ ] **Step 2: Run — expected FAIL** (`PipelineResult` has no `rolled_back` field; no rollback is performed).

Run: `cargo test -p metacog morphogenesis::pipeline::tests::rejected_candidate_is_rolled_back`

- [ ] **Step 3: Add rollback + `rolled_back` to the pipeline**

Extend `PipelineResult` (`pipeline.rs:103-110`) with `pub rolled_back: bool`, and change the migrate/skip logic (`:56-99`) so any generated-but-not-adopted candidate is rolled back:

```rust
// Step 4: Migrate if recommended, else roll back the pre-generation snapshot.
let (migration, rolled_back) = match &evaluation.recommendation {
    base::meta::Recommendation::Adopt
    | base::meta::Recommendation::PartialAdopt { .. } => {
        let result = self.meta_runtime.migrate(&candidate).await?;
        tracing::info!("Migration successful: {} -> {}", result.from_version, result.to_version);
        (Some(result), false)
    }
    other => {
        // Candidate was generated (snapshot saved by generate_candidate); undo it.
        tracing::info!("Not adopting ({:?}) — rolling back candidate {}", other, candidate.id);
        let rolled_back = match self.meta_runtime.rollback().await {
            Ok(()) => true,
            Err(e) => { tracing::warn!("rollback after non-adopt failed: {e}"); false }
        };
        (None, rolled_back)
    }
};

let success = migration.is_some();
```

Add `rolled_back` to the returned `PipelineResult { .. }` (`:93-99`).

> **Sandbox/eval error path:** `sandbox_test`/`evaluate` currently `?`-propagate (`:39`, `:48`). To honor "sandbox failure aborts cleanly *with* rollback", wrap those two calls so an `Err` triggers `rollback()` before returning the error. Minimal form — after `generate_candidate` succeeds, bind `let candidate = ...;` then:
>
> ```rust
> let test_result = match self.meta_runtime.sandbox_test(&candidate).await {
>     Ok(t) => t,
>     Err(e) => { let _ = self.meta_runtime.rollback().await; return Err(e); }
> };
> let evaluation = match self.meta_runtime.evaluate(&candidate, &test_result).await {
>     Ok(v) => v,
>     Err(e) => { let _ = self.meta_runtime.rollback().await; return Err(e); }
> };
> ```
>
> This keeps the "candidate generated ⇒ always either migrated or rolled back" invariant on every exit path.

- [ ] **Step 4: Run — expected PASS.** Then the whole crate to catch `PipelineResult` construction sites (only one, at `:93`) and the runtime integration tests that read `PipelineResult`:

Run: `cargo test -p metacog` and `cargo test -p runtime --test evolution_integration`
Expected: PASS. The runtime `MockMetaRuntime` (adopts) drives the migrate branch, so `rolled_back == false` there — the existing adopt tests still pass.

- [ ] **Step 5: Commit**

```bash
git add crates/metacog/src/impl/morphogenesis/pipeline.rs
git commit -m "feat(morphogenesis): roll back every non-adopted or failed candidate; add rolled_back"
```

---

## Phase 3 — Wire the dead `EvolutionAction` in as the trigger gate

### Task 4: `EvolutionCoordinator::run_action` runs the pipeline only on `TriggerEvolution`

**Files:** Modify `crates/runtime/src/core/evolution_coordinator.rs`; test in `crates/runtime/tests/evolution_integration.rs`.

- [ ] **Step 1: Write the failing test** (`Observe` ⇒ no-op; `TriggerEvolution` ⇒ pipeline runs its intents)

`EvolutionAction` is constructible directly (variants at `meta_cognition.rs:28-33`), so this test exercises the live consumption path without needing a full `DaseinContext`:

```rust
use metacog::core::meta_cognition::EvolutionAction;

#[tokio::test]
async fn run_action_gates_on_trigger_evolution() {
    let tmp = tempfile::tempdir().unwrap();
    let coordinator = EvolutionCoordinator::new(EvolutionConfig {
        enabled: true,
        trigger_every_n_turns: 0,
        trigger_on_failure: false,
        window_size: 20,
        lineage_dir: tmp.path().to_path_buf(),
    }).unwrap();
    let (mock, gen_calls, _mig) = MockMetaRuntime::new();
    let pipeline = MorphogenesisPipeline::new(mock);

    // Observe → nothing runs.
    let n = coordinator.run_action(&EvolutionAction::Observe, &pipeline).await.unwrap();
    assert_eq!(n, 0);
    assert_eq!(gen_calls.load(std::sync::atomic::Ordering::SeqCst), 0);

    // TriggerEvolution → the pipeline runs once per intent.
    let action = EvolutionAction::TriggerEvolution {
        intents: vec![base::MutationIntent {
            target: "care.priorities".into(),
            change: serde_json::json!({ "topic": "safety", "delta": 0.1 }),
            reason: "meta-cognition trigger".into(),
            reversible: true,
        }],
    };
    let ran = coordinator.run_action(&action, &pipeline).await.unwrap();
    assert_eq!(ran, 1, "one intent ⇒ one pipeline run");
    assert_eq!(gen_calls.load(std::sync::atomic::Ordering::SeqCst), 1);
}
```

- [ ] **Step 2: Run — expected FAIL** (`run_action` does not exist).

Run: `cargo test -p runtime --test evolution_integration run_action_gates_on_trigger_evolution`

- [ ] **Step 3: Implement `run_action`**

Add to `impl EvolutionCoordinator` (near `run_evolution` at `:303`). It honors the same `enabled` gate and reuses the existing per-intent pipeline call + lineage recording pattern (`:317-332`):

```rust
use metacog::core::meta_cognition::EvolutionAction;

/// Consume an EvolutionAction from meta-cognition. Runs the morphogenesis
/// pipeline once per intent **only** for `TriggerEvolution`, and only when the
/// loop is enabled. Returns the number of pipeline runs performed.
/// TODO(Tier 2a): additionally gate on PermissionManager before migrating.
pub async fn run_action<M: MetaRuntimeOps>(
    &self,
    action: &EvolutionAction,
    meta: &MorphogenesisPipeline<M>,
) -> Result<usize> {
    if !self.config.enabled {
        return Ok(0);
    }
    let intents = match action {
        EvolutionAction::TriggerEvolution { intents } => intents,
        _ => return Ok(0), // Observe / AdjustDasein / InjectReflection: no evolution
    };
    let mut ran = 0;
    for intent in intents {
        let result = meta.run(intent).await?;
        if result.success {
            if let Some(ref migration) = result.migration {
                self.lineage.record(&migration.to_version, Some(&migration.from_version), &result.message);
            }
            self.apply_care_mutation(intent).await;
        }
        ran += 1;
    }
    Ok(ran)
}
```

> Requires `metacog::core::meta_cognition::EvolutionAction` to be re-exported/reachable — it already is via `crates/metacog/src/core/mod.rs:5`. If the `core` module is not `pub` at the crate root, add `pub use core::meta_cognition::EvolutionAction;` to `crates/metacog/src/lib.rs` (verify the existing re-export list there first).

- [ ] **Step 4: Run — expected PASS.** Full suite: `cargo test -p runtime --test evolution_integration`.

- [ ] **Step 5: Commit**

```bash
git add crates/runtime/src/core/evolution_coordinator.rs crates/runtime/tests/evolution_integration.rs
git commit -m "feat(evolution): consume EvolutionAction::TriggerEvolution as the gated trigger"
```

---

## Self-review checklist (done at plan-write time)

- **Spec coverage:** default-off gate (Task 1–2) ↔ M-D "default-off, config-gated, permission-gated"; guaranteed rollback (Task 3) ↔ "every migration has a rollback; sandbox failure aborts cleanly"; `EvolutionAction` trigger (Task 4) ↔ "runs decide→(if TriggerEvolution) generate→sandbox_test→evaluate→migrate|rollback". Non-goal (no code/topology self-mutation) preserved: migration stays over the declarative `Genome` via `MutationSpec.allowed_targets`.
- **Placeholder scan:** none — every step has real Rust + exact `cargo` commands. Tests compile against real `MetaRuntimeOps`/`RuntimeCandidate`/`TestResult`/`Evaluation`/`MigrationResult`/`MutationIntent`/`EvolutionAction`/`EvolutionConfig` types (anchored above).
- **Type consistency:** `run_action` uses the verified async `MetaRuntimeOps` methods and the real `EvolutionAction::TriggerEvolution { intents: Vec<MutationIntent> }` shape; `PipelineResult` field add is Debug-derive-safe; `EvolutionConfig.enabled` threads config → coordinator → gate.
- **Drift corrected:** trait name (`MetaRuntimeOps`), crate paths (`crates/…`), and the "already-wired-but-unsafe" reality are reflected throughout rather than the roadmap's "never called" framing.

## Risks / notes for the implementer

- **This is HIGH-risk autonomy — do Phase 1 first and never reorder.** The gate must land before any behavior is exercised; the shipped `config/default.toml` must remain evolution-off (no `[evolution]` section ⇒ `serde(default)` ⇒ `false`).
- **`decide()` still needs a real `DaseinContext` feed to be fully live.** This plan lands the *consumption* side (`run_action`) and keeps `decide()`'s heuristics under its own unit test (`meta_cognition.rs:111`). Wiring the dasein subsystem's live `DaseinContext` into a `decide()` call at the post-turn site is a **follow-up** — it requires threading `DaseinContext` (5 snapshot fields, `dasein/mod.rs:249-255`) into `chat.rs`'s post-turn block; do not fake a context.
- **Tier 2a PermissionManager is absent** (`rg "PermissionManager"` = empty). Every gate here is the config flag alone; the `TODO(Tier 2a)` markers flag where a permission check must be added once that subsystem exists. Until then, "enabled" is the *only* thing standing between the daemon and an autonomous migration — treat turning it on as a deliberate operator action.
- **Rollback depends on a saved snapshot.** `DefaultMetaRuntime::generate_candidate` snapshots *before* returning (`traits.rs:155-159`), so a rollback after a rejected candidate has something to pop. If a future `MetaRuntimeOps` impl skips the pre-generation snapshot, `rollback()` will error (`rollback.rs:59-61`) — the pipeline logs and sets `rolled_back = false` rather than panicking, but such an impl violates the invariant and should be rejected in review.
- **Existing enabled-path tests must be updated, not deleted.** The three tests in `evolution_integration.rs` (`:164+`) exercise the *enabled* loop; add `enabled: true` to each so coverage of the live path is retained.
- **Do not migrate code.** `migrate` here rewrites the declarative `Genome` (care/boundary/memory specs). Any intent whose `target` is outside `Genome.mutation.allowed_targets` must be rejected upstream (existing mechanism) — keep it that way; extending mutation to source/topology is explicitly out of scope for M-D.
