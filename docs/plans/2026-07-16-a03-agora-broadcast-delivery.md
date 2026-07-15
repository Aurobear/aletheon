# A03 Agora Durable Broadcast Delivery Implementation Plan

> **For agentic workers:** Execute this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Persist every selected Agora epoch before bounded, visibility-filtered processor delivery, record every acknowledgement or terminal delivery failure, and replay the exact broadcast-response graph after restart.

**Architecture:** Fabric owns versioned broadcast, delivery, and acknowledgement contracts. Agora owns a SQLite epoch repository and a bounded asynchronous delivery hub. A coordinator opens the durable epoch before exposing selected content, delivers only eligible selected candidates, records terminal acknowledgements, closes the epoch, and only then finalizes the candidate selection.

**Tech Stack:** Rust, Tokio, async-trait, rusqlite, serde/serde_json, SHA-256.

**Requirement anchors:** `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:485-514`, `:677-689`, `:827-845`.

---

### Task 1: Versioned broadcast contracts

**Files:**
- Modify: `crates/fabric/src/types/workspace.rs`
- Modify: `crates/fabric/src/lib.rs`
- Test: `crates/fabric/tests/workspace_broadcast_contract.rs`

- [x] Write contract tests proving that a broadcast derives winner IDs and contents from selected candidates, rejects mismatched space/selection/version data, and bounds acknowledgement response IDs.
- [x] Run `cargo test -p fabric --test workspace_broadcast_contract`; observed the missing-contract failure during TDD.
- [x] Add `BroadcastAckStatus`, `BroadcastAck`, `BroadcastDelivery`, constructors, validation, and checksum material. Keep selected candidates in the durable broadcast so visibility and provenance cannot be lost.
- [x] Re-run the focused test; PASS (3 tests).

### Task 2: Durable SQLite epoch and acknowledgement log

**Files:**
- Modify: `crates/agora/Cargo.toml`
- Create: `crates/agora/src/broadcast/store.rs`
- Create: `crates/agora/src/broadcast/mod.rs`
- Modify: `crates/agora/src/lib.rs`
- Test: `crates/agora/tests/broadcast_delivery.rs`

- [x] Write tests for sequential epochs, exact idempotence, conflicting duplicate rejection, acknowledgement referential integrity, close semantics, reopen/replay equivalence, and checksum corruption detection.
- [x] Run the focused store test during TDD; observed the missing repository failure.
- [x] Create `broadcast_epochs` and `broadcast_acks` tables keyed by `(space, epoch)` and `(space, epoch, processor)`. Persist canonical JSON plus SHA-256 checksums in one SQLite transaction and validate the full graph during replay.
- [x] Re-run focused store tests; PASS.

### Task 3: Bounded visibility-filtered delivery

**Files:**
- Modify: `crates/agora/src/broadcast/mod.rs`
- Test: `crates/agora/tests/broadcast_delivery.rs`

- [x] Write tests proving subscriber capacity rejection, bounded concurrency, per-processor timeout/failure acknowledgements, `PrivateProcess`/`AgentTree`/`Session` filtering, response-count bounds, and that unselected candidates are never delivered.
- [x] Run the focused hub test during TDD; observed the missing delivery hub failure.
- [x] Add `BroadcastProcessor`, registration metadata, `BroadcastHubConfig`, and `BroadcastHub`. Use a semaphore and timeout for every processor and persist one terminal acknowledgement for every eligible processor.
- [x] Re-run focused hub tests; PASS.

### Task 4: Durable selection-to-broadcast coordinator

**Files:**
- Modify: `crates/agora/src/broadcast/mod.rs`
- Modify: `crates/agora/src/competition/mod.rs`
- Test: `crates/agora/tests/broadcast_delivery.rs`

- [x] Write tests proving an epoch-open failure leaves selected candidates pending, successful close finalizes exactly the selected candidates, and restart replay returns the same delivery/response edges.
- [x] Run the focused coordinator test during TDD; observed the missing orchestration failure.
- [x] Add the coordinator ordering: validate selection → allocate next epoch → durable open → bounded delivery/ACK → durable close → finalize selection. Never finalize on persistence or delivery-log failure.
- [x] Re-run the complete focused test; PASS (5 tests).

### Task 5: Validation and traceability

**Files:**
- Modify: `docs/plans/2026-07-15-executable-plan-decomposition-design.md`
- Modify: `docs/plans/2026-07-16-original-plan-coverage-matrix.md`

- [x] Run `cargo fmt --all -- --check`; PASS.
- [x] Run `cargo clippy -p fabric -p agora --all-targets -- -D warnings`; PASS.
- [x] Run `cargo test -p fabric -p agora`; PASS.
- [x] Run `cargo test --workspace`; PASS. One pre-existing 40 ms verification fixture timeout was made deterministic at 250 ms and the complete workspace rerun passed.
- [x] Run `bash tests/architecture_check.sh && bash tests/architecture_path_inventory.sh && bash scripts/architecture-check.sh`; all architecture gates PASS. (`just` is not installed on this machine, so its exact constituent commands were run.)
- [x] Mark A03 done only after all commands pass and record exact test locators in the coverage matrix.

**Deletion gate:** No previous broadcast implementation exists to retain. Future callers must not construct a global context directly from `SelectionResult`; the only supported exposure path is a durably opened `WorkspaceBroadcast` filtered by `BroadcastHub`.
