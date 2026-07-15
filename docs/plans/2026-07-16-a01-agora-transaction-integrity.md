# Agora Transaction Integrity Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every canonical Agora commit version-checked, permit-bound, ownership-safe and durable before it becomes visible.

**Architecture:** Fabric owns a serializable `WorkspaceCommitPermit` bound to one space, proposal, author, operation hash, base version and deadline. Agora owns one independently locked slot per space plus a direct proposal index; a per-space commit gate serializes prepare → durable append → apply while the workspace state lock is released during I/O. Legacy `AgoraOps` calls remain only as a compatibility surface, while the Executive evidence path uses the bound-permit method.

**Tech Stack:** Rust, Tokio synchronization, serde/serde_json, SHA-256, existing Agora persistence port and deterministic Clock

**Prerequisites:** S02 is complete; architecture fitness baseline is green.

**Source requirements:**
- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:529-542`
- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:677-689`
- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:806-825`
- `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:902-908`

---

## Current-code anchors

- Boolean-only permits cannot bind a decision to a transaction: `crates/fabric/src/include/agora.rs:124-140`.
- `Workspace::commit` removes a proposal without rechecking its base version: `crates/agora/src/workspace/mod.rs:122-155`.
- claim/release silently ignore ownership conflicts: `crates/agora/src/workspace/mod.rs:227-233`.
- Registry holds the global session-map lock across persistence and mutates memory first: `crates/agora/src/ops/mod.rs:171-190`, `:252-271`.
- proposal lookup scans every workspace: `crates/agora/src/ops/mod.rs:256-260`.
- the production evidence path uses permit-free compatibility commit: `crates/executive/src/service/turn_pipeline.rs:544-558`.

## Invariants

```text
permit(space, proposal, author, operation_hash, base_version, expiry)
                              |
                              v
per-space commit gate -> prepare/revalidate -> durable append -> apply/visible
                                     | failure           | success
                                     v                   v
                             proposal retained      version advances once
```

1. A permit mismatch or expiry never removes the proposal or changes workspace state.
2. Two proposals created at the same version cannot both commit.
3. A claim can only be created when unowned and released by its current owner.
4. Invalid/no-op task updates are rejected before persistence.
5. Persistence failure leaves the visible version and state unchanged.
6. Persistence I/O occurs without the workspace state mutex or registry map lock.
7. Recovery accepts only a contiguous, space-correct commit sequence.

## Explicit non-goals

- Typed candidates, salience and selection belong to A02.
- Broadcast epochs and acknowledgements belong to A03.
- Scratchpad visibility contracts land with typed workspace content in A02.
- SQLite production storage is selected at composition-root migration; A01 strengthens the durability port and its ordering contract.

## File map

- Modify: `crates/fabric/src/include/agora.rs`
- Modify: `crates/agora/src/workspace/mod.rs`
- Modify: `crates/agora/src/ops/mod.rs`
- Modify: `crates/agora/src/persistence/mod.rs`
- Create: `crates/agora/tests/transaction_integrity.rs`
- Modify: `crates/executive/src/service/turn_pipeline.rs`
- Modify: `docs/plans/2026-07-15-executable-plan-decomposition-design.md`
- Modify: `docs/plans/2026-07-16-original-plan-coverage-matrix.md`

### Task 1: Replace the boolean permit contract

- [x] Add `WorkspaceCommitPermit` with `permit_id`, `space`, `proposal_id`, `process`, `operation_hash`, `expected_version`, and `expires_at_ms`.
- [x] Add canonical SHA-256 `AgoraOperation::operation_hash()` and `WorkspaceCommitPermit::validate_for(...)`.
- [x] Reject wrong space, proposal, process, hash, version and deadline independently.
- [x] Change `AgoraService::commit` and add `AgoraOps::commit_with_permit` to accept the bound permit.

Run: `cargo test -p fabric include::agora::tests::workspace_permit_`

Expected: round-trip and every mismatch/expiry case pass.

### Task 2: Split workspace prepare from apply

- [x] Add `Workspace::prepare_commit` that retains the proposal and rechecks expiry, base version, space and operation semantics.
- [x] Add `Workspace::apply_prepared_commit` that verifies the prepared version and applies exactly once.
- [x] Reject an occupied claim, a non-owner release, missing task IDs, unknown task statuses and no-op task status updates.
- [x] Keep `Workspace::commit` only as a deprecated in-memory compatibility wrapper over prepare/apply.

Run: `cargo test -p agora workspace::tests::transaction_`

Expected: stale competing commit, claim ownership and task validation tests pass without mutation on rejection.

### Task 3: Introduce per-space slots and direct proposal indexing

- [x] Replace the global `Mutex<HashMap<String, Workspace>>` with a short-held registry lock containing `Arc<SpaceSlot>` values.
- [x] Give each `SpaceSlot` a workspace mutex and a distinct commit gate.
- [x] Maintain `proposal_id -> space` on propose/reject/successful commit.
- [x] Remove the linear `iter_mut().find(...)` proposal search.

Run: `cargo test -p agora ops::tests::per_space_`

Expected: independent spaces progress concurrently and proposal lookup never scans the registry.

### Task 4: Make durability precede visibility

- [x] Under the per-space gate, prepare while holding the workspace lock, release it, append through `AgoraPersistence`, then reacquire and apply.
- [x] Retain the proposal and visible version when append fails.
- [x] Strengthen recovery to reject wrong-space, duplicate-version or non-contiguous commit logs.
- [x] Add a blocking/failing persistence fixture proving state locks are released during I/O and failure is non-visible.

Run: `cargo test -p agora --test transaction_integrity durability_ recovery_`

Expected: failed append leaves version zero; valid retry commits once; corrupt recovery fails closed.

### Task 5: Move the production evidence path to bound permits

- [x] Derive the permit from the accepted proposal and the injected Executive clock.
- [x] Call `AgoraOps::commit_with_permit`; do not call the permit-free `commit` in production code.
- [x] Add a static assertion test rejecting production `.commit(session, proposal)` bypasses.

Run: `cargo test -p executive agora_bound_permit`

Expected: evidence enters Agora through the bound permit path and bypass scan is empty.

### Task 6: Verify and commit

```bash
cargo fmt --all -- --check
cargo clippy -p fabric -p agora -p executive --all-targets -- -D warnings
cargo test -p fabric
cargo test -p agora
cargo test -p executive agora_
cargo test --workspace
bash tests/architecture_check.sh
bash scripts/architecture-check.sh
```

Expected: all commands pass; architecture baseline has no additions.

Commit subject: `fix(agora): bind commits to durable transactions`

## Compatibility deletion gate

- Remove `CommitPermit` immediately; it has no safe semantics.
- Keep `AgoraOps::commit(session, proposal_id)` only for external compatibility tests until X02 exposes `AgoraService` as the sole Executive port.
- Static checks forbid permit-free commit calls under `crates/executive/src` after A01.
- Delete deprecated direct `publish`, `update`, `trace` and permit-free commit when F01/X02 migrate remaining consumers.

## Completion evidence

- [x] every permit field is validated;
- [x] stale same-base proposals cannot both commit;
- [x] claim ownership and semantic task validation fail closed;
- [x] persistence failure is never visible as a committed workspace state;
- [x] persistence runs outside workspace and registry locks;
- [x] recovery is contiguous and byte-equivalent;
- [x] production evidence uses a bound permit;
- [x] workspace and architecture checks pass.
