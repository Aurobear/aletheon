# M-D + M-E Consolidated Implementation Design

**Date:** 2026-07-02
**Status:** Design (design-only gate in effect -- no product code changes)
**Source plans:**
- `docs/plans/2026-07-01-md-self-evolution-loop-plan.md`
- `docs/plans/2026-07-01-me-subagent-lifecycle-plan.md`
**Roadmap:** `docs/plans/2026-07-01-modules-roadmap-design.md` § M-D, M-E

This document is the **single source of truth** for the M-D Self-Evolution Loop and M-E SubAgent Lifecycle implementation. It replaces the two separate plans. Every claim has been verified against the codebase (see Section 1).

---

## 1. Verified Ground Truth Table

All claims from both original plans were checked against the actual codebase at
`/home/rj001/Bear-ws/work/aletheon` on branch `auro/feat/20260701-aletheon-governed-memory-design`.

### 1.1 M-D Self-Evolution Loop Claims

| # | Claim | Anchor | Status |
|---|---|---|---|
| 1 | `MetaCognition::decide()` returns `EvolutionAction` from mood/turn heuristics | `crates/metacog/src/core/meta_cognition.rs:58` | **MATCH** -- `pub fn decide(&self, ctx: &DaseinContext, turn: usize) -> EvolutionAction` |
| 2 | `EvolutionAction` variants: `Observe`, `TriggerEvolution { intents }`, `AdjustDasein`, `InjectReflection` | `crates/metacog/src/core/meta_cognition.rs:28-33` | **MATCH** |
| 3 | `MetaCognition` / `decide()` are dead code -- referenced only by unit test | grep `crates/` for `MetaCognition\|\.decide\(` | **MATCH** -- zero external references outside `meta_cognition.rs` + `core/mod.rs:5` re-export |
| 4 | `MetaRuntimeOps` trait at `crates/base/src/include/meta.rs:67` | `pub trait MetaRuntimeOps: Subsystem` | **MATCH** -- 7 methods: `read_genome`, `generate_candidate`, `sandbox_test`, `evaluate`, `migrate`, `rollback`, `current_version` |
| 5 | `DefaultMetaRuntime` struct at `traits.rs:39`, impl at `:120`, snapshot at `:151-162` | `crates/metacog/src/core/traits.rs` | **MATCH** -- `save_snapshot` called at `:155-159` before candidate return |
| 6 | `RollbackManager::rollback` pops snapshot; errors if none | `crates/metacog/src/impl/meta_runtime/rollback.rs:50-63` | **MATCH** -- `pub async fn rollback(&self) -> Result<Genome>`, bails with "No previous genome version to roll back to" |
| 7 | Pipeline never calls `rollback()` | `crates/metacog/src/impl/morphogenesis/pipeline.rs:24-100` | **MATCH** -- `run()` calls `generate_candidate`, `sandbox_test`, `evaluate`, `migrate`; no rollback token anywhere |
| 8 | `post_evolution()` at `crates/runtime/src/core/orchestrator.rs:97-133` | drains signals, calls `coord.post_turn()`, updates genome config | **MATCH** |
| 9 | Called unconditionally post-turn | `crates/runtime/src/impl/daemon/handler/chat.rs:796-811` | **MATCH** -- `state.runtime.post_evolution(...)` with no `if enabled` guard |
| 10 | Coordinator attached unconditionally | `crates/runtime/src/impl/daemon/handler/mod.rs:337-348` | **MATCH** -- `EvolutionConfig { trigger_every_n_turns: 10, .. }` then `runtime.with_evolution(evo_config)` |
| 11 | Trigger logic: turn-count `% n == 0` or `trigger_on_failure && !success` | `crates/runtime/src/core/evolution_coordinator.rs:176-182` | **MATCH** -- `(n > 0 && *counter % n == 0) || on_fail` |
| 12 | `EvolutionConfig` has NO `enabled` field | `crates/runtime/src/core/evolution_coordinator.rs:26-47` | **MATCH** -- 4 fields: `trigger_every_n_turns`, `trigger_on_failure`, `window_size`, `lineage_dir` |
| 13 | `MutationIntent` fields: `target`, `change`, `reason`, `reversible` | `crates/base/src/include/self_field.rs:117-122` | **MATCH** |
| 14 | Reusable test harness exists | `crates/runtime/tests/evolution_integration.rs:28-158` | **MATCH** -- `minimal_genome()`, `MockMetaRuntime` (Subsystem + MetaRuntimeOps impl) |
| 15 | `PerceptionConfig` pattern for serde-defaulted config | `crates/runtime/src/core/config/agent.rs:139-159` | **MATCH** -- `#[derive(Debug, Clone, Serialize, Deserialize)]` + `Default` |
| 16 | `PipelineResult` has NO `rolled_back` field | `crates/metacog/src/impl/morphogenesis/pipeline.rs:103-110` | **MATCH** -- 5 fields: `success`, `candidate`, `evaluation`, `migration`, `message` |
| 17 | Cargo package names: `base`, `runtime`, `metacog` | `crates/*/Cargo.toml` | **MATCH** |
| 18 | `post_turn` builds `ExecutionResult` before reflection | `crates/runtime/src/core/evolution_coordinator.rs:130-155` | **MATCH** |
| 19 | `post_turn_with_stimmung` delegates to `post_turn` | `crates/runtime/src/core/evolution_coordinator.rs:213-262` | **MATCH** -- delegates at `:231`, adds forced evolution at `:246-258` |
| 20 | `run_evolution` exists as private method | `crates/runtime/src/core/evolution_coordinator.rs:303-335` | **MATCH** -- generates intents, runs pipeline per intent, records lineage |
| 21 | `AppConfig` uses `#[serde(default)]` for each field | `crates/runtime/src/core/config/mod.rs:25-49` | **MATCH** -- 11 fields, all `#[serde(default)]` |

### 1.2 M-E SubAgent Lifecycle Claims

| # | Claim | Anchor | Status |
|---|---|---|---|
| 1 | `SubAgentSpawner` stores `HashMap<String, SubAgentHandle>` + `next_id` | `crates/runtime/src/core/sub_agent.rs:11-14` | **MATCH** |
| 2 | API: `spawn`, `update_status`, `remove`, `list`, `get` | `crates/runtime/src/core/sub_agent.rs:25-62` | **MATCH** -- exact signatures confirmed |
| 3 | `remove` is bare `HashMap::remove`, no teardown | `crates/runtime/src/core/sub_agent.rs:50-52` | **MATCH** |
| 4 | No live task handle stored; `spawn` is sync | `crates/runtime/src/core/sub_agent.rs:12,28-38` | **MATCH** -- map value is `SubAgentHandle` (plain data), no `JoinHandle` |
| 5 | `SubAgentStatus`: `Planning`, `Executing{current_step}`, `WaitingApproval`, `Completed{summary}`, `Failed{error}` | `crates/base/src/events/ui_event.rs:114-121` | **MATCH** |
| 6 | `SubAgentHandle` fields: `id`, `task`, `status`, `parent_turn_id`, `spawned_at_ms` | `crates/base/src/events/ui_event.rs:123-131` | **MATCH** |
| 7 | `base` re-exports `SubAgentHandle`, `SubAgentStatus` | `crates/base/src/lib.rs:127-130` | **DRIFT (minor)** -- lines 127-130 (not 127-129); symbols correct |
| 8 | Spawner owned by runtime struct (field + accessor) | `crates/runtime/src/core/orchestrator.rs:38,390-396` | **DRIFT (minor)** -- struct is `AletheonRuntime` (not `Orchestrator`); field + accessors at correct lines |
| 9 | Only external reader is `sub_agents` RPC via `.list()` | `crates/runtime/src/impl/daemon/handler/rpc.rs:729-743` | **MATCH** -- accesses `.id`, `.task`, `.status` from `a` (handle ref) |
| 10 | `SubAgentSpawner` re-exported from `core/mod.rs` | `crates/runtime/src/core/mod.rs:27` | **MATCH** -- `pub use sub_agent::SubAgentSpawner;` |
| 11 | `tokio-util` is a `runtime` dependency | `crates/runtime/Cargo.toml:29` | **MATCH** -- `tokio-util = { workspace = true }` |
| 12 | `base` depends on serde with derive | `crates/base/Cargo.toml:10` | **MATCH** -- `serde = { version = "1", features = ["derive"] }` |
| 13 | `SubAgentStatusChanged` UiEvent variant | `crates/base/src/events/ui_event.rs:196` | **MATCH** -- `SubAgentStatusChanged { agent_id: String, status: SubAgentStatus }` |
| 14 | `Default for SubAgentSpawner` hand-written | `crates/runtime/src/core/sub_agent.rs:65-69` | **MATCH** -- delegates to `Self::new()` |

