# Aletheon M5 Durable Approval and Controlled Apply Detailed Plan

> **For agentic workers:** Implement repository, routing, and apply operations as separate reviewed stages.

**Goal:** Make Goal approvals survive restart and ensure a verified Pi diff reaches the main worktree only through a trusted, one-time, explicitly approved operation.

**Architecture:** Keep `SocketApprovalGate` for synchronous live tool approvals. Add a durable `ApprovalRepository` for Goal operations, route owner decisions through Telegram/RPC, and execute an immutable-hash-bound apply operation after revalidation.

**Tech Stack:** Rust, SQLite/ObjectiveStore migrations, existing channel identity binding, SocketApprovalGate, Telegram actions, git, Tokio command runner, M4 verification artifacts.

---

## 1. Anchors

- Telegram is preferred approval surface and protected actions are enumerated at `docs/arch/agent-google/03_CHANNEL_AND_MOBILE_COMMUNICATION.md:110-133`.
- Approval commands: `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:343-369`.
- Current synchronous gate is oneshot/in-memory: `crates/corpus/src/security/socket_approval.rs:15-54`.
- Current notification pump allocates transient IDs: `crates/executive/src/service/turn_pipeline.rs:604-633`.

M5 does not replace synchronous tool approval. It adds durable Goal approval beside it.

## 2. Task 1 — Define durable approval contracts

**Files:**

- Create: `crates/fabric/src/types/approval.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`

- [ ] Define `ApprovalId`, `ApprovalCategory`, `ApprovalRisk`, `ApprovalStatus`, `ApprovalSubject`, `ApprovalSnapshot`, and `ApprovalResolution`.
- [ ] Categories include ApplyCode, SendMail, DeleteFile, ModifyCalendar, GitPush, CapabilityExpansion, DaseinModification, and BudgetExpansion.
- [ ] Snapshot binds Goal/attempt/job IDs, owner PrincipalId, immutable subject hash, summary, artifact refs, creation/expiry, status, version, and resolution metadata.
- [ ] Test serde, terminal statuses, expiry, one-time decisions, and subject-hash stability.
- [ ] Run `cargo test -p fabric -- types::approval`; expect PASS.
- [ ] Commit `feat(fabric): define durable approval contracts`.

## 3. Task 2 — Add ApprovalRepository

**Files:**

- Modify: `crates/executive/src/impl/goal/migrations.rs`
- Create: `crates/executive/src/impl/approval/mod.rs`
- Create: `crates/executive/src/impl/approval/repository.rs`
- Modify: `crates/executive/src/impl/mod.rs`

- [ ] Add `approval_requests` table with ID, Goal/attempt/job refs, owner, category, risk, subject hash, summary, artifacts JSON, expiry, status, version, resolution principal/channel/time, and unique `(category, subject_hash)` for active requests.
- [ ] Implement create/get/list_pending/resolve/expire with optimistic versions and an append-only `approval_events` table.
- [ ] Test duplicate creation, approve/reject, replay, wrong owner, wrong channel policy, expiry-to-deny, stale version, restart recovery, and transaction rollback.
- [ ] Missing/expired/delivery-failed decisions resolve to denial, never approval.
- [ ] Run `cargo test -p executive -- impl::approval`; expect PASS.
- [ ] Commit `feat(executive): persist goal approval requests`.

## 4. Task 3 — Create approval requests after verification

**Files:**

- Modify: `crates/executive/src/impl/goal/attempt_coordinator.rs`
- Modify: `crates/executive/src/impl/goal/coordinator.rs`
- Test: `crates/executive/tests/approval_goal_flow.rs`

- [ ] After required M4 checks pass, compute subject hash from base commit, diff artifact hash, verification report hash, allowed scope, and apply target.
- [ ] Create one ApplyCode approval and transition Goal to AwaitingHuman with request ID in wait reason/event.
- [ ] Duplicate coordinator calls return the existing active approval.
- [ ] Verification failure, missing artifact, or changed hash cannot create approval.
- [ ] Test restart between verification and approval creation and between approval creation and delivery.
- [ ] Run `cargo test -p executive --test approval_goal_flow`; expect PASS.
- [ ] Commit `feat(executive): request approval for verified coding jobs`.

## 5. Task 4 — Route approval notifications through durable outbox

**Files:**

- Modify: `crates/executive/src/impl/channel/router.rs`
- Modify: `crates/executive/src/impl/channel/telegram/mod.rs`
- Modify: `crates/executive/src/impl/approval/repository.rs`

- [ ] Render Goal ID, changed-file count, verification summary, risk, expiry, and actions Apply/View Diff/Request Revision/Reject.
- [ ] Action callback data contains only approval ID plus action; repository provides authoritative details.
- [ ] Persist outbox before provider send and record delivery correlation/status.
- [ ] Test send retry, duplicate Telegram callback, unknown user, expired request, forged ID, and notification after daemon restart.
- [ ] Do not include unbounded diff in Telegram; View Diff returns a bounded artifact excerpt or trusted local reference.
- [ ] Run channel/approval scoped tests; expect PASS.
- [ ] Commit `feat(executive): deliver durable Telegram approvals`.

