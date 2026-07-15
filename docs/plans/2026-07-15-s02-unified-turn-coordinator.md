# S02 Unified Turn Coordinator Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make daemon and CLI exec enter one `TurnCoordinator`, create a real `OperationKind::Turn`, use the governed invoker, and append identical canonical lifecycle items.

**Architecture:** `TurnCoordinator` owns process/operation lifecycle, canonical appends, Cognit session creation, timeout/cancellation, and terminal settlement. Mode differences are a `TurnPolicy` value rather than separate orchestration implementations. Thin daemon/exec adapters translate input and project events only.

**Tech Stack:** Rust, Tokio, Cognit Harness, Kernel operation tables, E03 capability invoker, S01 append store

**Prerequisites:** E03 governed parity suite and S01 append/reopen suite pass.

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:981-1006`, `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1192-1199`.

---

## Current-code anchors and invariants

- `TurnService` already creates `OperationKind::Turn`: `crates/executive/src/service/turn_service.rs:51-158`.
- Daemon creates the wrong `SubAgent` kind: `crates/executive/src/service/daemon_turn/execute.rs:61-80`.
- Exec owns a separate state machine: `crates/executive/src/service/exec_session.rs:91-316`.
- Every started operation reaches exactly one of succeed/fail/cancel, including pre-turn, model, append, timeout, and event-sink errors.
- Canonical items reproduce the next input; memory and trace remain separate.
- Non-goals: memory consolidation and SubAgent runtime migration. Resume, fork, interrupt, replay, and duplicate ReAct removal are required here by the Phase 2 source.

## Resolved design conflicts

| Earlier statement | Code reality | Resolution |
|---|---|---|
| Record start/succeed/fail through `OperationManager` | That trait exposes only submit/cancel/wait: `crates/fabric/src/include/process.rs:32-37` | Temporarily inject the existing concrete `OperationTable` from `ServicePorts`; Phase 3 later completes the port |
| Async cleanup from `Drop` settles every path | Rust `Drop` cannot await operation-table calls | Put fallible work in `run_started_turn`; `submit` always awaits one terminal transition after matching its outcome |
| Lifecycle commands could be deferred | Source explicitly requires resume/fork/interrupt/replay: `architecture-coupling...:997-998` | Implement and integration-test all four in this plan |
| Existing harness factory is object-safe | It returns concrete `ReActLoop`: `crates/executive/src/service/harness_factory.rs:24-39` | Add a factory returning `Box<dyn CognitiveSession>` and remove direct production loop ownership |

```text
daemon adapter --\
                 TurnCoordinator -> operation Turn -> append input -> Cognit factory
exec adapter ----/       |                                  |
                         `-> TurnPolicy -> governed invoker -+-> append outputs -> settle
```

## File map

- Create: `crates/executive/src/service/turn_coordinator.rs` — one orchestration owner.
- Create: `crates/executive/src/service/turn_policy.rs` — explicit mode policies.
- Modify: `crates/executive/src/service/harness_factory.rs` — object-safe `CognitiveSessionFactory`.
- Modify: `crates/executive/src/service/mod.rs` — exports.
- Create: `crates/executive/src/service/session_service.rs` — lifecycle commands over the S01 store.
- Modify: `crates/executive/src/service/exec_session.rs` — thin adapter.
- Modify: `crates/executive/src/service/daemon_turn/execute.rs` and `turn_pipeline.rs` — thin adapter.
- Modify: bootstrap call sites found with `rg -n 'TurnService::new|ExecSessionBuilder|TurnPipeline::new' crates/executive crates/bin`.
- Create: `crates/executive/tests/turn_coordinator_lifecycle.rs` — lifecycle matrix.
- Modify: `crates/executive/tests/turn_service_equivalence.rs` — daemon/exec contract parity.

### Task 1: Define explicit mode policy and factory contracts

