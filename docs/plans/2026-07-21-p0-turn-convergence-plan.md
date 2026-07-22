# P0 Turn-Convergence Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Converge the daemon turn entry onto the existing `TurnEngine` contract (strangler) and make `harness_kind` config actually select the cognitive session factory — without rewriting the 64KB `TurnPipeline`.

**Architecture:** The `TurnEngine` trait and `SessionTurnEngine` (exec path) already exist (`crates/executive/src/service/turn_engine.rs`). The daemon path currently inlines `TurnPipeline::run` inside a `TurnCoordinator::submit_with` closure (`crates/executive/src/service/daemon_turn/execute.rs:154-291`) and never touches `TurnEngine`. We add a `DaemonTurnEngine` that implements `TurnEngine` over the pipeline, prove parity against the existing stub-based parity test, then switch the daemon closure to call it. Separately we thread the existing `harness_kind` from config into selection and prove unsupported values fail closed. P3 adds `HarnessKind::Robot` only together with a real RobotHarness; P0 never aliases Robot to Linear.

**Tech Stack:** Rust (workspace crates `executive`, `cognit`), `async_trait`, `tokio`, `serde`.

**Spec:** `docs/plans/2026-07-21-embodied-cognition-framework-design.md` §4.

**Non-goals (guard against scope creep):** No splitting of `TurnPipeline` fields. No real `RobotHarness` logic (P3). No EventBus semantic changes (§8 problem 10). No new robot-specific daemon entry.

---

## Baseline anchors (re-verify before starting)

```bash
# harness_kind is referenced only in config, never consumed at runtime
rg -n "harness_kind" crates/
# daemon inlines pipeline.run; exec uses SessionTurnEngine
rg -n "DaemonTurnEngine|SessionTurnEngine|pipeline\.run" crates/executive/src
# existing parity test + snapshot type
rg -n "TurnEngineParitySnapshot|StubTurnEngine" crates/executive
```

Expected at baseline: `harness_kind` appears only in `crates/executive/src/core/config/agent.rs:44-46,64` and doc comments in `crates/cognit/src/harness/mod.rs:37-38`; no `DaemonTurnEngine` exists; `TurnEngineParitySnapshot` exists in `turn_engine.rs`.

---

## Task 1: Lock the P0 harness boundary fail closed

**Files:**
- Modify: `crates/cognit/src/harness/mod.rs` (`JsonSchema` derive for typed app config)
- Test: `crates/cognit/src/harness/mod.rs` (inline `#[cfg(test)]`)

P0 must not publish a `robot` configuration that silently runs the Linear harness. The current enum has only `Linear`; preserve that boundary until P3 can add the variant and real implementation atomically.

- [ ] **Step 1: Add the boundary test**

```rust
#[cfg(test)]
mod harness_kind_tests {
    use super::HarnessKind;

    #[test]
    fn robot_is_rejected_until_robot_harness_exists() {
        let parsed = serde_json::from_str::<HarnessKind>(r#""robot""#);
        assert!(parsed.is_err());
    }

    #[test]
    fn linear_remains_default() {
        assert_eq!(HarnessKind::default(), HarnessKind::Linear);
    }
}
```

- [ ] **Step 2: Run the narrow test**

Run: `bash scripts/cargo-agent.sh test -p cognit harness_kind_tests -- --nocapture`
Expected: PASS with 2 tests; no production-code change is required.

- [ ] **Step 3: Make the existing enum usable in the root config schema**

Add `schemars::JsonSchema` to the existing `HarnessKind` derive list. Do not add a
`Robot` variant:

```rust
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, Default,
    Serialize, Deserialize, schemars::JsonSchema,
)]
#[serde(rename_all = "lowercase")]
pub enum HarnessKind {
    #[default]
    Linear,
}
```

- [ ] **Step 4: Record the stage intentionally**

