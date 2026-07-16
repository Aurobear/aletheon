# R01 Canonical Event Spine Implementation Plan

**Goal:** Make EnvelopeV2 the only durable event envelope with one ordered sequence per Session/Agent tree and explicit raw-observation separation.

**Architecture:** Fabric defines versioned EnvelopeV2 event contracts; Executive owns one append-only SQLite repository that allocates tree order transactionally before projections run.

**Tech Stack:** Rust, SQLite/rusqlite, EnvelopeV2, canonical Session identity

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1117-1124`.

**Prerequisite:** S02.

## Current-code anchors

- `EnvelopeV2` is defined at `crates/fabric/src/ipc/envelope_v2.rs:144-253` and still converts legacy envelopes at `crates/fabric/src/ipc/envelope_v2.rs:256-272`.
- `EventLog` explicitly remains based on `dyn Event` at `crates/fabric/src/events/event_log.rs:2-4`.
- The legacy `Event` trait and handler type remain at `crates/fabric/src/events/types.rs:100-131`.
- Canonical Session item sequencing exists at `crates/executive/src/impl/session/canonical_store.rs:90-128`.

## Invariants and non-goals

- Raw runtime payloads are not automatically model-visible transcript items.
- Trace remains non-authoritative.
- Unknown schema versions are rejected rather than coerced.

## Key contracts

```rust
pub struct EventPosition { pub tree_id: EventTreeId, pub event_id: EventId, pub parent: Option<EventId>, pub sequence: u64 }
pub struct SpineEvent { pub position: EventPosition, pub schema: SchemaId, pub visibility: EventVisibility, pub payload: EventPayload }
pub trait EventSpine { fn append(&self, event: UnsequencedEvent) -> anyhow::Result<SpineEvent>; }
```

## Task 1: Specify tree ordering and schema rejection

**Create:** `crates/fabric/tests/event_spine_contract.rs`

- [ ] Test unique event ID, root/session/agent identity, parent event, tree sequence and schema version.
- [ ] Test duplicate idempotency, conflicting duplicate rejection and monotonic allocation under concurrency.
- [ ] Test unknown/mismatched schemas fail explicitly rather than converting to a legacy type.

Run: `cargo test -p fabric --test event_spine_contract`

## Task 2: Extend EnvelopeV2 event metadata

**Modify:** `crates/fabric/src/ipc/envelope_v2.rs`
**Create:** `crates/fabric/src/events/spine.rs`
**Modify:** `crates/fabric/src/events/mod.rs`

- [ ] Add typed `EventTreeId`, `EventId`, `ParentEventId`, `TreeSequence` and payload visibility.
- [ ] Define versioned schemas for transcript events, control events and raw observation references.
- [ ] Validate target, causal parent, sequence and payload/reference exclusivity.
- [ ] Keep large/sensitive raw payloads outside model-visible transcript events.

Run: `cargo test -p fabric --test event_spine_contract`

## Task 3: Add an append-only event repository

**Create:** `crates/executive/src/impl/events/mod.rs`
**Create:** `crates/executive/src/impl/events/sqlite_event_spine.rs`
**Create:** `crates/executive/tests/event_spine_repository.rs`

- [ ] Allocate each tree sequence transactionally in SQLite.
- [ ] Append envelope metadata and payload/reference atomically.
- [ ] Expose bounded reads by tree, sequence range, schema and visibility.
- [ ] Reopen after simulated crash without gaps caused by acknowledged events.

Run: `cargo test -p executive --test event_spine_repository`

## Task 4: Migrate producers and remove legacy conversion

**Modify:** `crates/executive/src/service/turn_coordinator.rs`
**Modify:** `crates/fabric/src/ipc/stream.rs`
**Modify:** `crates/fabric/src/ipc/bus/kernel_bus.rs`
**Delete:** `crates/fabric/src/events/event_log.rs`

- [ ] Append turn, tool, Agent and lifecycle events through the spine before downstream projection.
- [ ] Replace `Box<dyn Event>` publication and subscriptions with schema-filtered EnvelopeV2 handling.
- [ ] Delete legacy `Envelope -> EnvelopeV2` conversion after all producers migrate.
- [ ] Expose bounded backpressure and rejected-append metrics.

Run: `if rg -n 'Box<dyn Event>|&dyn Event|from_legacy' crates --glob '*.rs'; then exit 1; fi; cargo test --workspace --all-targets --no-fail-fast`

Expected final grep: no production matches.

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `feat(events): establish canonical envelope spine` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] Recorded events have stable causal tree order after restart.
- [ ] Raw runtime evidence is distinguishable from model-visible Session items.
- [ ] Legacy event/envelope coercion is absent from production code.