- [ ] Add tests constructing `TurnPolicy::daemon()` and `TurnPolicy::exec()` and asserting only persistence, reviewer, memory eligibility, Agora availability, event sink, and environment/sandbox profile differ.
- [ ] Run `cargo test -p executive --test turn_coordinator_lifecycle policy_contains_all_mode_differences`; expected FAIL: policy absent.
- [ ] Add:

```rust
pub struct TurnPolicy {
    pub persistence: PersistenceMode,
    pub reviewer: ReviewerMode,
    pub memory_eligible: bool,
    pub agora_available: bool,
    pub event_delivery: EventDelivery,
    pub environment: EnvironmentProfile,
}

#[async_trait]
pub trait CognitiveSessionFactory: Send + Sync {
    async fn create(&self, session: &SessionRecord, policy: &TurnPolicy) -> Result<Box<dyn CognitiveSession>>;
}
```

Move existing Harness construction behind the factory; do not add a second model loop.
- [ ] Run the policy test; expected PASS.

### Task 2: Centralize operation ownership

- [ ] Add test `ServicePorts` with its concrete in-memory `OperationTable`, then run a matrix for success, pre-turn error, Cognit error, timeout, cancellation, and append error. Inspect each `OperationRecord`; assert `kind == Turn`, owner equals request process, parent ownership is validated, and one terminal state is recorded.
- [ ] Run the matrix; expected FAIL: coordinator absent.
- [ ] Implement `TurnCoordinator::submit` with the concrete `OperationTable` already carried by `ServicePorts`: submit/start `OperationKind::Turn`, await `run_started_turn`, then match `Completed`, `Failed`, or `Cancelled` and await exactly one `succeed`, `fail`, or `cancel`. Do not use `?` between `start` and the terminal match. Use the injected Clock and operation-scope token for deadline cancellation.
- [ ] Run the matrix; expected PASS with one terminal call per row.

### Task 3: Append the canonical lifecycle

- [ ] Add a successful-turn assertion for ordered items: user message, each tool call/result pair, assistant message; add failure assertion that the user item and a system failure notice remain replayable.
- [ ] Run it; expected FAIL because coordinator does not append.
- [ ] In `submit`, load/create `SessionRecord`, reserve the next sequence, append input before Cognit, collect structured `TurnEvent`s into canonical items, append terminal assistant/failure item, and only then settle the operation. Use stable item IDs derived once per event so retry returns `AlreadyPresent`.
- [ ] Run the lifecycle test; expected PASS and strictly increasing sequences.

### Task 4: Bind one governed invoker into TurnServices

- [ ] Add a test whose fake Cognit emits one tool call; assert the E03 recorder observes review/admit/execute/settle/audit once and the canonical store contains one correlated call/result pair.
- [ ] Run it; expected FAIL before coordinator TurnServices wiring.
- [ ] Implement a private `CoordinatorTurnServices` that delegates `invoke` only to `Arc<dyn CapabilityInvoker>`, supplies LLM/definitions/seed messages, and enriches trusted session/working-directory policy through E03’s context constructor. Remove any admission/registry/runner fields from this service.
- [ ] Run the exact test; expected PASS.

### Task 5: Replace daemon and exec orchestration

- [ ] Expand `turn_service_equivalence.rs` to submit identical scripted input through both adapters and compare operation kind, tool contract, canonical item payloads, stop reason, and output. Normalize only IDs/timestamps.
- [ ] Run it; expected FAIL because paths diverge and daemon uses `SubAgent`.
- [ ] Make `ExecSession` and `DaemonTurnOrchestrator` call the same `Arc<TurnCoordinator>`. Retain their transport event projection, but delete duplicated process/operation, Cognit loop, timeout, tool invocation, and terminal settlement logic. Delete `TurnPipeline` or reduce it to a deprecated internal alias only if an unmigrated test constructor requires it.
- [ ] Run equivalence test; expected PASS and both operation kinds `Turn`.

### Task 6: Restart and failure parity