---

## 2. Architecture Overview

### 2.1 M-D: Evolution Loop Flow

```
Turn End (chat.rs:799)
  |
  v
AletheonRuntime::post_evolution()  [orchestrator.rs:97]
  |
  v
EvolutionCoordinator::post_turn()  [evolution_coordinator.rs:130]
  |
  +--[NEW] Check EvolutionConfig.enabled
  |     +-- false --> return inert EvolutionSummary { evolution_triggered: false, .. }
  |     +-- true  --> continue
  |
  +-- Reflect on execution outcome (always, safe)
  +-- Add to sliding window
  +-- Check trigger condition: turn_count % N == 0 || (on_failure && !success)
  |     +-- not triggered --> return summary with evolution_triggered: false
  |     +-- triggered --> run_evolution()
  |
  v
EvolutionCoordinator::run_evolution()  [evolution_coordinator.rs:303]
  |
  +-- MutationIntentGenerator::from_reflections() --> Vec<MutationIntent>
  +-- For each intent:
  |     |
  |     v
  |   MorphogenesisPipeline::run(intent)  [pipeline.rs:24]
  |     |
  |     +-- 1. generate_candidate() ---> RuntimeCandidate  [snapshot saved here]
  |     +-- 2. sandbox_test() --------> TestResult
  |     |     [NEW: on Err --> rollback() then return Err]
  |     +-- 3. evaluate() ------------> Evaluation
  |     |     [NEW: on Err --> rollback() then return Err]
  |     +-- 4. match recommendation:
  |           Adopt / PartialAdopt --> migrate() --> success=true, rolled_back=false
  |           Reject / _ ----------> rollback() --> success=false, rolled_back=true
  |
  +-- Record successful migrations to lineage
  +-- Apply care mutations to genome config

  [NEW] EvolutionAction::TriggerEvolution path (run_action):
    MetaCognition::decide(ctx, turn) --> EvolutionAction
      +-- Observe / AdjustDasein / InjectReflection --> no-op
      +-- TriggerEvolution { intents } --> run pipeline per intent
```

### 2.2 M-E: SubAgent State Machine

```
     spawn()
       |
       v
   +---------+     transition(Running)    +---------+
   | Created | -------------------------> | Running |
   +---------+                            +---------+
       |    \                               /  |  \
       |     \                             /   |   \
       |  destroy()                  Waiting  |  Completed
       |       \                       ^      |      \
       |        v                      |      |       v
       |     [freed]                   v      |   +-----------+
       |                          +---------+  |   | Completed |
       |                          | Waiting |--+   +-----------+
       |                          +---------+          |
       |                             |   \             |
       |                             |    \            |
       |                      Running    destroy()     |
       |                           \        |         |
       v                            \       v         v
   +-------+                         \  [freed]   [freed after
   | Failed|                          \             destroy()]
   +-------+                           \
       |                                v
       |                            [freed]
       v
   [freed after
    destroy()]


   destroy() = cancel token + HashMap::remove  (entry gone, state() -> None)
   remove()  = delegates to destroy()          (stronger than old bare HashMap::remove)

   Legal transitions (can_transition_to):
     Created     -> Running | Failed | Destroyed
     Running     -> Waiting | Completed | Failed | Destroyed
     Waiting     -> Running | Completed | Failed | Destroyed
     Completed   -> Destroyed
     Failed      -> Destroyed
     Destroyed   -> (terminal, no outgoing)
```

### 2.3 Cross-Module Interaction (M-D <-> M-E)

Evolution and subagents are independent today but interact in two ways:

1. **Evolution can spawn subagents (future):** When the evolution loop generates a candidate, it may need to test it by spawning a subagent. The subagent lifecycle (`SubAgentState`, `CancellationToken`) is ready for this -- `cancel_token(id)` hands the token to the spawned task.

2. **Subagents can be evolved (future):** A subagent's trace/metrics feed into `MutationIntentGenerator::from_reflections()`, so subagent execution quality influences evolution triggers.

3. **Today's interaction:** None. Both modules operate independently in parallel:
   - Chat turn ends --> evolution loop runs (now gated)
   - LLM `agent` tool call --> subagent spawner (now lifecycle-tracked)

---

## 3. Complete Code for All Changes

### 3.1 M-D Phase 1: Default-OFF Config Gate

#### 3.1.1 `crates/runtime/src/core/evolution_coordinator.rs` -- Add `enabled` field

**Insertion point:** Modify `EvolutionConfig` struct at line 26 and its `Default` impl at line 38.

**Current code (lines 26-47):**
```rust
#[derive(Debug, Clone)]
pub struct EvolutionConfig {
    /// Trigger evolution every N turns (0 = disabled).
    pub trigger_every_n_turns: usize,
    /// Also trigger evolution after any failed turn.
    pub trigger_on_failure: bool,
    /// Maximum number of recent reflections to keep.
    pub window_size: usize,
    /// Directory for lineage persistence.
    pub lineage_dir: PathBuf,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            trigger_every_n_turns: 5,
            trigger_on_failure: true,
            window_size: 20,
            lineage_dir: PathBuf::from("/var/lib/aletheon/lineage"),
        }
    }
}
```

**Replace with:**
```rust
#[derive(Debug, Clone)]
pub struct EvolutionConfig {
    /// Master switch. When false, the whole loop is inert (default).
    /// HIGH-risk autonomy -- OFF unless explicitly enabled by the operator.
    pub enabled: bool,
    /// Trigger evolution every N turns (0 = disabled).
    pub trigger_every_n_turns: usize,
    /// Also trigger evolution after any failed turn.
    pub trigger_on_failure: bool,
    /// Maximum number of recent reflections to keep.
    pub window_size: usize,
    /// Directory for lineage persistence.
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

#### 3.1.2 `crates/runtime/src/core/evolution_coordinator.rs` -- Gate `post_turn`

**Insertion point:** In `post_turn()` method, after line 155 (`elapsed_ms,` line) and before line 157 (`// Reflect on the turn`).

**Insert BEFORE the `// Reflect on the turn` comment (after the exec struct construction):**
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

This goes between the `ExecutionResult` construction (ending at line 155, the `};` closing the struct literal) and the `// Reflect on the turn` comment at line 157. The exact insertion is: after line 155, before line 156 (blank line), before line 157.

