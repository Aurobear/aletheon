# C01 Recurrent Workspace Coordinator Implementation Plan

**Goal:** Close the Dasein–Agora loop so selected content changes self-state and processor responses can causally affect later selection.

**Architecture:** Executive coordinates Dasein modulation, Agora selection/broadcast and bounded processor responses; every recurrence edge is persisted through typed IDs and budgets.

**Tech Stack:** Rust, Tokio, Dasein reducer, Agora competition/broadcast, Kernel budgets

**Source requirements:** `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:847-865`.

**Prerequisites:** D03, A03, K02, X02 and S02.

## Current-code anchors

- Dasein reducer and durable ledger are `DaseinStateEngine` at `crates/dasein/src/dasein/reducer.rs:28` and `SelfLedger` at `crates/dasein/src/dasein/ledger.rs:11`.
- Agora competition and broadcast are implemented in `crates/agora/src/competition/mod.rs:151-415` and `crates/agora/src/broadcast/mod.rs:19-236`.
- Workspace candidate, selection and broadcast contracts exist at `crates/fabric/src/types/workspace.rs:158-454`.
- Executive turn coordination currently centers on `TurnCoordinator` at `crates/executive/src/service/turn_coordinator.rs:34`.

## Invariants and non-goals

- The coordinator does not implement domain state internally.
- Prompt construction is a projection, not the integration mechanism.
- Unselected content cannot mutate Dasein or invoke actions.

## Key contracts

```rust
#[async_trait] pub trait ConsciousProcessor: Send + Sync { fn id(&self) -> ProcessorId; async fn on_broadcast(&self, broadcast: WorkspaceBroadcast, ctx: ProcessorContext) -> ProcessorResponse; }
pub struct ProcessorResponse { pub source_epoch: BroadcastEpoch, pub candidates: Vec<WorkspaceCandidate>, pub acknowledgements: Vec<ProcessorAck> }
```

## Task 1: Freeze recurrent-loop causality

**Create:** `crates/executive/tests/conscious_core_recurrence.rs`

- [ ] Drive observation -> candidate -> selection -> broadcast -> Dasein integration -> processor response -> later selection.
- [ ] Assert every edge carries event, content, broadcast epoch and Dasein version references.
- [ ] Assert unselected/expired content never enters global context.
- [ ] Assert deterministic clock/IDs reproduce the same winner and state transition.

Run: `cargo test -p executive --test conscious_core_recurrence`

## Task 2: Define processor and context ports

**Create:** `crates/fabric/src/types/conscious_core.rs`
**Modify:** `crates/fabric/src/types/mod.rs`
**Create:** `crates/executive/src/service/conscious_core_ports.rs`

- [ ] Define `ConsciousProcessor`, bounded response, candidate submission and latest-context projection contracts.
- [ ] Bind responses to source broadcast epoch and processor identity.
- [ ] Separate observations, recalled experiences, predictions, actions and outcomes in typed content.
- [ ] Define overload, timeout and degraded-processor behavior.

Run: `cargo test -p fabric conscious_core --lib`

## Task 3: Implement ConsciousCoreCoordinator

**Create:** `crates/executive/src/service/conscious_core_coordinator.rs`
**Modify:** `crates/executive/src/service/mod.rs`

- [ ] Admit bounded candidates and request Dasein salience modulation before Agora selection.
- [ ] Commit selection/broadcast through A03 and deliver to eligible processors.
- [ ] Integrate each broadcast into Dasein as one versioned lived event.
- [ ] Submit structured Dasein concerns, projections and protentions as later candidates.
- [ ] Compare prediction/outcome pairs and emit typed prediction-error candidates.
- [ ] Enforce per-cycle work, deadline and recurrence depth budgets through K02.

Run: `cargo test -p executive --test conscious_core_recurrence`

## Task 4: Feed governed action outcomes back

**Modify:** `crates/executive/src/service/turn_pipeline.rs`
**Modify:** `crates/executive/src/service/governed_capability.rs`
**Create:** `crates/executive/tests/conscious_action_outcome.rs`

- [ ] Link selected action candidate to capability permit, operation and outcome observation.
- [ ] Re-enter the outcome through candidate competition rather than direct self mutation.
- [ ] Attribute root, child, user, environment and external-memory sources distinctly.

Run: `cargo test -p executive --test conscious_action_outcome`

## Task 5: Replace prompt-only integration

**Modify:** `crates/executive/src/service/context_assembler.rs`
**Modify:** `crates/executive/src/service/turn_coordinator.rs`

- [ ] Build model context from latest selected broadcast plus bounded structured SelfView.
- [ ] Remove full self/memory dumps and direct store queries from assembly.
- [ ] Persist a projection receipt that identifies every included content/version.

Run: `cargo test -p executive context_assembler --all-targets && cargo test --workspace --all-targets --no-fail-fast`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `feat(conscious-core): close recurrent workspace loop` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] One external observation is traceable through selection, self integration, action and outcome recurrence.
- [ ] Dasein modulation changes later selection in a controlled fixture.
- [ ] Broadcast causally changes at least two registered processors.