## 6. Task 5 — Extend RPC for durable approvals

**Files:**

- Modify: `crates/executive/src/impl/daemon/handler/rpc.rs`
- Create: `crates/executive/src/impl/daemon/handler/rpc/rpc_approval.rs`
- Preserve: `approval_response` in `rpc_admin.rs` for synchronous SocketApprovalGate

- [ ] Add `approval.list`, `approval.show`, `approval.approve`, and `approval.reject` for durable requests.
- [ ] Require authenticated local principal/channel context; never infer owner from request JSON alone.
- [ ] Keep transient `approval_response` behavior unchanged.
- [ ] Test namespace separation so a durable ID cannot resolve a oneshot and vice versa.
- [ ] Run RPC tests; expect PASS.
- [ ] Commit `feat(executive): expose durable approval RPC`.

## 7. Task 6 — Implement controlled apply primitive

**Files:**

- Create: `crates/corpus/src/tools/subagent/apply.rs`
- Modify: `crates/corpus/src/tools/subagent/mod.rs`
- Test: `crates/corpus/tests/controlled_apply.rs`

- [ ] Define `ApplySpec` containing repository root, expected HEAD, diff artifact/hash, allowed paths, approval ID/subject hash, timeout, and dry-run flag.
- [ ] Recompute and compare every hash; verify approval through an injected read-only authorization interface.
- [ ] Require current HEAD equals approved base commit unless an explicit rebase/reverification workflow creates a new approval.
- [ ] Re-run `git apply --check`, path/symlink scope checks, then `git apply --index` or repository-standard apply method using argv, not shell strings.
- [ ] On failure, restore only changes introduced by this operation using a pre-operation index/worktree snapshot; never use `git reset --hard` or overwrite unrelated user changes.
- [ ] Test success, dry run, rejected/expired/replayed approval, stale HEAD, tampered diff/report, path escape, conflict, cancellation, and dirty unrelated files.
- [ ] Run `cargo test -p corpus --test controlled_apply`; expect PASS.
- [ ] Commit `feat(corpus): apply approved coding diffs safely`.

## 8. Task 7 — Coordinate one-time apply

**Files:**

- Modify: `crates/executive/src/impl/goal/coordinator.rs`
- Create: `crates/executive/src/impl/approval/apply_coordinator.rs`
- Test: `crates/executive/tests/approved_apply_flow.rs`

- [ ] Approval resolution schedules one apply operation with OperationTable and persists Running state before execution.
- [ ] Revalidate subject, HEAD, scope, verification, and approval immediately before apply.
- [ ] Success marks approval Consumed, stores apply receipt, transitions Goal Completed, and cleans managed worktree.
- [ ] Reject/request-revision transitions Goal appropriately without applying.
- [ ] Apply failure transitions Blocked/AwaitingHuman with evidence and does not reuse approval.
- [ ] Test duplicate callbacks/concurrent apply, restart before/after apply, cancellation, and receipt recovery.
- [ ] Run `cargo test -p executive --test approved_apply_flow`; expect PASS.
- [ ] Commit `feat(executive): coordinate one-time approved apply`.

## 9. Task 8 — Audit and completion summary

**Files:**

- Modify Goal event/summary integration files
- Test: `crates/executive/tests/goal_completion_summary.rs`

- [ ] Produce summary containing Goal intent, attempts, changed files, checks, approval resolution, apply receipt, risks, and final state.
- [ ] Persist summary before notification and future M8 memory ingestion.
- [ ] Redact secrets and bound excerpts.
- [ ] Test accepted, rejected, revision, apply-failed, and restart cases.
- [ ] Commit `feat(executive): record approved goal outcomes`.

## 10. M5 release audit

- [ ] Run formatting, all approval/apply/Goal/channel tests, workspace tests, and workspace build.
- [ ] Prove approval survives daemon restart and only owner Telegram/local authenticated RPC can resolve it.
- [ ] Prove callback replay and concurrent apply produce at most one apply receipt.
- [ ] Prove tampered/stale artifacts require a new verification and approval.
- [ ] Prove rejected approval leaves main worktree unchanged.
- [ ] Prove synchronous SocketApprovalGate tests still pass.
- [ ] Prove no automatic push/merge/protected-branch mutation exists.

## 11. DeepSeek batches

1. Tasks 1–2: contracts/repository.
2. Tasks 3–5: Goal/channel/RPC.
3. Tasks 6–7: apply primitive/coordinator.
4. Tasks 8–10: summary/audit.

Guardrails:

```text
Do not replace SocketApprovalGate.
Do not trust callback payload details beyond approval ID/action.
Do not apply without immutable hash revalidation.
Do not use git reset --hard or checkout to roll back.
Do not push or merge automatically.
Stop after each batch with exact evidence.
```