The full `post_turn` method after change (lines 130-201):
```rust
    pub async fn post_turn<M: MetaRuntimeOps>(
        &self,
        task_summary: &str,
        output: &str,
        success: bool,
        tool_calls: usize,
        tool_errors: usize,
        elapsed_ms: u64,
        _iterations: usize,
        meta: &MorphogenesisPipeline<M>,
        awareness_signals: Vec<AwarenessSignal>,
    ) -> Result<EvolutionSummary> {
        // Build an ExecutionResult from turn metrics
        let exec = ExecutionResult {
            plan_id: Uuid::new_v4(),
            success,
            steps_completed: tool_calls.saturating_sub(tool_errors),
            steps_total: tool_calls,
            output: output.to_string(),
            error: if tool_errors > 0 {
                Some(format!("{tool_errors} tool errors"))
            } else {
                None
            },
            elapsed_ms,
        };

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

        // Reflect on the turn
        let trigger = if success {
            ReflectionTrigger::TaskComplete
        } else {
            ReflectionTrigger::Impasse
        };
        // ... rest unchanged ...
    }
```

#### 3.1.3 `crates/runtime/src/core/config/agent.rs` -- Add `EvolutionSettings`

**Insertion point:** After `PerceptionConfig` block ending at line 160, before the `AgentLoopConfig` comment at line 162.

**Insert after line 160:**
```rust
// ---------------------------------------------------------------------------
// EvolutionSettings
// ---------------------------------------------------------------------------

/// Self-evolution loop settings. Default OFF (HIGH-risk autonomy).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EvolutionSettings {
    /// Master switch for the self-evolution loop.
    /// When false (default), the loop is inert regardless of other settings.
    #[serde(default)] // bool default = false
    pub enabled: bool,
}

impl Default for EvolutionSettings {
    fn default() -> Self {
        Self { enabled: false }
    }
}
```

#### 3.1.4 `crates/runtime/src/core/config/mod.rs` -- Wire into `AppConfig`

**Two changes:**

**Change A -- Re-export (line 8):**
Current:
```rust
pub use agent::{AgentConfig, AgentLoopConfig, CircuitBreakerConfig, HooksConfig, PerceptionConfig, RuntimeConfig};
```
Replace with:
```rust
pub use agent::{AgentConfig, AgentLoopConfig, CircuitBreakerConfig, EvolutionSettings, HooksConfig, PerceptionConfig, RuntimeConfig};
```

**Change B -- Add field to `AppConfig` struct (after `perception` field, line 48 area):**
In the `AppConfig` struct (lines 26-49), add after the `perception` field (line 48-49):
```rust
    #[serde(default)]
    pub evolution: EvolutionSettings,
```

And in `Default for AppConfig` (starting line 190), add after the `perception` entry (line 203):
```rust
            evolution: EvolutionSettings::default(),
```

#### 3.1.5 `crates/runtime/src/impl/daemon/handler/mod.rs` -- Map flag to coordinator

**Insertion point:** Lines 337-344, the `evo_config` construction.

**Current code (lines 337-344):**
```rust
        // Wire EvolutionCoordinator for post-turn self-evolution
        let evo_config = EvolutionConfig {
            trigger_every_n_turns: 10,
            trigger_on_failure: true,
            window_size: 20,
            lineage_dir: data_dir.join("lineage"),
        };
        runtime = runtime.with_evolution(evo_config)?;
```

**Replace with:**
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

Note: `config` in this context is the `AppConfig` loaded at daemon startup. The `config` variable is already in scope; verify by checking the surrounding code to find the variable name. If `config` is not directly available, it may be `app_config` or similar -- adapt the field path accordingly.

---

### 3.2 M-D Phase 2: Guaranteed Rollback

#### 3.2.1 `crates/metacog/src/impl/morphogenesis/pipeline.rs` -- Rollback on failure

**Insertion points:**

**Change A -- Add `rolled_back` to `PipelineResult` (lines 103-110):**

Current:
```rust
#[derive(Debug)]
pub struct PipelineResult {
    pub success: bool,
    pub candidate: Option<RuntimeCandidate>,
    pub evaluation: Option<Evaluation>,
    pub migration: Option<MigrationResult>,
    pub message: String,
}
```

Replace with:
```rust
#[derive(Debug)]
pub struct PipelineResult {
    pub success: bool,
    pub candidate: Option<RuntimeCandidate>,
    pub evaluation: Option<Evaluation>,
    pub migration: Option<MigrationResult>,
    pub message: String,
    /// Whether a rollback was performed (candidate was generated but not adopted).
    pub rolled_back: bool,
}
```

**Change B -- Wrap sandbox/eval calls with rollback-on-error (replace lines 38-53):**

Current:
```rust
        // Step 2: Sandbox test
        let test_result = self.meta_runtime.sandbox_test(&candidate).await?;
        tracing::info!(
            "Sandbox test: {} passed, {} failed ({}ms)",
            test_result.tests_passed,
            test_result.tests_failed,
            test_result.elapsed_ms
        );

        // Step 3: Evaluate
        let evaluation = self.meta_runtime.evaluate(&candidate, &test_result).await?;
        tracing::info!(
            "Evaluation score: {:.2}, recommendation: {:?}",
            evaluation.score,
            evaluation.recommendation
        );
```

Replace with:
```rust
        // Step 2: Sandbox test -- rollback on error (candidate was already generated)
        let test_result = match self.meta_runtime.sandbox_test(&candidate).await {
            Ok(t) => t,
            Err(e) => {
                let _ = self.meta_runtime.rollback().await;
                return Err(e);
            }
        };
        tracing::info!(
            "Sandbox test: {} passed, {} failed ({}ms)",
            test_result.tests_passed,
            test_result.tests_failed,
            test_result.elapsed_ms
        );

        // Step 3: Evaluate -- rollback on error
        let evaluation = match self.meta_runtime.evaluate(&candidate, &test_result).await {
            Ok(v) => v,
            Err(e) => {
                let _ = self.meta_runtime.rollback().await;
                return Err(e);
            }
        };
        tracing::info!(
            "Evaluation score: {:.2}, recommendation: {:?}",
            evaluation.score,
            evaluation.recommendation
        );
```

**Change C -- Rollback on non-adopt recommendation (replace lines 55-78):**

Current:
```rust
        // Step 4: Migrate if recommended
        let migration = match &evaluation.recommendation {
            base::meta::Recommendation::Adopt => {
                let result = self.meta_runtime.migrate(&candidate).await?;
                tracing::info!(
                    "Migration successful: {} -> {}",
                    result.from_version,
                    result.to_version
                );
                Some(result)
            }
            base::meta::Recommendation::PartialAdopt { changes } => {
                tracing::info!("Partial adopt with {} changes -- migrating", changes.len());
                let result = self.meta_runtime.migrate(&candidate).await?;
                Some(result)
            }
            _ => {
                tracing::info!(
                    "Skipping migration -- recommendation: {:?}",
                    evaluation.recommendation
                );
                None
            }
        };
```

Replace with:
```rust
        // Step 4: Migrate if recommended, otherwise roll back the pre-generation snapshot.
        let (migration, rolled_back) = match &evaluation.recommendation {
            base::meta::Recommendation::Adopt => {
                let result = self.meta_runtime.migrate(&candidate).await?;
                tracing::info!(
                    "Migration successful: {} -> {}",
                    result.from_version,
                    result.to_version
                );
                (Some(result), false)
            }
            base::meta::Recommendation::PartialAdopt { changes } => {
                tracing::info!("Partial adopt with {} changes -- migrating", changes.len());
                let result = self.meta_runtime.migrate(&candidate).await?;
                (Some(result), false)
            }
            other => {
                // Candidate was generated (snapshot saved by generate_candidate); undo it.
                tracing::info!("Not adopting ({:?}) -- rolling back candidate {}", other, candidate.id);
                let rolled_back = match self.meta_runtime.rollback().await {
                    Ok(()) => true,
                    Err(e) => {
                        tracing::warn!("rollback after non-adopt failed: {e}");
                        false
                    }
                };
                (None, rolled_back)
            }
        };

        let success = migration.is_some();
```