Stage only `crates/cognit/src/harness/mod.rs`, inspect `git diff --cached`, and commit with the repository-required conventional subject, problem/solution paragraphs, and a file bullet. Do not use a subject-only commit.

---

## Task 2: Thread `harness_kind` through the executive `DaemonConfig`

**Files:**
- Modify: `crates/executive/src/impl/daemon/mod.rs:36` (`DaemonConfig` struct)
- Modify: `crates/executive/src/core/runtime_core.rs:74` (construction site)
- Modify: `crates/executive/src/user_runtime/mod.rs:75` (construction site)
- Test: `crates/executive/src/impl/daemon/mod.rs` (inline `#[cfg(test)]`)

Note: the executive `DaemonConfig` (`mod.rs:36`) is built by hand from the app config; it is NOT deserialized, so no serde attr is needed. Source it the same way `agent_max_iterations` flows from `app.agent.max_iterations`.

- [ ] **Step 1: Add the field and observe the construction failures**

Edit `crates/executive/src/impl/daemon/mod.rs:36` — add after `pub agent_max_iterations: usize,`:

```rust
    /// Selects the cognitive harness the factory builds. Mirrors the flow of
    /// `agent_max_iterations` from `app.agent`. Defaults to Linear.
    pub harness_kind: cognit::harness::HarnessKind,
```

Run: `bash scripts/cargo-agent.sh check -p executive`
Expected: FAIL at every `DaemonConfig` literal with a missing `harness_kind` field. This compile failure is the complete caller inventory; do not add a default inside `DaemonConfig` to hide a caller.

- [ ] **Step 2: Populate both construction sites**

At `crates/executive/src/core/runtime_core.rs:74` (the `DaemonConfig { ... }` literal), add — mirroring `agent_max_iterations: app_config.agent.max_iterations`:

```rust
    harness_kind: app_config.agent.harness_kind,
```

At `crates/executive/src/user_runtime/mod.rs:75` (the `DaemonConfig { ... }` literal), add — mirroring `agent_max_iterations: app.agent.max_iterations`:

```rust
    harness_kind: app.agent.harness_kind,
```

- [ ] **Step 3: Add the single application-config source field**

`AppConfig.agent` is `cognit::config::AgentConfig`; add the typed field beside
`max_iterations` in `crates/cognit/src/config/mod.rs`:

```rust
    #[serde(default)]
    pub harness_kind: crate::harness::HarnessKind,
```

Populate it as `HarnessKind::default()` in `AgentConfig::default()`. This is the
single persisted application-config owner. The existing
`ExecutiveConfig.harness_kind` (`crates/executive/src/core/config/agent.rs:44-64`)
is a runtime snapshot populated later by bootstrap, not a second file/env source.

- [ ] **Step 4: Run the narrow check**

Run: `bash scripts/cargo-agent.sh check -p cognit && bash scripts/cargo-agent.sh check -p executive`
Expected: PASS (all `DaemonConfig` literals now include the field).

- [ ] **Step 7: Commit**

```bash
git add crates/executive/src/impl/daemon/mod.rs crates/executive/src/core/runtime_core.rs crates/executive/src/user_runtime/mod.rs crates/cognit/src/config
# Suggested subject: feat(executive): thread harness_kind from AgentConfig into DaemonConfig
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 3: Make the factory select on `harness_kind`

**Files:**
- Modify: `crates/executive/src/impl/daemon/bootstrap/request.rs:482-503`
- Modify: `crates/executive/src/service/harness_factory.rs` (add selector fn)
- Test: `crates/executive/src/service/harness_factory.rs` (inline `#[cfg(test)]`)

- [ ] **Step 1: Write the failing test**

Add to `crates/executive/src/service/harness_factory.rs`:

```rust
#[cfg(test)]
mod selection_tests {
    use super::*;
    use cognit::harness::HarnessKind;

    #[test]
    fn selected_kind_is_reported() {
        // The bootstrap must record which kind it selected so operators can verify.
        assert_eq!(selected_harness_kind(HarnessKind::Linear), "linear");
            }
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/cargo-agent.sh test -p executive selection_tests`
Expected: FAIL — `selected_harness_kind` not found.

