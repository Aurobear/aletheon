# M05 Leased Memory Consolidation Implementation Plan

> **Status:** Partial — production does not yet enqueue the extraction feed

**Goal:** Replace the duplicate in-memory extraction paths with one restart-safe, leased two-stage consolidation pipeline.

**Architecture:** Mnemosyne persists extraction jobs, candidates, scope leases and decisions in SQLite; one Executive-supervised worker advances the durable state machine.

**Tech Stack:** Rust, Tokio, SQLite/rusqlite, canonical Session events, Mnemosyne records

**Source requirements:** `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md:453-502`.

**Prerequisite:** M04.

## Current-code anchors

- Two pipeline implementations coexist in `crates/mnemosyne/src/impl/pipeline/mod.rs` and `crates/mnemosyne/src/impl/pipeline/memory_pipeline.rs:81-176`.
- `StateDatabase` is explicitly process-memory state at `crates/mnemosyne/src/impl/pipeline/state_db.rs:35-48`.
- `DefaultMemoryService::consolidate` is a no-op at `crates/mnemosyne/src/service.rs:328-335`.
- Canonical record identity, authority and provenance are defined at `crates/mnemosyne/src/model/record.rs:8-203`.

## Invariants and non-goals

- No raw conversation is persisted as an approved fact.
- No in-memory claim is authoritative.
- The memory worker cannot recursively delegate another consolidation worker.

## Key contracts

```rust
pub enum ExtractionStatus { Pending, Leased, Succeeded, SucceededNoOutput, RetryableFailure, PermanentFailure }
pub struct MemoryCandidate { pub kind: MemoryKind, pub claim: String, pub source_event_ids: Vec<String>, pub confidence: f64, pub proposed_scope: MemoryScope }
pub trait ConsolidationRepository { fn claim_extraction(&self, now_ms: u64, lease_ms: u64) -> anyhow::Result<Option<LeasedExtraction>>; fn complete(&self, result: ExtractionCompletion) -> anyhow::Result<()>; }
```

## Task 1: Specify durable job state transitions

**Create:** `crates/mnemosyne/tests/consolidation_jobs.rs`

- [ ] Test pending-to-leased-to-succeeded, succeeded-no-output and retryable/permanent failure transitions.
- [ ] Test lease expiry, competing claimers, idempotent completion and restart recovery using a file-backed SQLite fixture.
- [ ] Test that ephemeral, active and memory-worker sessions are ineligible.

Run: `cargo test -p mnemosyne --test consolidation_jobs`

Expected before implementation: compilation fails on missing repository types.

## Task 2: Add SQLite job and candidate repositories

**Create:** `crates/mnemosyne/src/consolidation/mod.rs`
**Create:** `crates/mnemosyne/src/consolidation/repository.rs`
**Create:** `crates/mnemosyne/src/consolidation/migrations.rs`
**Modify:** `crates/mnemosyne/src/lib.rs`

- [ ] Define extraction jobs, leases, attempts, watermarks and structured `MemoryCandidate` rows.
- [ ] Store source event IDs, redaction version, content hash, proposed scope/validity and confidence.
- [ ] Use transactional compare-and-set claims and unique idempotency keys.
- [ ] Preserve exact consumed candidate snapshots for replay and audit.

Run: `cargo test -p mnemosyne --test consolidation_jobs`

Expected: repository state-machine tests pass across reopen.

## Task 3: Implement bounded extraction

**Create:** `crates/mnemosyne/src/consolidation/extractor.rs`
**Create:** `crates/mnemosyne/tests/consolidation_extraction.rs`

- [ ] Read bounded canonical Session/Goal items and retain their event IDs.
- [ ] Redact secrets before inference and again before persistence.
- [ ] Validate structured candidates; never persist arbitrary model prose as a fact.
- [ ] Record `succeeded_no_output` when no valid candidate remains.

Run: `cargo test -p mnemosyne --test consolidation_extraction`

## Task 4: Implement deterministic scoped consolidation

**Create:** `crates/mnemosyne/src/consolidation/consolidator.rs`
**Create:** `crates/mnemosyne/tests/scoped_consolidation.rs`
**Modify:** `crates/mnemosyne/src/service.rs`

- [ ] Acquire one lease per target scope and load a bounded candidate batch.
- [ ] Produce deterministic insert, merge, reject or supersede decisions.
- [ ] Require explicit approval evidence before Core or Dasein-adjacent writes.
- [ ] Make `MemoryService::consolidate` invoke this canonical path.
- [ ] Prevent a consolidation worker from recursively scheduling itself.

Run: `cargo test -p mnemosyne --test scoped_consolidation`

## Task 5: Supervise one worker and remove duplicate state

**Create:** `crates/executive/src/service/memory_consolidation_worker.rs`
**Modify:** `crates/executive/src/impl/daemon/bootstrap/runtime.rs`
**Modify:** `crates/mnemosyne/src/impl/pipeline/mod.rs`
**Delete:** `crates/mnemosyne/src/impl/pipeline/state_db.rs`

- [ ] Start one bounded Executive-supervised worker with cancellation and backoff.
- [ ] Resume expired jobs after daemon restart.
- [ ] Remove the in-memory state database and select one canonical pipeline implementation.

Run: `cargo test -p mnemosyne --all-targets && cargo test -p executive --all-targets`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `feat(mnemosyne): persist leased consolidation` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] Restart and competing-claimer tests pass.
- [ ] Every durable write links to candidates, source events and a consumed watermark.
- [ ] No second extraction/consolidation implementation remains in production exports.