**Change D -- Update PipelineResult construction (lines 80-99):**

Current:
```rust
        let success = migration.is_some();
        let message = if success {
            format!(
                "Pipeline complete. Candidate {} adopted with score {:.2}.",
                candidate.id, evaluation.score
            )
        } else {
            format!(
                "Pipeline complete. Candidate {} not adopted. Recommendation: {:?}",
                candidate.id, evaluation.recommendation
            )
        };

        Ok(PipelineResult {
            success,
            candidate: Some(candidate),
            evaluation: Some(evaluation),
            migration,
            message,
        })
```

Replace with:
```rust
        let message = if success {
            format!(
                "Pipeline complete. Candidate {} adopted with score {:.2}.",
                candidate.id, evaluation.score
            )
        } else {
            format!(
                "Pipeline complete. Candidate {} not adopted. Recommendation: {:?}",
                candidate.id, evaluation.recommendation
            )
        };

        Ok(PipelineResult {
            success,
            candidate: Some(candidate),
            evaluation: Some(evaluation),
            migration,
            message,
            rolled_back,
        })
```

Note: Remove the `let success = migration.is_some();` line from its old position (line 80) since it now lives in Change C above. The `success` binding is now assigned in Change C's match block.

---

### 3.3 M-D Phase 3: Wire `EvolutionAction` as Trigger Gate

#### 3.3.1 `crates/runtime/src/core/evolution_coordinator.rs` -- Add `run_action` method

**Insertion point:** After the `run_evolution` method ending at line 335 (the closing `}` of the method), before `apply_care_mutation` at line 341.

**Insert:**
```rust
    /// Consume an EvolutionAction from meta-cognition. Runs the morphogenesis
    /// pipeline once per intent **only** for `TriggerEvolution`, and only when
    /// the loop is enabled. Returns the number of pipeline runs performed.
    ///
    /// This is the live consumption path for `MetaCognition::decide()`.
    /// TODO(Tier 2a): additionally gate on PermissionManager before migrating.
    pub async fn run_action<M: MetaRuntimeOps>(
        &self,
        action: &metacog::core::meta_cognition::EvolutionAction,
        meta: &MorphogenesisPipeline<M>,
    ) -> Result<usize> {
        if !self.config.enabled {
            return Ok(0);
        }
        let intents = match action {
            metacog::core::meta_cognition::EvolutionAction::TriggerEvolution { intents } => intents,
            _ => return Ok(0), // Observe / AdjustDasein / InjectReflection: no evolution
        };
        let mut ran = 0;
        for intent in intents {
            let result = meta.run(intent).await?;
            if result.success {
                if let Some(ref migration) = result.migration {
                    self.lineage.record(
                        &migration.to_version,
                        Some(&migration.from_version),
                        &result.message,
                    );
                }
                self.apply_care_mutation(intent).await;
            }
            ran += 1;
        }
        Ok(ran)
    }
```

#### 3.3.2 `crates/metacog/src/lib.rs` -- Ensure `EvolutionAction` is publicly reachable

**Current (line 1-9):**
```rust
pub mod bridge;
pub mod core;
#[path = "impl/mod.rs"]
pub mod r#impl;

pub use core::traits::DefaultMetaRuntime;
pub use core::types::*;
pub use r#impl::genome::loader::GenomeLoader;
pub use r#impl::morphogenesis::pipeline::MorphogenesisPipeline;
```

**Replace with:**
```rust
pub mod bridge;
pub mod core;
#[path = "impl/mod.rs"]
pub mod r#impl;

pub use core::meta_cognition::EvolutionAction;
pub use core::traits::DefaultMetaRuntime;
pub use core::types::*;
pub use r#impl::genome::loader::GenomeLoader;
pub use r#impl::morphogenesis::pipeline::MorphogenesisPipeline;
```

Note: `core/mod.rs:5` already does `pub use meta_cognition::{MetaCognition, EvolutionAction, EvolutionDecision, SystemState};`, and `core` module is `pub` at `lib.rs:2`. However, to use `metacog::core::meta_cognition::EvolutionAction` from `runtime`, we need to verify that `metacog::core` is publicly accessible. The current `lib.rs` does `pub mod core;` (line 2), so `metacog::core::meta_cognition::EvolutionAction` should already be reachable. If not, the re-export above ensures `metacog::EvolutionAction` is a direct path.

---

### 3.4 M-E Phase 1: `SubAgentState` in `base`

#### 3.4.1 `crates/base/src/events/ui_event.rs` -- Add `SubAgentState` enum

**Insertion point:** After `SubAgentStatus` enum closing brace at line 121, before `SubAgentHandle` struct at line 124.

**Insert after line 121 (blank line), before line 123 (`/// Sub-agent handle...`):**
```rust
/// Explicit sub-agent lifecycle state (control-plane; distinct from the
/// UI-facing `SubAgentStatus`). Roadmap M-E: Created -> Running -> Waiting ->
/// Completed -> Destroyed, with Failed as an alternate terminal.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SubAgentState {
    Created,
    Running,
    Waiting,
    Completed,
    Failed,
    Destroyed,
}

impl SubAgentState {
    /// Whether a transition from `self` to `next` is legal.
    ///
    /// `Destroyed` is reachable from any non-terminal state (teardown may run at
    /// any time) but is itself terminal. `Completed`/`Failed` only advance to
    /// `Destroyed`.
    pub fn can_transition_to(&self, next: &SubAgentState) -> bool {
        use SubAgentState::*;
        matches!(
            (self, next),
            (Created, Running)
                | (Created, Failed)
                | (Created, Destroyed)
                | (Running, Waiting)
                | (Running, Completed)
                | (Running, Failed)
                | (Running, Destroyed)
                | (Waiting, Running)
                | (Waiting, Completed)
                | (Waiting, Failed)
                | (Waiting, Destroyed)
                | (Completed, Destroyed)
                | (Failed, Destroyed)
        )
    }
}
```

#### 3.4.2 `crates/base/src/events/ui_event.rs` -- Add unit tests

**Insertion point:** At end of file, before any existing test module (check if one exists; if not, append at end).

**Append at end of file:**
```rust
#[cfg(test)]
mod subagent_state_tests {
    use super::SubAgentState;

    #[test]
    fn legal_forward_path_is_allowed() {
        use SubAgentState::*;
        assert!(Created.can_transition_to(&Running));
        assert!(Running.can_transition_to(&Waiting));
        assert!(Waiting.can_transition_to(&Running));
        assert!(Running.can_transition_to(&Completed));
        assert!(Completed.can_transition_to(&Destroyed));
    }

    #[test]
    fn destroy_is_reachable_from_every_non_terminal_state() {
        use SubAgentState::*;
        for s in [Created, Running, Waiting, Completed, Failed] {
            assert!(s.can_transition_to(&Destroyed), "{s:?} -> Destroyed must be legal");
        }
    }

    #[test]
    fn illegal_transitions_are_rejected() {
        use SubAgentState::*;
        assert!(!Created.can_transition_to(&Completed), "must run before completing");
        assert!(!Completed.can_transition_to(&Running), "terminal-forward: no resurrection");
        assert!(!Destroyed.can_transition_to(&Running), "Destroyed is terminal");
        assert!(!Destroyed.can_transition_to(&Destroyed), "no self-loop on Destroyed");
    }
}
```

#### 3.4.3 `crates/base/src/lib.rs` -- Re-export `SubAgentState`