- [ ] **Step 3: Add the selector helper**

Append to `crates/executive/src/service/harness_factory.rs`:

```rust
/// Human-readable label for the selected harness kind. Used for the bootstrap
/// log line that proves config selection is honored at runtime.
pub fn selected_harness_kind(kind: cognit::harness::HarnessKind) -> &'static str {
    match kind {
        cognit::harness::HarnessKind::Linear => "linear",
    }
}
```

- [ ] **Step 4: Consume `harness_kind` in bootstrap**

Edit `crates/executive/src/impl/daemon/bootstrap/request.rs:482-490` — replace the `..Default::default()` construction so `harness_kind` comes from `config`:

```rust
    let runtime_config = ExecutiveConfig {
        session_id: session_id.clone(),
        context_window_tokens: context_window,
        conscious_arbitration_mode: config.conscious_arbitration_mode,
        compaction_v2: grok_hardening.compaction_v2,
        streaming_tools: grok_hardening.streaming_tools,
        max_iterations: config.agent_max_iterations,
        harness_kind: config.harness_kind,
        ..Default::default()
    };
```

Then, immediately after `let runtime_config_snapshot = runtime_config.clone();` (`request.rs:492`), add the proof-of-wiring log line:

```rust
    tracing::info!(
        harness = crate::service::harness_factory::selected_harness_kind(
            runtime_config_snapshot.harness_kind
        ),
        "cognitive harness selected from config"
    );
```

The `HarnessConfig` passed to `LinearCognitiveSessionFactory` continues to come from `harness_config_from_executive`; the only accepted P0 value is `Linear`. A `robot` value remains a typed configuration error until P3 provides the real factory.

- [ ] **Step 5: Run test + build**

Run: `bash scripts/cargo-agent.sh test -p executive selection_tests && bash scripts/cargo-agent.sh build -p executive`
Expected: PASS + clean build.

- [ ] **Step 6: Commit**

```bash
git add crates/executive/src/service/harness_factory.rs crates/executive/src/impl/daemon/bootstrap/request.rs
# Suggested subject: feat(executive): consume harness_kind from config at daemon bootstrap
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 4: Add `DaemonTurnEngine` implementing `TurnEngine`

**Files:**
- Create: `crates/executive/src/service/daemon_turn_engine.rs`
- Modify: `crates/executive/src/service/mod.rs` (declare module)
- Test: `crates/executive/tests/daemon_turn_engine.rs`

The engine wraps `Arc<TurnPipeline>` and maps `TurnEngineRequest`/`TurnEngineContext` → `TurnPipeline::run(...)` → `TurnEngineResult`, mirroring the JSON parsing already in `execute.rs:229-241` and the request shaping in `SessionTurnEngine::execute`.

- [ ] **Step 1: Write the failing test**

Create `crates/executive/tests/daemon_turn_engine.rs`:

```rust
use std::sync::Arc;

use executive::service::daemon_turn_engine::map_pipeline_response;
use executive::service::turn_engine::TurnEngineStatus;

#[test]
fn maps_successful_pipeline_response_to_completed() {
    let turn_id = fabric::TurnId::new();
    let response = serde_json::json!({
        "result": {
            "response": "hi",
            "succeeded": true,
            "metrics": { "tool_calls_made": 3, "elapsed_ms": 42 }
        }
    });
    let result = map_pipeline_response(turn_id, &response);
    assert_eq!(result.status, TurnEngineStatus::Completed);
    assert_eq!(result.output, "hi");
    assert_eq!(result.tool_calls, 3);
    assert_eq!(result.elapsed_ms, 42);
    assert_eq!(result.turn_id, turn_id);
}

