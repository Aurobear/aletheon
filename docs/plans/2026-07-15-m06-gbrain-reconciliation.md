# M06 GBrain Reconciliation Implementation Plan

**Goal:** Keep GBrain supplemental and replaceable while making outbound replay and remote state reconciliation durable and idempotent.

**Architecture:** Mnemosyne owns reconciliation and receipts while Executive only supervises a worker and the existing transport remains the sole GBrain protocol boundary.

**Tech Stack:** Rust, Tokio, SQLite/rusqlite, GBrain supplemental transport, durable spool

**Source requirements:** `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md:504-529`.

**Prerequisite:** M05.

## Current-code anchors

- The replaceable boundary is `SupplementalMemoryTransport` at `crates/mnemosyne/src/backends/gbrain/backend.rs:67-93`.
- Supplemental recall normalization begins at `crates/mnemosyne/src/backends/gbrain/backend.rs:174-241`.
- Direct remote forget is unsupported at `crates/mnemosyne/src/backends/gbrain/backend.rs:204-205`.
- The SQLite spool already has enqueue, claim and acknowledgement operations at `crates/mnemosyne/src/backends/gbrain/spool.rs:139-315`.

## Invariants and non-goals

- GBrain remains supplemental rather than authoritative.
- Raw messages and unapproved M05 candidates never leave local storage.
- Remote content cannot grant capabilities or mutate identity/policy.

## Key contracts

```rust
pub struct RemoteMemoryReceipt { pub logical_page_id: String, pub remote_id: String, pub content_hash: String, pub schema_version: u32, pub synced_at_ms: u64 }
pub enum ReconcileOperation { Upsert(MemoryRecordId), Supersede(MemoryRecordId), Tombstone(MemoryRecordId) }
```

## Task 1: Freeze reconciliation semantics

**Create:** `crates/mnemosyne/tests/gbrain_reconciliation.rs`

- [ ] Test replaying one logical page repeatedly yields one remote identity.
- [ ] Test persisted receipt, local content hash, schema version and last-sync timestamp.
- [ ] Test conflict, supersession, tombstone projection, transient retry and permanent dead letter.
- [ ] Test remote content is supplemental data and cannot grant tool, identity or policy authority.

Run: `cargo test -p mnemosyne --test gbrain_reconciliation`

## Task 2: Extend durable spool receipts

**Modify:** `crates/mnemosyne/src/backends/gbrain/migrations.rs`
**Modify:** `crates/mnemosyne/src/backends/gbrain/spool.rs`

- [ ] Add logical page ID, record ID, operation kind, schema version and expected content hash.
- [ ] Persist remote receipt/hash/version/time before acknowledging a claimed item.
- [ ] Reject acknowledgement whose receipt addresses another claim or payload.
- [ ] Keep migration forward-only and verify reopen from the previous schema fixture.

Run: `cargo test -p mnemosyne --test gbrain_spool --test gbrain_reconciliation`

## Task 3: Add a reconciliation service

**Create:** `crates/mnemosyne/src/backends/gbrain/reconcile.rs`
**Modify:** `crates/mnemosyne/src/backends/gbrain/mod.rs`
**Modify:** `crates/mnemosyne/src/backends/gbrain/backend.rs`

- [ ] Normalize recalled hits with explicit `Supplemental` authority and untrusted-content provenance.
- [ ] Reconcile local active/superseded/tombstoned state against the last remote receipt.
- [ ] Emit supersession/tombstone pages when the transport has no delete operation.
- [ ] Never enqueue raw messages or unapproved M05 candidates.
- [ ] Surface failures through supplemental health rather than failing the root Goal.

Run: `cargo test -p mnemosyne --test gbrain_backend_contract --test gbrain_reconciliation`

## Task 4: Reduce Executive to transport supervision

**Modify:** `crates/executive/src/impl/gbrain/worker.rs`
**Modify:** `crates/executive/src/impl/gbrain/bootstrap.rs`
**Modify:** `crates/executive/src/impl/gbrain/mcp_adapter.rs`

- [ ] Make the worker claim and submit reconciliation operations owned by Mnemosyne.
- [ ] Keep protocol-specific mapping inside the `SupplementalMemoryTransport` adapter.
- [ ] Add cancellation, bounded retry and health reporting without memory-domain decisions.

Run: `cargo test -p executive gbrain --all-targets`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `feat(mnemosyne): reconcile supplemental gbrain memory` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] Crash after remote success and before local acknowledgement is recovered without duplication.
- [ ] Tombstone propagation is auditable when direct delete is unavailable.
- [ ] Executive contains no GBrain authority, merge or record-normalization policy.