**Insertion point:** Add to the re-export block at lines 127-130.

**Current (lines 127-130):**
```rust
pub use events::ui_event::{
    AwarenessLevel, CollaborationMode, EvolutionStage, InterruptReason,
    PlanUpdate, SubAgentHandle, SubAgentStatus, UiEvent,
};
```

**Replace with:**
```rust
pub use events::ui_event::{
    AwarenessLevel, CollaborationMode, EvolutionStage, InterruptReason,
    PlanUpdate, SubAgentHandle, SubAgentState, SubAgentStatus, UiEvent,
};
```

---

### 3.5 M-E Phase 2: State Machine + `destroy()` in Spawner

#### 3.5.1 `crates/runtime/src/core/sub_agent.rs` -- Complete Rewrite

**File to be replaced entirely** at `crates/runtime/src/core/sub_agent.rs` (currently 70 lines):

```rust
//! Sub-agent spawning and tracking.
//!
//! Sub-agents are spawned by the LLM via the `agent` tool call.
//! Their status is tracked and emitted to the TUI via UiEvent, and their
//! control-plane lifecycle is enforced via `SubAgentState`.

use std::collections::HashMap;
use base::ui_event::{SubAgentHandle, SubAgentStatus};
use base::SubAgentState;
use tokio_util::sync::CancellationToken;

/// Error returned when an illegal lifecycle transition is requested.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TransitionError {
    /// No agent with the given id is tracked.
    Unknown(String),
    /// The transition `from -> to` is not legal.
    Illegal { from: SubAgentState, to: SubAgentState },
}

impl std::fmt::Display for TransitionError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            TransitionError::Unknown(id) => write!(f, "unknown sub-agent: {id}"),
            TransitionError::Illegal { from, to } => {
                write!(f, "illegal transition {from:?} -> {to:?}")
            }
        }
    }
}
impl std::error::Error for TransitionError {}

/// Internal per-agent record: the UI handle, the control-plane state, and a
/// cancellation token for in-flight work.
#[derive(Debug)]
struct SubAgentEntry {
    handle: SubAgentHandle,
    state: SubAgentState,
    cancel: CancellationToken,
}

/// Spawns and tracks sub-agents.
#[derive(Debug, Default)]
pub struct SubAgentSpawner {
    agents: HashMap<String, SubAgentEntry>,
    next_id: usize,
}

impl SubAgentSpawner {
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
            next_id: 0,
        }
    }

    /// Register a new sub-agent and return its handle. Starts in `Created`.
    pub fn spawn(&mut self, task: String, parent_turn_id: String) -> SubAgentHandle {
        self.next_id += 1;
        let id = format!("agent-{}", self.next_id);
        let handle = SubAgentHandle {
            id: id.clone(),
            task,
            status: SubAgentStatus::Planning,
            parent_turn_id,
            spawned_at_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        };
        self.agents.insert(
            id,
            SubAgentEntry {
                handle: handle.clone(),
                state: SubAgentState::Created,
                cancel: CancellationToken::new(),
            },
        );
        handle
    }

    /// Update an agent's UI status (unchanged UI-display behavior).
    pub fn update_status(&mut self, id: &str, status: SubAgentStatus) {
        if let Some(entry) = self.agents.get_mut(id) {
            entry.handle.status = status;
        }
    }

    /// Current control-plane state of an agent, if tracked.
    pub fn state(&self, id: &str) -> Option<SubAgentState> {
        self.agents.get(id).map(|e| e.state)
    }

    /// A clone of the agent's cancellation token (for wiring into spawned work).
    pub fn cancel_token(&self, id: &str) -> Option<CancellationToken> {
        self.agents.get(id).map(|e| e.cancel.clone())
    }

    /// Attempt a legal-only lifecycle transition.
    pub fn transition(
        &mut self,
        id: &str,
        next: SubAgentState,
    ) -> Result<(), TransitionError> {
        let entry = self
            .agents
            .get_mut(id)
            .ok_or_else(|| TransitionError::Unknown(id.to_string()))?;
        if entry.state.can_transition_to(&next) {
            entry.state = next;
            Ok(())
        } else {
            Err(TransitionError::Illegal {
                from: entry.state,
                to: next,
            })
        }
    }

    /// Tear an agent down: cancel its in-flight work, drop its handle, free the
    /// map slot. Returns `false` if no such agent was tracked (idempotent).
    pub fn destroy(&mut self, id: &str) -> bool {
        match self.agents.remove(id) {
            Some(entry) => {
                entry.cancel.cancel();
                true
            }
            None => false,
        }
    }

    /// Remove a completed/failed agent (delegates to `destroy` for teardown).
    pub fn remove(&mut self, id: &str) -> bool {
        self.destroy(id)
    }

    /// List all active agents.
    pub fn list(&self) -> Vec<&SubAgentHandle> {
        self.agents.values().map(|e| &e.handle).collect()
    }

    /// Get a specific agent's handle.
    pub fn get(&self, id: &str) -> Option<&SubAgentHandle> {
        self.agents.get(id).map(|e| &e.handle)
    }
}
```

> The hand-written `impl Default for SubAgentSpawner` at the old `:65-69` is replaced by `#[derive(Default)]` on the struct (both `HashMap` and `usize` are `Default`).

---

## 4. TDD Test Code

### 4.1 M-D Test: Disabled Coordinator is a No-Op

**File:** `crates/runtime/tests/evolution_integration.rs`
**Insertion point:** After the `sliding_window_eviction` test ending at line 333, before end of file.

```rust
// ---------------------------------------------------------------------------
// Test: Disabled coordinator never touches the pipeline
// ---------------------------------------------------------------------------

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
        .post_turn(
            "task", "error output", false, 5, 2, 1000, 1,
            &pipeline,
            vec![],
        )
        .await
        .unwrap();

    assert!(!summary.evolution_triggered, "disabled loop must not trigger");
    assert_eq!(
        gen_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no candidate generated"
    );
    assert_eq!(
        mig_calls.load(std::sync::atomic::Ordering::SeqCst),
        0,
        "no migration"
    );
    assert!(!summary.reflected, "disabled loop skips reflection too");
}
```

**Run command:**
```bash
cargo test -p runtime --test evolution_integration disabled_coordinator_is_a_noop
```

> Note: The three existing tests (`failure_triggers_evolution` at line 164, `periodic_trigger_at_n_turns` at line 220, `sliding_window_eviction` at line 277) construct `EvolutionConfig` positionally. Each must add `enabled: true` as the first field so the enabled paths remain covered:
>
> ```rust
> // failure_triggers_evolution (line 167):
> let config = EvolutionConfig {
>     enabled: true,  // <-- ADD
>     trigger_every_n_turns: 0,
>     ...
> };
>
> // periodic_trigger_at_n_turns (line 223):
> let config = EvolutionConfig {
>     enabled: true,  // <-- ADD
>     trigger_every_n_turns: 3,
>     ...
> };
>
> // sliding_window_eviction (line 280):
> let config = EvolutionConfig {
>     enabled: true,  // <-- ADD
>     trigger_every_n_turns: 0,
>     ...
> };
> ```

### 4.2 M-D Test: Rejected Candidate is Rolled Back