#[test]
fn maps_error_pipeline_response_to_blocked() {
    let turn_id = fabric::TurnId::new();
    let response = serde_json::json!({
        "error": { "code": -32603, "message": "boom" }
    });
    let result = map_pipeline_response(turn_id, &response);
    assert_eq!(result.status, TurnEngineStatus::Blocked);
    assert_eq!(result.output, "");
}
```

- [ ] **Step 2: Run test to verify it fails**

Run: `bash scripts/cargo-agent.sh test -p executive --test daemon_turn_engine`
Expected: FAIL — module/function not found.

- [ ] **Step 3: Create the engine + pure mapping fn**

Create `crates/executive/src/service/daemon_turn_engine.rs`:

```rust
//! DaemonTurnEngine — adapts the daemon `TurnPipeline` to the unified
//! `TurnEngine` contract (strangler step W1). Behavior-preserving: it runs the
//! same `TurnPipeline::run` the coordinator closure runs today, then maps the
//! JSON-RPC-shaped response into a `TurnEngineResult`.

use std::sync::Arc;

use async_trait::async_trait;
use tokio_util::sync::CancellationToken;

use crate::service::turn_engine::{
    TurnEngine, TurnEngineContext, TurnEngineError, TurnEngineEventSink, TurnEngineRequest,
    TurnEngineResult, TurnEngineStatus,
};
use crate::service::TurnPipeline;

/// Pure mapping from the pipeline's JSON response to `TurnEngineResult`.
/// Extracted so it is unit-testable without constructing a pipeline.
pub fn map_pipeline_response(
    turn_id: fabric::TurnId,
    response: &serde_json::Value,
) -> TurnEngineResult {
    if response.get("error").is_some() {
        return TurnEngineResult {
            turn_id,
            output: String::new(),
            status: TurnEngineStatus::Blocked,
            tool_calls: 0,
            tokens_in: 0,
            tokens_out: 0,
            elapsed_ms: 0,
        };
    }
    let result = &response["result"];
    let succeeded = result["succeeded"].as_bool().unwrap_or(false);
    let metric = &result["metrics"];
    TurnEngineResult {
        turn_id,
        output: result["response"].as_str().unwrap_or_default().to_string(),
        status: if succeeded {
            TurnEngineStatus::Completed
        } else {
            TurnEngineStatus::Blocked
        },
        tool_calls: metric["tool_calls_made"].as_u64().unwrap_or(0) as usize,
        tokens_in: 0,
        tokens_out: 0,
        elapsed_ms: metric["elapsed_ms"].as_u64().unwrap_or(0),
    }
}

pub struct DaemonTurnEngine {
    pipeline: Arc<TurnPipeline>,
}

impl DaemonTurnEngine {
    pub fn new(pipeline: Arc<TurnPipeline>) -> Self {
        Self { pipeline }
    }
}

