# C02 Conscious Processor Integration Implementation Plan

**Goal:** Register bounded Mnemosyne, Metacog, Corpus and SubAgent processors and expose a read-only conscious-core inspector.

**Architecture:** Thin adapters translate C01 broadcasts to the F01 domain facades and translate bounded results back to candidates without bypassing selection, policy or scope.

**Tech Stack:** Rust, Tokio, C01 processor port, Mnemosyne, Metacog, Corpus, AgentControl facades

**Source requirements:** `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:867-884`.

**Prerequisites:** M05, G10, C01, F01 and Q02.

## Current-code anchors

- Agora processors implement `BroadcastProcessor` at `crates/agora/src/broadcast/mod.rs:19-28`.
- Metacog candidate bridging exists at `crates/metacog/src/bridge/candidate_bridge.rs:9-30`; current bootstrap construction remains separate at `crates/executive/src/impl/daemon/bootstrap/request.rs:37-38`.
- Cognit emits harness events through `EventSink` at `crates/cognit/src/harness/event_sink.rs:140-169`.
- Interact has debug UI plumbing at `crates/interact/src/tui/debug.rs:208` but no conscious-core inspector contract.

## Invariants and non-goals

- Processors do not mutate Agora or Dasein directly.
- Private child/mailbox content is not inspector-visible.
- The inspector is read-only and contains no hidden reasoning.

## Key contracts

```rust
pub struct ProcessorRegistration { pub id: ProcessorId, pub schemas: Vec<SchemaId>, pub capacity: usize, pub deadline_ms: u64, pub response_visibility: VisibilityScope }
pub struct ConsciousCoreSnapshot { pub epoch: BroadcastEpoch, pub dispositions: Vec<CandidateDisposition>, pub acknowledgements: Vec<ProcessorAck>, pub dasein_version: SelfVersion }
```

## Task 1: Add processor integration fixtures

**Create:** `crates/executive/tests/conscious_processors.rs`

- [ ] Test memory recall produces private candidates and only selected memory becomes context.
- [ ] Test Metacog calibration/conflict output changes deliberation without directly mutating self.
- [ ] Test Corpus action proposals execute only after selection and governed capability approval.
- [ ] Test child evidence preserves Agent provenance and follows G09/M08 promotion rules.

Run: `cargo test -p executive --test conscious_processors`

## Task 2: Implement Mnemosyne and Metacog processors

**Create:** `crates/executive/src/impl/conscious/memory_processor.rs`
**Create:** `crates/executive/src/impl/conscious/metacog_processor.rs`
**Create:** `crates/executive/src/impl/conscious/mod.rs`

- [ ] Adapt M04 projection to C01 candidates and enqueue M05 experience work asynchronously.
- [ ] Emit calibration, uncertainty, conflict and governed mutation proposals from the F01 Metacog facade.
- [ ] Prevent recalled/adversarial content from directly changing identity, care or boundary state.

Run: `cargo test -p executive --test conscious_processors`

## Task 3: Implement Corpus and SubAgent processors

**Create:** `crates/executive/src/impl/conscious/corpus_processor.rs`
**Create:** `crates/executive/src/impl/conscious/agent_processor.rs`

- [ ] Convert selected action proposals to E03 invocation and return typed outcome candidates.
- [ ] Consume child Agent progress/evidence/results from G07/G10 without exposing private mailbox content.
- [ ] Attribute root/child/user/environment/external-memory sources and attach promotion receipts where required.
- [ ] Keep persistent child-self creation outside ordinary G03 spawn.

Run: `cargo test -p executive --test conscious_processors`

## Task 4: Register processors with bounded policy

**Modify:** `crates/executive/src/impl/daemon/bootstrap/request.rs`
**Modify:** `crates/executive/src/service/conscious_core_coordinator.rs`

- [ ] Register processor IDs, accepted content schemas, queue capacity, deadline and response visibility.
- [ ] Isolate processor failure and record timeout/rejection acknowledgements.
- [ ] Apply source quotas and prevent one processor from monopolizing recurrence.

Run: `cargo test -p executive --test conscious_processors && cargo test -p agora --all-targets`

## Task 5: Add a read-only inspector

**Create:** `crates/fabric/src/protocol/conscious_core.rs`
**Create:** `crates/executive/src/service/conscious_core_inspector.rs`
**Create:** `crates/interact/src/tui/conscious_core.rs`
**Create:** `crates/interact/tests/conscious_core_inspector.rs`

- [ ] Expose candidate disposition, salience, winner/coalition, broadcast acknowledgements and Dasein version references.
- [ ] Exclude hidden reasoning, secrets and private child/memory content.
- [ ] Render explicit functional-indicator limitations and degraded processor state.

Run: `cargo test -p interact --test conscious_core_inspector`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `feat(conscious-core): integrate bounded processors` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] All four processor adapters pass deterministic integration tests.
- [ ] Self-attribution distinguishes root, child, user, environment and supplemental memory.
- [ ] Inspector is read-only and contains no sensitive reasoning payloads.