**File:** `crates/metacog/src/impl/morphogenesis/pipeline.rs`
**Insertion point:** At end of file.

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
            identity: IdentitySpec {
                name: "t".into(),
                description: "t".into(),
                self_model: "t".into(),
            },
            boundary: BoundarySpec { rules: vec![] },
            care: CareSpec { priorities: vec![] },
            memory: MemorySpec {
                backends: vec![],
                compaction_strategy: "none".into(),
            },
            mutation: MutationSpec {
                allowed_targets: vec!["care.priorities".into()],
                require_sandbox: false,
                require_self_field_approval: false,
            },
            lifecycle: LifecycleSpec {
                auto_compact: false,
                health_check_interval_secs: 60,
                max_idle_time_secs: 3600,
            },
        }
    }

    struct RejectingMeta {
        rollbacks: Arc<AtomicUsize>,
    }

    #[async_trait]
    impl Subsystem for RejectingMeta {
        fn name(&self) -> &str {
            "reject-meta"
        }
        fn version(&self) -> Version {
            Version::new(0, 1, 0)
        }
        async fn init(&mut self, _c: &base::SubsystemContext) -> anyhow::Result<()> {
            Ok(())
        }
        async fn shutdown(&mut self) -> anyhow::Result<()> {
            Ok(())
        }
        async fn health(&self) -> SubsystemHealth {
            SubsystemHealth::Healthy
        }
    }

    #[async_trait]
    impl MetaRuntimeOps for RejectingMeta {
        async fn read_genome(&self) -> anyhow::Result<Genome> {
            Ok(genome())
        }
        async fn generate_candidate(
            &self,
            _i: &MutationIntent,
        ) -> anyhow::Result<RuntimeCandidate> {
            Ok(RuntimeCandidate {
                id: uuid::Uuid::new_v4(),
                genome: genome(),
                changes: vec!["c".into()],
                generated_at: chrono::Utc::now(),
            })
        }
        async fn sandbox_test(&self, _c: &RuntimeCandidate) -> anyhow::Result<TestResult> {
            Ok(TestResult {
                passed: false,
                tests_run: 1,
                tests_passed: 0,
                tests_failed: 1,
                failures: vec!["boom".into()],
                elapsed_ms: 1,
            })
        }
        async fn evaluate(
            &self,
            _c: &RuntimeCandidate,
            _t: &TestResult,
        ) -> anyhow::Result<Evaluation> {
            Ok(Evaluation {
                score: 0.0,
                strengths: vec![],
                weaknesses: vec!["failed".into()],
                recommendation: Recommendation::Reject,
            })
        }
        async fn migrate(&self, _c: &RuntimeCandidate) -> anyhow::Result<MigrationResult> {
            panic!("migrate must not be called on a rejected candidate")
        }
        async fn rollback(&self) -> anyhow::Result<()> {
            self.rollbacks.fetch_add(1, Ordering::SeqCst);
            Ok(())
        }
        fn current_version(&self) -> Version {
            Version::new(0, 1, 0)
        }
    }

    #[tokio::test]
    async fn rejected_candidate_is_rolled_back() {
        let rollbacks = Arc::new(AtomicUsize::new(0));
        let meta = RejectingMeta {
            rollbacks: rollbacks.clone(),
        };
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
        assert_eq!(
            rollbacks.load(Ordering::SeqCst),
            1,
            "rollback() must fire exactly once"
        );
    }

    #[tokio::test]
    async fn sandbox_error_rolls_back() {
        use std::sync::atomic::AtomicBool;

        struct SandboxFailingMeta {
            rolled_back: Arc<AtomicBool>,
        }

        #[async_trait]
        impl Subsystem for SandboxFailingMeta {
            fn name(&self) -> &str {
                "sandbox-fail"
            }
            fn version(&self) -> Version {
                Version::new(0, 1, 0)
            }
            async fn init(&mut self, _c: &base::SubsystemContext) -> anyhow::Result<()> {
                Ok(())
            }
            async fn shutdown(&mut self) -> anyhow::Result<()> {
                Ok(())
            }
            async fn health(&self) -> SubsystemHealth {
                SubsystemHealth::Healthy
            }
        }

        #[async_trait]
        impl MetaRuntimeOps for SandboxFailingMeta {
            async fn read_genome(&self) -> anyhow::Result<Genome> {
                Ok(genome())
            }
            async fn generate_candidate(
                &self,
                _i: &MutationIntent,
            ) -> anyhow::Result<RuntimeCandidate> {
                Ok(RuntimeCandidate {
                    id: uuid::Uuid::new_v4(),
                    genome: genome(),
                    changes: vec!["c".into()],
                    generated_at: chrono::Utc::now(),
                })
            }
            async fn sandbox_test(&self, _c: &RuntimeCandidate) -> anyhow::Result<TestResult> {
                anyhow::bail!("sandbox crashed")
            }
            async fn evaluate(
                &self,
                _c: &RuntimeCandidate,
                _t: &TestResult,
            ) -> anyhow::Result<Evaluation> {
                unimplemented!()
            }
            async fn migrate(&self, _c: &RuntimeCandidate) -> anyhow::Result<MigrationResult> {
                unimplemented!()
            }
            async fn rollback(&self) -> anyhow::Result<()> {
                self.rolled_back.store(true, Ordering::SeqCst);
                Ok(())
            }
            fn current_version(&self) -> Version {
                Version::new(0, 1, 0)
            }
        }

        let rolled_back = Arc::new(AtomicBool::new(false));
        let meta = SandboxFailingMeta {
            rolled_back: rolled_back.clone(),
        };
        let pipeline = MorphogenesisPipeline::new(meta);
        let intent = MutationIntent {
            target: "care.priorities".into(),
            change: serde_json::json!({}),
            reason: "test".into(),
            reversible: true,
        };
        let result = pipeline.run(&intent).await;
        assert!(result.is_err(), "sandbox crash must error");
        assert!(
            rolled_back.load(Ordering::SeqCst),
            "sandbox crash must trigger rollback"
        );
    }
}
```

**Run commands:**
```bash
cargo test -p metacog morphogenesis::pipeline::tests::rejected_candidate_is_rolled_back
cargo test -p metacog morphogenesis::pipeline::tests::sandbox_error_rolls_back
```

### 4.3 M-D Test: `run_action` Gates on `TriggerEvolution`

**File:** `crates/runtime/tests/evolution_integration.rs`
**Insertion point:** After `disabled_coordinator_is_a_noop` test.

```rust
// ---------------------------------------------------------------------------
// Test: run_action gates on EvolutionAction variant
// ---------------------------------------------------------------------------

#[tokio::test]
async fn run_action_gates_on_trigger_evolution() {
    let tmp = tempfile::tempdir().unwrap();
    let coordinator = EvolutionCoordinator::new(EvolutionConfig {
        enabled: true,
        trigger_every_n_turns: 0,
        trigger_on_failure: false,
        window_size: 20,
        lineage_dir: tmp.path().to_path_buf(),
    })
    .unwrap();
    let (mock, gen_calls, _mig) = MockMetaRuntime::new();
    let pipeline = MorphogenesisPipeline::new(mock);

    // Observe -> nothing runs.
    let n = coordinator
        .run_action(&metacog::EvolutionAction::Observe, &pipeline)
        .await
        .unwrap();
    assert_eq!(n, 0);
    assert_eq!(
        gen_calls.load(std::sync::atomic::Ordering::SeqCst),
        0
    );

    // TriggerEvolution -> the pipeline runs once per intent.
    let action = metacog::EvolutionAction::TriggerEvolution {
        intents: vec![base::MutationIntent {
            target: "care.priorities".into(),
            change: serde_json::json!({ "topic": "safety", "delta": 0.1 }),
            reason: "meta-cognition trigger".into(),
            reversible: true,
        }],
    };
    let ran = coordinator.run_action(&action, &pipeline).await.unwrap();
    assert_eq!(ran, 1, "one intent => one pipeline run");
    assert_eq!(
        gen_calls.load(std::sync::atomic::Ordering::SeqCst),
        1
    );
}