#[async_trait]
impl TurnEngine for DaemonTurnEngine {
    async fn execute(
        &self,
        request: TurnEngineRequest,
        context: TurnEngineContext,
        events: Arc<dyn TurnEngineEventSink>,
    ) -> Result<TurnEngineResult, TurnEngineError> {
        let turn_id = fabric::TurnId::new();
        events.on_turn_started(turn_id).await;

        let principal = context.principal_id.clone();
        let model_policy = request
            .model_policy
            .or(context.profile.model_policy.clone())
            .unwrap_or_default();

        let turn_request = fabric::TurnRequest {
            operation_id: context.operation_id,
            process_id: context.process_id,
            context: fabric::PrincipalContext::new(
                principal.clone(),
                fabric::LocalOsPrincipal { uid: 0, gid: 0 },
                fabric::ConnectionId::default(),
                fabric::ThreadId(principal.0.clone()),
                (*context.workspace).clone(),
                fabric::PermissionProfileId("daemon".into()),
                fabric::ApprovalPolicy::OnRequest,
            ),
            input: request.input.clone(),
            model_policy: Some(model_policy),
            deadline: request.deadline,
        };

        {
            let mut guard = self.pipeline.current_scope.lock().await;
            *guard = Some(kernel::operation::OperationScope::new(context.operation_id));
        }

        let cancel: CancellationToken = context.cancel_token.clone();
        let response = self
            .pipeline
            .run(
                serde_json::Value::Null,
                turn_request.input.clone(),
                turn_request.clone(),
                context.operation_id,
                context.process_id,
                cancel,
                principal,
            )
            .await?;

        let result = map_pipeline_response(turn_id, &response);
        events.on_turn_settled(turn_id, &result).await;
        Ok(result)
    }
}
```

- [ ] **Step 4: Declare the module**

Edit `crates/executive/src/service/mod.rs` — add in alpha order near `pub mod daemon_turn;`:

```rust
pub mod daemon_turn_engine;
```

- [ ] **Step 5: Run test to verify it passes**

Run: `bash scripts/cargo-agent.sh test -p executive --test daemon_turn_engine`
Expected: PASS (both mapping tests).

- [ ] **Step 6: Commit**

```bash
git add crates/executive/src/service/daemon_turn_engine.rs crates/executive/src/service/mod.rs crates/executive/tests/daemon_turn_engine.rs
# Suggested subject: feat(executive): add DaemonTurnEngine implementing TurnEngine contract
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 5: Parity test — daemon vs exec produce isomorphic lifecycle

**Files:**
- Modify: `crates/executive/tests/turn_engine_parity.rs`

Reuse the existing `TurnEngineParitySnapshot` type and `CountingEventSink`/`StubTurnEngine` scaffolding already in that test file. Assert that a `TurnEngineResult` from a successful run yields the same parity snapshot fields regardless of which engine produced it.

- [ ] **Step 1: Write the failing test**

Append to `crates/executive/tests/turn_engine_parity.rs`:

```rust
fn snapshot_of(result: &TurnEngineResult) -> TurnEngineParitySnapshot {
    TurnEngineParitySnapshot {
        turn_id: result.turn_id,
        output_len: result.output.len(),
        tool_calls: result.tool_calls,
        status: result.status.clone(),
        tokens_in: result.tokens_in,
        tokens_out: result.tokens_out,
    }
}

#[tokio::test]
async fn daemon_mapping_matches_engine_result_snapshot() {
    // A DaemonTurnEngine response and a stub engine result with identical
    // observable fields must yield identical parity snapshots.
    let turn_id = fabric::TurnId::new();
    let mapped = executive::service::daemon_turn_engine::map_pipeline_response(
        turn_id,
        &serde_json::json!({
            "result": { "response": "ok", "succeeded": true,
                        "metrics": { "tool_calls_made": 2, "elapsed_ms": 5 } }
        }),
    );
    let stub = TurnEngineResult {
        turn_id,
        output: "ok".into(),
        status: TurnEngineStatus::Completed,
        tool_calls: 2,
        tokens_in: 0,
        tokens_out: 0,
        elapsed_ms: 5,
    };
    assert_eq!(snapshot_of(&mapped), snapshot_of(&stub));
}
```

- [ ] **Step 2: Run test to verify it fails / compiles-then-passes**

Run: `bash scripts/cargo-agent.sh test -p executive --test turn_engine_parity daemon_mapping_matches_engine_result_snapshot`
Expected: initially FAIL only if imports missing; add `use executive::service::turn_engine::{TurnEngineResult, TurnEngineStatus, TurnEngineParitySnapshot};` at the top if not present.

- [ ] **Step 3: Fix imports as needed, re-run**

Run: `bash scripts/cargo-agent.sh test -p executive --test turn_engine_parity`
Expected: PASS (all parity tests including the new one).

- [ ] **Step 4: Commit**

