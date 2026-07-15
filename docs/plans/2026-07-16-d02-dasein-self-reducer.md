# D02 Versioned Dasein Self Reducer Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every accepted production Dasein experience pass through one versioned, idempotent reducer and return a narrative-bearing receipt.

**Architecture:** Fabric defines the stable transition vocabulary. Dasein owns a single serialized state engine that validates event identity, provenance and expected version before applying an explicitly matched interpreted experience. Sorge and external bridges submit transition commands to that engine; they do not mutate lived or reflective state themselves. Existing raw component accessors remain test-only compatibility surfaces until D03 provides replayable snapshots and closes their deletion gate.

**Tech Stack:** Rust, Tokio `mpsc`/`oneshot`/`Mutex`, UUID, existing Fabric Dasein snapshots, Dasein temporal/world/self/care components.

**Prerequisites:** D01 (`df7bced`).

**Source requirements:** `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:378-413`, `:782-804`, specifically contracts/reducer at `:786-791`, receipt/idempotence at `:799-803`, and removal of raw hidden-reasoning ingestion at `:795`.

---

## Current-code anchors

- Fabric exposes an unversioned `DaseinEvent` and `DaseinOps::handle_event` at `crates/fabric/src/dasein/event.rs:7-54` and `crates/fabric/src/dasein/ops.rs:9-39`.
- `DaseinModule::quick_mood_update` mutates mood by keywords at `crates/dasein/src/dasein/mod.rs:188-211`.
- Sorge matches only three event variants and silently skips all others at `crates/dasein/src/dasein/sorge.rs:127-151`.
- Daemon turn completion invokes the keyword adapter from three production paths at `crates/executive/src/service/daemon_turn/self_field.rs:20-62` and `crates/executive/src/impl/daemon/handler/mod.rs:176-184`.
- Raw component accessors and setters remain public throughout `crates/dasein/src/dasein/mod.rs:213-230`, `self_model.rs:93-169`, `care_structure.rs:162-230`, and `bewandtnis.rs:65-119`.

## Invariants and non-goals

- An accepted request increments `SelfVersion` exactly once; a duplicate event ID returns the original receipt and never reapplies state.
- A new event with a stale expected version fails without changing any component.
- Every `InterpretedExperience` variant is matched explicitly; there is no wildcard discard path.
- Raw chain-of-thought/reasoning text has no transition variant and cannot enter Dasein.
- Outcome mood is selected by structured status, never keyword scanning.
- Every receipt contains a stable narrative entry ID and emitted typed signals.
- D02 keeps idempotence receipts in memory. Durable events, snapshots, checksums, replay and causal lineage belong to D03.

## File map

- Create: `crates/fabric/src/dasein/transition.rs`
- Create: `crates/dasein/src/dasein/reducer.rs`
- Create: `crates/dasein/tests/dasein_transition_contract.rs`
- Modify: `crates/fabric/src/dasein/mod.rs`
- Modify: `crates/fabric/src/dasein/event.rs`
- Modify: `crates/fabric/src/dasein/ops.rs`
- Modify: `crates/dasein/src/dasein/mod.rs`
- Modify: `crates/dasein/src/dasein/sorge.rs`
- Modify: `crates/dasein/src/dasein/event_bridge.rs`
- Modify: `crates/executive/src/service/daemon_turn/self_field.rs`
- Modify: `crates/executive/src/impl/daemon/handler/mod.rs`

### Task 1: Define bounded transition contracts in Fabric

- [x] Add `SelfVersion`, `SelfEventId`, `NarrativeEntryId`, `ExperienceSource`, `ExperienceProvenance`, `OutcomeStatus`, `InterpretedExperience`, `SelfSignal`, `SelfTransitionRequest`, and `SelfTransitionReceipt`.
- [x] Validate non-empty source identity, finite confidence values and bounded text/list fields.
- [x] Remove `ThinkingObserved` and `ReasoningObserved`; retain `DaseinEvent` only as a compatibility input whose variants all have explicit safe mappings.
- [x] Add `DaseinOps::transition` and make compatibility `handle_event` return a receipt.

Run: `cargo test -p fabric dasein::transition`

Expected: contract validation, serde round trips and version ordering pass; hidden-reasoning variants no longer compile or serialize.

### Task 2: Build the serialized idempotent reducer

- [x] Add one `DaseinStateEngine` owning version, dedup receipts and the authoritative component references.
- [x] Validate/deduplicate under one async transition lock before mutation.
- [x] Explicitly reduce lived input, system observation, structured outcome, asserted knowledge, negation, readiness change, mood observation and scheduled reflection.
- [x] Advance temporality only for accepted lived experiences.
- [x] Return stable narrative IDs and structured signals from every accepted request.

Run: `cargo test -p dasein --test dasein_transition_contract reducer_`

Expected: accepted, stale, duplicate and invalid requests have deterministic state/version outcomes.

### Task 3: Route Sorge and bridges through the reducer

- [x] Keep the bounded legacy event receiver only as an ingress adapter; canonical callers use `transition` and receive receipts.
- [x] Submit scheduled reflection through the reducer rather than mutating components in Sorge.
- [x] Convert every retained compatibility event explicitly; reject invalid/unsupported input rather than skipping it.
- [x] Preserve D01 start/stop/restart semantics and prove queued events pass through versioned transitions after restart.

Run: `cargo test -p dasein --test dasein_transition_contract bridge_ && cargo test -p dasein --test dasein_runtime_lifecycle`

Expected: all production event ingress reaches the reducer and lifecycle tests remain green.

### Task 4: Replace keyword mood mutation with structured outcomes

- [x] Add a temporary `record_outcome(summary, OutcomeStatus, producer)` adapter backed by `transition`.
- [x] Update the live daemon turn path to pass explicit success/failure status and update dormant compatibility coordinators to require a status.
- [x] Deprecate `quick_mood_update`; it queues a reducer observation and is not called from production.
- [x] Verify statically that production code has no `quick_mood_update` call.

Run: `cargo test -p executive --lib self_field && ! rg -n 'quick_mood_update' crates/executive/src`

Expected: production mood transitions use structured status and return receipts.

### Task 5: Verify and commit

```bash
cargo fmt --all -- --check
cargo clippy -p fabric -p dasein -p executive --all-targets -- -D warnings
cargo test -p fabric
cargo test -p dasein
cargo test -p executive
cargo test --workspace
bash tests/architecture_check.sh
bash scripts/architecture-check.sh
```

Commit subject: `feat(dasein): route experiences through one reducer`

## Compatibility deletion gate

- `DaseinEvent`, `quick_mood_update`, and raw component accessors are compatibility-only after D02.
- D03 must persist/replay the reducer state and provide snapshot restoration before `DaseinEvent` and direct restore setters can be removed.
- C01 must consume `SelfSignal` before internal signal compatibility paths can be deleted.
- The gate closes only when `rg` shows no production raw mutator or keyword adapter call and D03 replay tests reconstruct the same version and state checksum.

## Completion evidence

- [x] Fabric transition contracts validate and round-trip;
- [x] duplicate IDs return byte-equivalent receipts without mutation;
- [x] stale versions fail without mutation;
- [x] all interpreted variants have explicit reducer tests;
- [x] raw reasoning events are absent;
- [x] daemon outcome paths use structured status;
- [x] D01 lifecycle behavior remains green;
- [x] workspace and architecture checks pass.