#[tokio::test]
async fn run_action_disabled_coordinator_does_nothing() {
    let tmp = tempfile::tempdir().unwrap();
    let coordinator = EvolutionCoordinator::new(EvolutionConfig {
        enabled: false, // DISABLED
        trigger_every_n_turns: 0,
        trigger_on_failure: false,
        window_size: 20,
        lineage_dir: tmp.path().to_path_buf(),
    })
    .unwrap();
    let (mock, gen_calls, _mig) = MockMetaRuntime::new();
    let pipeline = MorphogenesisPipeline::new(mock);

    let action = metacog::EvolutionAction::TriggerEvolution {
        intents: vec![base::MutationIntent {
            target: "care.priorities".into(),
            change: serde_json::json!({ "topic": "safety", "delta": 0.1 }),
            reason: "should not run".into(),
            reversible: true,
        }],
    };
    let ran = coordinator.run_action(&action, &pipeline).await.unwrap();
    assert_eq!(ran, 0, "disabled coordinator must not run any action");
    assert_eq!(
        gen_calls.load(std::sync::atomic::Ordering::SeqCst),
        0
    );
}
```

**Run command:**
```bash
cargo test -p runtime --test evolution_integration run_action
```

### 4.4 M-E Tests: Spawner Lifecycle

**File:** `crates/runtime/src/core/sub_agent.rs`
**Insertion point:** At end of file.

```rust
#[cfg(test)]
mod lifecycle_tests {
    use super::*;
    use base::SubAgentState;

    #[test]
    fn spawn_starts_in_created_and_legal_transitions_advance() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        assert_eq!(s.state(&h.id), Some(SubAgentState::Created));
        assert!(s.transition(&h.id, SubAgentState::Running).is_ok());
        assert!(s.transition(&h.id, SubAgentState::Waiting).is_ok());
        assert_eq!(s.state(&h.id), Some(SubAgentState::Waiting));
    }

    #[test]
    fn illegal_transition_is_rejected_and_state_unchanged() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        // Created -> Completed is illegal (must Run first).
        assert!(s.transition(&h.id, SubAgentState::Completed).is_err());
        assert_eq!(s.state(&h.id), Some(SubAgentState::Created));
    }

    #[tokio::test]
    async fn destroy_cancels_in_flight_work_and_frees_the_slot() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        let token = s
            .cancel_token(&h.id)
            .expect("token exists while agent is live");

        // Simulate in-flight work awaiting cancellation.
        let worker = tokio::spawn(async move {
            token.cancelled().await;
            "cancelled"
        });

        assert!(s.destroy(&h.id), "destroy returns true for a live agent");
        assert_eq!(
            worker.await.unwrap(),
            "cancelled",
            "destroy must cancel the token"
        );
        assert!(s.get(&h.id).is_none(), "map slot is freed after destroy");
        assert_eq!(s.state(&h.id), None);
        assert!(!s.destroy(&h.id), "second destroy is a no-op");
    }

    #[test]
    fn remove_delegates_to_destroy() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        let token = s.cancel_token(&h.id).unwrap();
        assert!(!token.is_cancelled());

        assert!(s.remove(&h.id));
        assert!(token.is_cancelled(), "remove must cancel the token");
        assert!(s.get(&h.id).is_none());
    }

    #[test]
    fn list_and_get_preserved_after_internal_type_change() {
        let mut s = SubAgentSpawner::new();
        let h1 = s.spawn("task1".into(), "t1".into());
        let h2 = s.spawn("task2".into(), "t2".into());

        let list = s.list();
        assert_eq!(list.len(), 2);
        assert_eq!(list[0].id, h1.id);
        assert_eq!(list[1].id, h2.id);

        let got = s.get(&h1.id).unwrap();
        assert_eq!(got.task, "task1");
    }

    #[test]
    fn update_status_still_works() {
        let mut s = SubAgentSpawner::new();
        let h = s.spawn("task".into(), "turn-1".into());
        s.update_status(
            &h.id,
            SubAgentStatus::Executing {
                current_step: "step-1".into(),
            },
        );
        let got = s.get(&h.id).unwrap();
        assert!(matches!(
            got.status,
            SubAgentStatus::Executing { .. }
        ));
    }

    #[test]
    fn transition_error_display() {
        let err = TransitionError::Unknown("x".into());
        assert!(err.to_string().contains("x"));

        let err = TransitionError::Illegal {
            from: SubAgentState::Created,
            to: SubAgentState::Completed,
        };
        assert!(err.to_string().contains("Created"));
        assert!(err.to_string().contains("Completed"));
    }
}
```

**Run commands:**
```bash
cargo test -p runtime lifecycle_tests
cargo test -p runtime  # full crate
```

---

## 5. Exact File Paths and Insertion Points

| # | File (absolute path) | Change | Line(s) |
|---|---|---|---|
| M-D.1 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/src/core/evolution_coordinator.rs` | Add `enabled: bool` to `EvolutionConfig` struct | 26-36 (modify struct), 38-47 (modify Default) |
| M-D.2 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/src/core/evolution_coordinator.rs` | Early-return gate in `post_turn` | After 155, before 157 |
| M-D.3 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/src/core/config/agent.rs` | Add `EvolutionSettings` struct | After 160, before 162 |
| M-D.4 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/src/core/config/mod.rs` | Re-export `EvolutionSettings` | Line 8 |
| M-D.5 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/src/core/config/mod.rs` | Add `evolution` field to `AppConfig` | After `perception` field (line 48-49) + Default impl |
| M-D.6 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/src/impl/daemon/handler/mod.rs` | Map `config.evolution.enabled` into `EvolutionConfig` | 337-344 |
| M-D.7 | `/home/rj001/Bear-ws/work/aletheon/crates/metacog/src/impl/morphogenesis/pipeline.rs` | Add `rolled_back` to `PipelineResult` | 103-110 |
| M-D.8 | `/home/rj001/Bear-ws/work/aletheon/crates/metacog/src/impl/morphogenesis/pipeline.rs` | Wrap sandbox/eval with rollback-on-error | 38-53 |
| M-D.9 | `/home/rj001/Bear-ws/work/aletheon/crates/metacog/src/impl/morphogenesis/pipeline.rs` | Rollback on non-adopt recommendation | 55-78 |
| M-D.10 | `/home/rj001/Bear-ws/work/aletheon/crates/metacog/src/impl/morphogenesis/pipeline.rs` | Update `PipelineResult` construction | 80-99 |
| M-D.11 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/src/core/evolution_coordinator.rs` | Add `run_action` method | After 335, before 341 |
| M-D.12 | `/home/rj001/Bear-ws/work/aletheon/crates/metacog/src/lib.rs` | Re-export `EvolutionAction` | After line 5 |
| M-D.T1 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/tests/evolution_integration.rs` | Test: disabled no-op | After 333 |
| M-D.T2 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/tests/evolution_integration.rs` | Fix existing tests: add `enabled: true` | Lines 167, 223, 280 |
| M-D.T3 | `/home/rj001/Bear-ws/work/aletheon/crates/metacog/src/impl/morphogenesis/pipeline.rs` | Test: rejected rolled back + sandbox error | End of file |
| M-D.T4 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/tests/evolution_integration.rs` | Test: run_action gating | After disabled test |
| M-E.1 | `/home/rj001/Bear-ws/work/aletheon/crates/base/src/events/ui_event.rs` | Add `SubAgentState` enum + `can_transition_to` | After 121, before 123 |
| M-E.2 | `/home/rj001/Bear-ws/work/aletheon/crates/base/src/events/ui_event.rs` | Add `subagent_state_tests` module | End of file |
| M-E.3 | `/home/rj001/Bear-ws/work/aletheon/crates/base/src/lib.rs` | Re-export `SubAgentState` | Lines 127-130 |
| M-E.4 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/src/core/sub_agent.rs` | Complete rewrite with state machine | Entire file (1-70) |
| M-E.5 | `/home/rj001/Bear-ws/work/aletheon/crates/runtime/src/core/sub_agent.rs` | Add `lifecycle_tests` module | End of file |