```bash
git add crates/executive/tests/turn_engine_parity.rs
# Suggested subject: test(executive): parity between DaemonTurnEngine mapping and engine result
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 6: Switch the daemon closure to call `DaemonTurnEngine`

**Files:**
- Modify: `crates/executive/src/service/daemon_turn/orchestrator.rs:37-67` (hold an engine)
- Modify: `crates/executive/src/service/daemon_turn/execute.rs:154-291`
- Modify: `crates/executive/src/impl/daemon/bootstrap/services.rs` (construct engine)

This is the strangler cut-over. The coordinator closure keeps its post-turn projection and metrics mapping (those stay in `execute.rs`), but the `pipeline.run(...)` call is replaced by `engine.execute(...)`, and the result is read from `TurnEngineResult` instead of raw JSON. Behavior is preserved because `DaemonTurnEngine` runs the same pipeline.

- [ ] **Step 1: Extend `DaemonTurnResources` and struct to carry the engine**

Edit `crates/executive/src/service/daemon_turn/orchestrator.rs`. Add to `DaemonTurnResources` (after `pipeline`):

```rust
    pub(crate) turn_engine: Arc<dyn crate::service::turn_engine::TurnEngine>,
```

Add to `DaemonTurnOrchestrator` (after `pipeline`):

```rust
    pub(crate) turn_engine: Option<Arc<dyn crate::service::turn_engine::TurnEngine>>,
```

Wire it in `new()`:

```rust
            turn_engine: Some(resources.turn_engine),
```

- [ ] **Step 2: Construct the engine at bootstrap and pass it in**

Edit `crates/executive/src/impl/daemon/bootstrap/services.rs` where `DaemonTurnResources { ... }` is built (near the `TurnPipeline::new` construction at `services.rs:376-398`). After the `pipeline` is available:

```rust
    let turn_engine: Arc<dyn crate::service::turn_engine::TurnEngine> = Arc::new(
        crate::service::daemon_turn_engine::DaemonTurnEngine::new(pipeline.clone()),
    );
```

and add `turn_engine,` to the `DaemonTurnResources { ... }` literal.

- [ ] **Step 3: Replace the inline `pipeline.run` with `engine.execute`**

In `crates/executive/src/service/daemon_turn/execute.rs:202-219`, inside the `submit_with` closure, replace the block that locks `current_scope` and calls `pipeline.run(...)` with a call through the engine. Build a `TurnEngineRequest`/`TurnEngineContext` from the `request` and run:

```rust
                let engine = self
                    .turn_engine
                    .clone()
                    .expect("production daemon orchestrator has a turn engine");
                let ctx = crate::service::turn_engine::TurnEngineContext {
                    principal_id: principal.clone(),
                    operation_id: request.operation_id,
                    process_id: request.process_id,
                    workspace: Arc::new(request.context.workspace.clone()),
                    profile: /* ResolvedTurnProfile from active_profile snapshot */,
                    cancel_token: cancel.clone(),
                };
                let engine_req = crate::service::turn_engine::TurnEngineRequest {
                    input: request.input.clone(),
                    model_policy: request.model_policy.clone(),
                    deadline: request.deadline,
                };
                let sink: Arc<dyn crate::service::turn_engine::TurnEngineEventSink> =
                    Arc::new(crate::service::daemon_turn::NoopEngineSink);
                let engine_result = engine.execute(engine_req, ctx, sink).await
                    .map_err(anyhow::Error::from)?;
```

> **Blocker note for the implementer:** `TurnEngineContext.profile` requires a `ResolvedTurnProfile`. The daemon closure currently derives `model_policy` from `active_profile.snapshot()` but does not build a full `ResolvedTurnProfile`. Before writing Step 3, read `crates/executive/src/service/turn_runtime_ports.rs` for the `ResolvedTurnProfile` constructor and `crates/executive/src/service/turn_coordinator.rs` for what `submit_with` passes. If a full profile is not cheaply available here, this task is too large as one step — split it: (6a) add a `ResolvedTurnProfile` accessor to the coordinator turn context; (6b) then do the cut-over. Do NOT fabricate profile fields.

- [ ] **Step 4: Map `TurnEngineResult` back to the closure's `TurnExecution`**

Replace the JSON-field reads (`execute.rs:229-262`) that build `metrics`/`output`/`succeeded` with reads from `engine_result`:

```rust
                let output = engine_result.output.clone();
                let succeeded = engine_result.status
                    == crate::service::turn_engine::TurnEngineStatus::Completed;
                let metrics = fabric::TurnMetrics {
                    tool_calls_made: engine_result.tool_calls,
                    tool_errors: 0,
                    elapsed_ms: engine_result.elapsed_ms,
                    iterations: 0,
                    completed_normally: succeeded,
                };
