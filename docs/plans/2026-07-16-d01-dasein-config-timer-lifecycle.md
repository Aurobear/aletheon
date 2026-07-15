# D01 Dasein Configuration, Timer and Lifecycle Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make Dasein construction honor configured temporality and make its event loop deterministically start, stop and restart through injected timing.

**Architecture:** Introduce a validated `DaseinRuntimeConfig` and an object-safe local sleep port. Sorge reacts immediately to queued events, uses injected timing only for scheduled reflection, keeps receiver/task ownership across stops, and exposes async shutdown so no background task survives subsystem shutdown.

**Tech Stack:** Rust, Tokio channels/tasks/Notify, injected Fabric Clock, Kernel SystemTimer adapter.

**Prerequisites:** S02.

**Source requirements:** `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:782-804`, specifically configured retention/decay and restartable event-driven Sorge timing at `:789-795`.

---

## Current-code anchors

- SelfField has retention/decay config but ignores it when calling `DaseinModule::new` at `crates/dasein/src/core/mod.rs:47-65` and `:143-148`.
- Dasein hard-codes `50/0.8` at `crates/dasein/src/dasein/mod.rs:57-67`.
- Sorge consumes its receiver once, sleeps through concrete SystemTimer and only flips an atomic on stop at `crates/dasein/src/dasein/sorge.rs:29-176`.
- SelfField shutdown does not await Sorge termination at `crates/dasein/src/core/mod.rs:349-365`.

## Invariants and non-goals

- Invalid depth, decay or buffer values fail construction.
- Start is idempotent while running; stop awaits the owned task.
- After stop, the same DaseinModule can restart and receive new events.
- All Sorge sleeps use the injected object-safe timer port.
- D01 does not introduce the versioned self reducer or change event semantics.

## File map

- Create: `crates/dasein/tests/dasein_runtime_lifecycle.rs`
- Modify: `crates/dasein/src/dasein/mod.rs`
- Modify: `crates/dasein/src/dasein/sorge.rs`
- Modify: `crates/dasein/src/core/mod.rs`

### Task 1: Add truthful validated construction

- [x] Add `DaseinRuntimeConfig { retention_depth, decay_rate, event_buffer }`.
- [x] Add `DaseinModule::with_runtime(clock, timer, config) -> Result<(Self, Sender)>`.
- [x] Keep `new(clock)` as a compatibility constructor using defaults.
- [x] Pass SelfField `dasein_retention_depth` and `dasein_decay_rate` into the runtime config.

Run: `cargo test -p dasein --test dasein_runtime_lifecycle configured_`

Expected: configured retention evicts at the requested depth and invalid values are rejected.

### Task 2: Inject Sorge timing

- [x] Define object-safe async `SorgeTimer::sleep` and Kernel `SystemTimer` adapter.
- [x] Store `Arc<dyn SorgeTimer>` in SorgeLoop.
- [x] Replace polling/blocking sleeps with an event-or-scheduled-reflection select.
- [x] Use a deterministic test timer that records/wakes sleepers.

Run: `cargo test -p dasein --test dasein_runtime_lifecycle injected_timer_drives_idle_loop`

Expected: the loop makes no wall-clock dependency and advances when the test timer wakes it.

### Task 3: Make lifecycle restartable and owned

- [x] Keep receiver ownership in shared state and return it after task exit.
- [x] Store the JoinHandle, reject duplicate start, notify stop and await termination.
- [x] Restart, send another event and prove temporal position advances exactly once per event.
- [x] Make SelfField shutdown await Dasein stop.

Run: `cargo test -p dasein --test dasein_runtime_lifecycle start_stop_restart`

Expected: PASS with no orphan task.

### Task 4: Verify and commit

```bash
cargo fmt --all -- --check
cargo clippy -p dasein --all-targets -- -D warnings
cargo test -p dasein
cargo test --workspace
bash tests/architecture_check.sh
bash scripts/architecture-check.sh
```

Commit subject: `fix(dasein): make configured lifecycle restartable`

## Compatibility deletion gate

The `DaseinModule::new(clock)` compatibility constructor remains for tests and non-SelfField callers. D02 must route production self changes through its reducer; D03 may then narrow construction to the composition root.

## Completion evidence

- [x] retention/decay are no longer hard-coded in production;
- [x] all Sorge scheduling is injected and incoming events are not blocked by sleeps;
- [x] duplicate start is harmless;
- [x] stop awaits task exit;
- [x] restart processes new events;
- [x] workspace and architecture checks pass.