- [ ] Add a file-backed test: complete a daemon turn, drop all services, reopen, run exec in the same session, and assert the second factory receives context projected from the first turn.
- [ ] Add timeout and cancellation rows for both adapters and assert identical terminal operation and canonical failure notice.
- [ ] Run `cargo test -p executive --test turn_coordinator_lifecycle && cargo test -p executive --test turn_service_equivalence`; expected PASS.

### Task 7: Implement resume, fork, interrupt, and replay

- [ ] Create `crates/executive/tests/session_lifecycle_commands.rs` with four failing tests: resume returns next sequence plus projected context; fork creates a child with `parent_session_id` and copies items only through the requested sequence; interrupt cancels the active Turn and appends one interruption notice; replay projects byte-identical messages without invoking LLM or tools.
- [ ] Run `cargo test -p executive --test session_lifecycle_commands`; expected FAIL because `SessionService` is absent.
- [ ] Implement `SessionService::{resume,fork,interrupt,replay}` over S01 `SessionAppendStore`, a coordinator active-operation index, and the pure projector. Fork writes child record and copied immutable items in one SQLite transaction. Interrupt is idempotent: a second call returns `AlreadyTerminal` and appends no second notice.
- [ ] Route the typed S01 Interact requests in the existing daemon RPC match located with `rg -n '"chat"|match method' crates/executive/src/impl/daemon/handler`; serialize Fabric records rather than hand-built lifecycle response fields.
- [ ] Run the four tests and `cargo test -p interact --test session_protocol`; expected PASS.

### Task 8: Remove duplicate Executive ReAct state

- [ ] Add an architecture test asserting `AletheonExecutive` no longer stores `ReActLoop` and production loop creation occurs only inside `CognitiveSessionFactory`.
- [ ] Run it; expected FAIL at `crates/executive/src/core/orchestrator.rs:24-40`.
- [ ] Remove `react_loop` from `AletheonExecutive`. Return awareness signals in the completed coordinator turn result so post-turn evolution consumes per-turn signals instead of retained loop state.
- [ ] Run `cargo test -p executive --test genome_runtime_mapping && cargo test -p executive --test turn_coordinator_lifecycle`; expected PASS.

### Task 9: Remove migration scaffolding and commit

- [ ] Delete the S01 `LegacyJournalProjector` only after `rg -n 'LegacyJournalProjector|OperationKind::SubAgent' crates/executive/src/service` shows no production turn use. Remove resolved E01 allowlist entries.
- [ ] Run `cargo fmt --all -- --check && cargo test -p executive --test turn_coordinator_lifecycle && cargo test -p executive --test turn_service_equivalence && cargo test -p executive --test session_lifecycle_commands && cargo test -p executive --test governed_capability_path && cargo test -p interact --test session_protocol && cargo test --workspace && bash scripts/architecture-check.sh`.
- [ ] Expected: all pass; architecture output contains no manual daemon/exec capability finding.
- [ ] Commit:

```text
refactor(executive): unify daemon and exec turn lifecycle

Daemon and exec owned separate operation, Cognit, capability, and persistence
flows, causing incorrect operation kinds and divergent failures. Introduce one
policy-driven coordinator and reduce both front ends to transport adapters.

- own Turn operations and terminal settlement in one service
- append canonical items for replayable context
- share Cognit factory and governed capability invocation
```

## Compatibility deletion gate and completion evidence

Delete the old `TurnPipeline`, `TurnService`, and Exec ReAct state when `rg` shows only the coordinator owns `run_turn`, operation submission, and `CapabilityInvoker::invoke`; keep a forwarding type alias for at most one release if external Rust users require source compatibility.

- [ ] Daemon and exec equivalence tests compare the same normalized record stream.
- [ ] Every failure matrix row settles one `OperationKind::Turn` exactly once.
- [ ] Restart projects prior canonical items into the next Cognit session.
- [ ] Resume, fork, interrupt, and replay pass through typed protocol requests.
- [ ] `AletheonExecutive` stores no duplicate ReAct loop.
- [ ] No production front end accesses admission, registry, runner, or settlement.
- [ ] Full workspace and E01 gates pass.