```

> **Blocker note:** the current closure also builds `PostTurnDispatch`/`context_projection` from `result["projection"]`, which `TurnEngineResult` does not carry. Either (a) extend `TurnEngineResult` with an optional `projection: Option<serde_json::Value>` populated by `DaemonTurnEngine` from the pipeline response, or (b) keep the projection read by having `DaemonTurnEngine::execute` also return the raw response. Option (a) is cleaner; add the field in Task 4 retroactively if you reach here. This is the riskiest step — gate it behind the parity test in Step 5.

- [ ] **Step 5: Run the daemon turn tests (behavior preservation)**

Run: `bash scripts/cargo-agent.sh test -p executive daemon_turn`
Expected: PASS — `execute_turn_success_runs_kernel_and_coordinator_lifecycle` and `execute_turn_error_settles_operation_and_returns_json_rpc_error` (`execute.rs:320-372`) still pass unchanged, proving the cut-over is behavior-preserving.

- [ ] **Step 6: Commit**

```bash
git add crates/executive/src/service/daemon_turn/orchestrator.rs crates/executive/src/service/daemon_turn/execute.rs crates/executive/src/impl/daemon/bootstrap/services.rs
# Suggested subject: refactor(executive): route daemon turns through DaemonTurnEngine (strangler cut-over)
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Task 7: Full regression + completion check

- [ ] **Step 1: Run the P0 acceptance set**

```bash
bash scripts/cargo-agent.sh test -p cognit
bash scripts/cargo-agent.sh test -p executive --test turn_engine_parity
bash scripts/cargo-agent.sh test -p executive daemon_turn
bash scripts/cargo-agent.sh build --workspace
```

Expected: all PASS + clean build.

- [ ] **Step 2: Verify config selection is observable**

Start the daemon with `harness_kind = "linear"` and confirm the log line
`cognitive harness selected from config harness=linear` appears. Separately run
configuration preflight with `harness_kind = "robot"`; it must fail before daemon startup
with an unknown-variant error and must not emit a Robot-selected log line.

- [ ] **Step 3: Commit any test-only fixups**

```bash
git status --short
# Stage only files owned by this task.
# Suggested subject: test(executive): P0 turn-convergence acceptance green
# Inspect the staged diff, then commit with the required problem paragraph,
# solution paragraph, and one concrete bullet per changed file.
```

---

## Completion criteria (maps to spec §4.2)

- daemon and exec both reach the turn via a `TurnEngine` implementation (`DaemonTurnEngine` / `SessionTurnEngine`).
- `turn_engine_parity` proves isomorphic observable results.
- `harness_kind` from config is read at runtime and logged; `Linear` behavior unchanged.
- `robot` configuration fails closed; P3 owns adding `HarnessKind::Robot` together with its real factory.
- No `TurnPipeline` field split; no new daemon entry; no EventBus change.

## Known risk carried into implementation

Tasks 6.3–6.4 depend on `ResolvedTurnProfile` availability and projection plumbing inside the coordinator closure. If either is not cheaply reachable, split Task 6 as noted rather than fabricating fields. If the split itself proves large, stop at Task 5: `DaemonTurnEngine` + parity already establishes the daemon-side `TurnEngine` implementation, and the cut-over can be its own follow-up plan.
