# R02 Deterministic Event Projections Implementation Plan

**Goal:** Rebuild every public and operational derived view deterministically from the R01 spine.

**Architecture:** Independent reducers consume R01 envelopes in sequence and atomically commit derived state with their own versioned checkpoint and checksum.

**Tech Stack:** Rust, SQLite/rusqlite, R01 event spine, deterministic reducer fixtures

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:1125-1139`.

**Prerequisite:** R01.

## Current-code anchors

- Public Session/Item storage currently owns an independent sequence at `crates/executive/src/impl/session/canonical_store.rs:30-38`.
- Goal projection evidence is assembled separately at `crates/executive/src/impl/goal/verification.rs:316-343`.
- Memory projection currently records completion events directly at `crates/executive/src/impl/memory_projection.rs:143-175`.
- Agora broadcast replay already demonstrates deterministic domain replay at `crates/agora/src/broadcast/store.rs:12-118`.

## Invariants and non-goals

- Projection tables are never handler-owned authorities.
- One poisoned projection does not stop unrelated reducers.
- Debug projection excludes hidden reasoning and raw secrets.

## Key contracts

```rust
pub trait EventProjection { type State; fn descriptor(&self) -> ProjectionDescriptor; fn apply(&self, state: &mut Self::State, event: &SpineEvent) -> Result<(), ProjectionError>; }
pub struct ProjectionCheckpoint { pub projection: String, pub version: u32, pub through_sequence: u64, pub checksum: String }
```

## Task 1: Define a common reducer checkpoint contract

**Create:** `crates/executive/src/service/event_projection.rs`
**Create:** `crates/executive/tests/event_projection_contract.rs`

- [ ] Define projection name/version, accepted schemas, input watermark, state checksum and transactional checkpoint.
- [ ] Require pure event application plus an atomic state/checkpoint commit.
- [ ] Test duplicate delivery, restart, schema upgrade and rebuild from sequence zero.

Run: `cargo test -p executive --test event_projection_contract`

## Task 2: Build public Session/Turn/Item projection

**Create:** `crates/executive/src/impl/events/session_projection.rs`
**Modify:** `crates/executive/src/impl/session/canonical_store.rs`

- [ ] Derive Session/Turn/Item lifecycle only from R01 transcript/control events.
- [ ] Preserve public item order while excluding raw/sensitive observations.
- [ ] Verify rebuild checksum matches the incrementally maintained view.

Run: `cargo test -p executive session_projection --all-targets`

## Task 3: Build debug, memory, Agent and metrics projections

**Create:** `crates/executive/src/impl/events/debug_projection.rs`
**Create:** `crates/executive/src/impl/events/memory_job_projection.rs`
**Create:** `crates/executive/src/impl/events/agent_tree_projection.rs`
**Create:** `crates/executive/src/impl/events/metrics_projection.rs`

- [ ] Debug projection stores causal graph edges and redacted summaries, not hidden reasoning.
- [ ] Memory projection feeds M05 eligible source-event watermarks.
- [ ] Agent projection rebuilds parent/child/status edges used by G10 recovery.
- [ ] Metrics projection calculates latency, queue pressure, tool and token counters from immutable timestamps/counters.

Run: `cargo test -p executive event_projection --all-targets`

## Task 4: Separate trace and remove duplicate writers

**Modify:** `crates/executive/src/service/post_turn_projection.rs`
**Modify:** `crates/executive/src/impl/memory_projection.rs`
**Modify:** `crates/agora/src/trace/mod.rs`

- [ ] Replace direct derived-table writes with R01 append plus reducer advancement.
- [ ] Keep runtime trace best-effort, sensitive and non-authoritative.
- [ ] Keep Agora audit focused on candidate/selection/broadcast facts; remove duplicated runtime trace fields.
- [ ] Report projection lag and poison events without blocking unrelated projections.

Run: `cargo test --workspace --all-targets --no-fail-fast`

## Task 5: Prove byte-stable rebuilds

**Create:** `tests/fixtures/event_spine/cross_domain_v1.jsonl`
**Create:** `tests/e2e/event_projection_replay.rs`

- [ ] Include a turn, tool observation, child Agent, memory candidate, Agora broadcast and restart.
- [ ] Rebuild every projection twice and compare canonical serialized state/checksums.
- [ ] Verify raw evidence never appears in the public transcript projection.

Run: `cargo test --test event_projection_replay`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `feat(events): rebuild deterministic projections` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] Incremental and full rebuild outputs match for every projection.
- [ ] Projection failure is isolated and resumable from its last checkpoint.
- [ ] No derived view is independently mutated by production handlers.