---

## 6. Integration Test Strategy

### 6.1 M-D: Evolution Gated by Config Flag

**Scenario 1: Default config produces no evolution.**
```bash
# Start daemon with default config (no [evolution] section)
# Send a chat turn that would normally trigger evolution (e.g., a failing tool call)
# Verify: no lineage.jsonl entries created, no genome changes
```

**Scenario 2: Enabled config triggers evolution on failure.**
```bash
# config.toml: [evolution] enabled = true
# Send a failing turn
# Verify: lineage.jsonl has a new entry, genome care weights updated
```

**Scenario 3: Rollback on failed candidate.**
```bash
# config.toml: [evolution] enabled = true
# Sandbox returns failure
# Verify: rollback manager snapshot count unchanged, genome unchanged
```

### 6.2 M-E: SubAgent Destroy Mid-Execution

**Scenario 1: Normal lifecycle.**
```bash
# Spawn subagent -> status = Created
# transition(Running) -> status = Running
# transition(Completed) -> status = Completed
# destroy() -> slot freed
```

**Scenario 2: Destroy mid-execution cancels work.**
```bash
# Spawn subagent -> get cancel_token
# Spawn async task that awaits token.cancelled()
# destroy() -> task unblocks, slot freed
```

**Scenario 3: Illegal transition rejected.**
```bash
# Spawn subagent -> Created
# transition(Completed) -> Err(Illegal)
# state still Created
```

### 6.3 Cross-Module: Evolution Consumer Reads SubAgent Traces

**Future scenario (not implemented in this plan):**
```bash
# Subagent completes work -> trace stored
# Evolution loop runs -> MutationIntentGenerator::from_reflections reads subagent traces
# Pipeline generates candidate based on subagent performance
```

---

## 7. Rollback Plan

### 7.1 M-D Rollback (Evolution Changes)

If the self-evolution loop causes issues in production:

1. **Immediate:** Set `[evolution] enabled = false` in config and restart daemon. All evolution stops. No genome changes.
2. **Code revert:** Revert the `EvolutionConfig.enabled` field addition and the `post_turn` gate. The old behavior (unconditional evolution) returns.
3. **Data cleanup:** Delete `lineage_dir` (`/var/lib/aletheon/lineage/` by default) to remove accumulated lineage entries.
4. **Genome reset:** If care weights were mutated, reset `GenomeConfig.care_weights` to default values.

### 7.2 M-E Rollback (SubAgent Lifecycle)

If the state machine or destroy() behavior causes issues:

1. **Code revert:** Revert `sub_agent.rs` to its original (70-line) version. The `SubAgentState` enum in `base` is additive and harmless -- leave it.
2. **Behavior change:** The new `remove()` calls `destroy()` which cancels a token. If callers relied on `remove()` not having side effects, they would have already lost the handle (map slot freed) -- the cancellation is only meaningful if someone held a `cancel_token` clone, which no existing code does.
3. **Data cleanup:** None needed -- subagents are in-memory only.

---

## 8. Risk Assessment

### 8.1 M-D Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Evolution running in production | **HIGH** | Default-OFF config gate is the single choke point. Operator must explicitly set `[evolution] enabled = true`. `TODO(Tier 2a)` markers flag where PermissionManager must be added. |
| Rollback snapshot missing | Medium | `DefaultMetaRuntime::generate_candidate` always snapshots before returning (`traits.rs:155-159`). Pipeline logs a warning if rollback fails (best-effort). |
| Rollback fails silently | Low | Pipeline sets `rolled_back = false` and logs a warning. Does not mask the original error. |
| `decide()` still dead after Phase 3 | Medium | `run_action` is the consumption side. Wiring `DaseinContext` into a post-turn `decide()` call is a follow-up; this plan documents it explicitly. |
| Existing tests break | Low | Three tests need `enabled: true` added. Section 4 documents the exact fix. |
| Config parse regression | Low | `#[serde(default)]` on `EvolutionSettings` ensures existing configs without `[evolution]` section parse correctly and default to `enabled: false`. |
| `apply_care_mutation` modifies runtime state | Medium | Only modifies the in-memory `GenomeConfig.care_weights`. Does not persist. Tracked as "care weights are ephemeral" -- Tier 1 (Governed Memory) may persist them later. |

### 8.2 M-E Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Subagent leaks (token not cancelled) | **Low** | `destroy()` calls `cancel.cancel()` before dropping the map slot. Idempotent (second destroy is no-op). |
| Map value change breaks callers | **Low** | `list()` and `get()` still return `&SubAgentHandle` (projected from `entry.handle`). `rpc.rs:729-743` accesses `.id/.task/.status` on the handle -- unchanged. |
| `remove()` semantics change | **Low** | Old `remove()` was bare `HashMap::remove`. New `remove()` delegates to `destroy()` which also calls `cancel.cancel()`. Since no code holds a token clone today, the cancellation is a no-op. Stronger guarantees, no regression. |
| `#[derive(Default)]` vs hand-written `impl Default` | **None** | Both produce identical behavior (`HashMap::new()` + `next_id: 0`). |
| `Destroyed` state never persisted in map | **Low** | By design -- `destroy()` removes the entry. `state(id)` returns `None` after destroy. If audit trail is needed later, keep the entry with `state = Destroyed` -- a follow-up. |
| No live task wired to token | **None** | `cancel_token(id)` provides the token for future task-spawning code. Until then, cancellation is a correct no-op. Documented as out of M-E scope. |

### 8.3 Cross-Module Risks

| Risk | Severity | Mitigation |
|---|---|---|
| Evolution+subagent deadlock | None | Today they are independent. No shared locks between `EvolutionCoordinator` and `SubAgentSpawner`. |
| PermissionManager absent (blocks M-D hardening) | Medium | Documented with `TODO(Tier 2a)` markers at every gate site. Config flag alone protects until Tier 2a lands. |

---

## 9. Implementation Sequence

The modules are independent and can be implemented in parallel on separate branches:

```
Branch: auro/feat/20260702-aletheon-self-evolution  (M-D)
Branch: auro/feat/20260702-aletheon-subagent-lifecycle  (M-E)
```

**M-D sequence (3 phases, sequential):**
1. Phase 1: Default-OFF config gate (Tasks 1+2 from original plan)
2. Phase 2: Guaranteed rollback (Task 3)
3. Phase 3: Wire `EvolutionAction` trigger (Task 4)

**M-E sequence (2 phases, sequential):**
1. Phase 1: `SubAgentState` in `base` (Task 1)
2. Phase 2: State machine + `destroy()` in spawner (Task 2)

**Validation per phase:**
- M-D: `cargo test -p runtime --test evolution_integration` + `cargo test -p metacog`
- M-E: `cargo test -p base` then `cargo test -p runtime`
- Final: `cargo build --workspace` and `cargo test --workspace`

---

## 10. Non-Goals (Explicitly Excluded)

- No code/topology self-mutation (migration stays over declarative `Genome`)
- No `DaseinContext` wiring into `decide()` (follow-up)
- No task-spawning wired to `CancellationToken` (follow-up)
- No `PermissionManager` (follow-up, Tier 2a)
- No new RPC surface for subagent lifecycle
- No changes to `SubAgentStatus` wire shape
- No scheduling policy for subagents
