# K01 Opaque Kernel Runtime Contracts Implementation Plan

> **For agentic workers:** Execute this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish one opaque Kernel lifecycle handle with exact Process and Operation transitions and fail-closed parent/owner validation before later authority migration.

**Architecture:** Fabric keeps domain-neutral lifecycle states and their complete transition matrices. Kernel tables enforce those matrices even for compatibility callers, while a new `KernelRuntime` privately composes Process, Operation, Space and Clock state and validates cross-table ownership. K02 will migrate callers and delete direct table/service-locator access; K01 deliberately preserves compatibility constructors behind an explicit deletion gate.

**Tech Stack:** Rust, Tokio, async-trait, Fabric lifecycle contracts, Kernel runtime tables, property-style matrix tests.

**Requirement anchors:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:249-283`, `:1008-1032`.

---

### Task 1: Exact lifecycle matrices

**Files:**
- Modify: `crates/fabric/src/types/process.rs`
- Modify: `crates/fabric/src/types/operation.rs`
- Test: `crates/fabric/tests/kernel_lifecycle_contract.rs`

- [x] Enumerate every Process and Operation state pair and assert only the documented edges pass.
- [x] Run the focused contract test during TDD; observed missing Operation transition validation.
- [x] Add total `can_transition_to` matrices and terminal predicates. No table may infer legality from “not terminal”.
- [x] Re-run the focused contract test; PASS (2 tests).

### Task 2: Enforce exact tables and parent integrity

**Files:**
- Modify: `crates/kernel/src/process/table.rs`
- Modify: `crates/kernel/src/operation/table.rs`
- Test: `crates/kernel/tests/lifecycle_integrity.rs`

- [x] Test orphan/terminal Process parents, orphan/cross-owner/terminal Operation parents, every illegal transition, and exact Completed/Cancelled/Failed terminal paths.
- [x] Run the focused lifecycle test during TDD; observed orphan and broad non-terminal behavior.
- [x] Validate Process parent existence before forking Space. Validate Operation parent existence, owner equality and liveness in the same records critical section used for insertion. Route start/success/failure/cancel through the exact matrix.
- [x] Re-run focused tests; PASS (3 tests).

### Task 3: Opaque cross-table KernelRuntime

**Files:**
- Create: `crates/kernel/src/runtime.rs`
- Modify: `crates/kernel/src/lib.rs`
- Test: `crates/kernel/tests/kernel_runtime.rs`

- [x] Test that runtime submission rejects unknown or terminal owners, validates parent ownership, exposes snapshots/results rather than tables, and preserves deterministic Space cleanup.
- [x] Run the focused runtime test during TDD; observed the missing runtime handle.
- [x] Add `KernelRuntime` with private Clock, Space, Process and Operation components and narrow lifecycle methods. Cross-table checks happen before mutation; snapshots are the only state views.
- [x] Re-run focused tests; PASS (3 tests).

### Task 4: Validation and traceability

**Files:**
- Modify: `docs/plans/2026-07-15-executable-plan-decomposition-design.md`
- Modify: `docs/plans/2026-07-16-original-plan-coverage-matrix.md`

- [x] Run Fabric/Kernel format, clippy and tests; PASS.
- [x] Run `cargo test --workspace` and all three architecture scripts; PASS. One unrelated Corpus script fixture failed once under the first full parallel run; its exact rerun and the complete workspace rerun passed.
- [x] Mark K01 done with exact test locators. K02 remains mapped because production consumers still use `ServicePorts` and direct tables.

**Compatibility deletion gate:** `ProcessTable`, `OperationTable`, `ServicePorts`, and their compatibility constructors remain temporarily public because Executive still imports them. K02 must migrate every production import, make tables crate-private, remove Agora from Kernel composition, and delete the compatibility surface before claiming sole Kernel authority.
