# M07 Retention and Forgetting Implementation Plan

**Goal:** Replace no-op forgetting with scoped tombstones, explicit retention policy and auditable physical compaction.

**Architecture:** Logical deletion writes durable tombstones and remote-pending receipts first; a separate leased compactor physically removes eligible payloads only after policy gates.

**Tech Stack:** Rust, SQLite/rusqlite, Mnemosyne service facade, Executive admin use cases

**Source requirements:** `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md:531-549`.

**Prerequisite:** M06.

## Current-code anchors

- `ForgetPolicy` contains only `scope` and `reason` at `crates/mnemosyne/src/service.rs:178-182`.
- `DefaultMemoryService::forget` currently returns success without mutation at `crates/mnemosyne/src/service.rs:337-340`.
- Canonical records already include `MemoryStatus::Tombstoned` at `crates/mnemosyne/src/model/record.rs:27-34`.
- Existing `CompactionManager` in `crates/mnemosyne/src/impl/compaction.rs:6` compacts conversations, not retained memory records.

## Invariants and non-goals

- Ordinary Agent tools cannot delete Principal, Global or Core memory.
- Physical deletion is never the first forgetting operation.
- Conversation compaction is not reused as retention compaction.

## Key contracts

```rust
pub struct ForgetPolicy { pub request_id: String, pub selector: ForgetSelector, pub requester: PrincipalId, pub reason: String, pub authority: ForgetAuthority }
pub struct ForgetReceipt { pub tombstoned: Vec<MemoryRecordId>, pub already_tombstoned: Vec<MemoryRecordId>, pub denied: Vec<MemoryRecordId>, pub remote_pending: Vec<MemoryRecordId> }
```

## Task 1: Define deletion authority and receipts

**Modify:** `crates/mnemosyne/src/service.rs`
**Create:** `crates/mnemosyne/tests/forgetting_contract.rs`

- [ ] Extend `ForgetPolicy` with exact record IDs or a bounded selector, requester, authority proof and request ID.
- [ ] Define a receipt listing newly tombstoned, already tombstoned, denied and remote-pending records.
- [ ] Reject unbounded selectors and Principal/Global/Core deletion without elevated policy evidence.
- [ ] Prove repeated requests with the same ID are idempotent.

Run: `cargo test -p mnemosyne --test forgetting_contract`

## Task 2: Persist tombstones transactionally

**Create:** `crates/mnemosyne/src/retention/mod.rs`
**Create:** `crates/mnemosyne/src/retention/repository.rs`
**Modify:** `crates/mnemosyne/src/service.rs`

- [ ] Tombstone selected records without erasing provenance or prior version links.
- [ ] Persist requester, reason, policy decision, request time and external projection state.
- [ ] Exclude tombstoned records from normal recall while allowing privileged audit recall.
- [ ] Enqueue M06 remote tombstone reconciliation in the same local transaction boundary.

Run: `cargo test -p mnemosyne --test forgetting_contract --test unified_memory_contract`

## Task 3: Add retention-based physical compaction

**Create:** `crates/mnemosyne/src/retention/compactor.rs`
**Create:** `crates/mnemosyne/tests/retention_compaction.rs`

- [ ] Require tombstone age, completed backup/checkpoint and settled external projection policy.
- [ ] Compact in bounded batches with a durable watermark and resumable lease.
- [ ] Retain an immutable deletion receipt after payload removal.
- [ ] Keep this service distinct from conversation `CompactionManager`.

Run: `cargo test -p mnemosyne --test retention_compaction`

## Task 4: Expose governed administration

**Modify:** `crates/executive/src/service/admin_service.rs`
**Modify:** `crates/executive/src/service/request_use_cases.rs`
**Create:** `crates/executive/tests/memory_forget_admin.rs`

- [ ] Route preview, tombstone and compaction requests through an authenticated admin use case.
- [ ] Require a dry-run preview before elevated-scope execution.
- [ ] Return the durable receipt without exposing backend handles.

Run: `cargo test -p executive --test memory_forget_admin`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `feat(mnemosyne): enforce scoped retention and forgetting` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] Forgetting changes recall results and remains replayable after restart.
- [ ] Elevated scopes cannot be deleted through ordinary Agent tools.
- [ ] Physical removal occurs only after every configured retention gate passes.
