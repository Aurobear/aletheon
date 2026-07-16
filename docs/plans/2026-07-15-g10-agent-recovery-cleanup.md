# G10 Agent Recovery and Cleanup Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Recover durable Agent metadata after daemon restart without replaying ambiguous provider work, and clean resources only after verified terminal state.

**Architecture:** Startup recovery loads open G03 rows, reconciles them with Kernel state and runtime resumability, and persists an explicit interruption/recovery decision. Expired admission/mailbox/worktree leases are reclaimed idempotently; terminal result metadata outlives live runtime eviction under retention policy.

**Tech Stack:** Rust, Tokio, SQLite, KernelRuntime, storage quota/worktree recovery

**Prerequisites:** G09.

**Source requirements:** `docs/plans/2026-07-15-subagent-unified-harness-plan.md:690-708`.

---

## Current-code anchors

- G03 repository will own durable run identity; current `SubAgentSpawner` state is memory-only at `crates/executive/src/core/sub_agent.rs:153-176`.
- Kernel exposes process snapshots and process/operation waits at `crates/kernel/src/runtime.rs:611-644` and `crates/kernel/src/runtime.rs:750-762`.
- Worktree recovery service construction and recovery entrypoint exist at `crates/executive/src/impl/runtime/worktree_recovery.rs:65-102`.
- Production health ownership is assembled at `crates/executive/src/impl/daemon/bootstrap/request.rs:1024-1044`.

## Invariants and non-goals

- Ambiguous provider/tool work is interrupted rather than replayed.
- Resume requires an explicit runtime checkpoint and preserves Agent identity.
- Cleanup removes only resources whose terminal identity and lease are verified.

## Key contracts

```rust
pub enum RuntimeResumability { Never, Checkpointed { reference: String } }
pub enum AgentRecoveryDecision { Interrupt, Resume, Finalize, Reclaim }
```

### Task 1: Define resumability and recovery decisions

**Files:**
- Modify: `crates/fabric/src/types/agent_control.rs`
- Create: `crates/executive/src/service/agent_control/recovery.rs`
- Create: `crates/executive/tests/agent_recovery.rs`

- [ ] Add explicit `RuntimeResumability::{Never,Checkpointed}` and `AgentRecoveryDecision::{Interrupt,Resume,Finalize,Reclaim}`.
- [ ] Require runtimes to advertise resumability and checkpoint reference; native provider calls default to `Never`.
- [ ] Persist decision, daemon generation and recovery timestamp before taking action.
- [ ] Run `cargo test -p executive --test agent_recovery decision`; expect ambiguous native work to resolve to interruption.
- [ ] Commit with subject `feat(agent): define restart decisions`.

### Task 2: Reconcile open runs at startup

**Files:**
- Modify: `crates/executive/src/service/agent_control/recovery.rs`
- Modify: `crates/executive/src/impl/daemon/bootstrap/runtime.rs`
- Test: `crates/executive/tests/agent_recovery.rs`

- [ ] Load bounded open rows and parent edges before accepting new spawns.
- [ ] If Kernel/runtime work cannot be proven live and resumable, transition to `Interrupted`; never silently repeat a provider/tool call.
- [ ] Resume only checkpointed runtimes using the stored checkpoint and same Agent identity.
- [ ] Finalize rows whose Kernel operation is terminal but repository transition was interrupted.
- [ ] Test crash points before launch, during provider, after terminal Kernel state and after result persistence.
- [ ] Run `cargo test -p executive --test agent_recovery startup`; expect every open-row fixture to reach one durable decision.
- [ ] Commit with subject `feat(agent): reconcile open runs`.

### Task 3: Reclaim leases and verified resources

**Files:**
- Modify: `crates/executive/src/service/agent_control/recovery.rs`
- Modify: `crates/executive/src/impl/runtime/worktree_recovery.rs`
- Modify: `crates/executive/src/service/agent_control/admission.rs`
- Test: `crates/executive/tests/agent_cleanup.rs`

- [ ] Reclaim expired admission, mailbox and execution leases with stable idempotency keys.
- [ ] Delete worktrees only for verified terminal runs with matching lease, root and expected head; retain unsafe failures for inspection.
- [ ] Keep result/audit rows until configured retention deadline and compact in bounded batches.
- [ ] Test forged leases, dirty worktrees, repeated recovery and partial cleanup failure.
- [ ] Run `cargo test -p executive --test agent_cleanup`; expect only verified terminal resources to be reclaimed.
- [ ] Commit with subject `feat(agent): reclaim terminal resources`.

### Task 4: Expose readiness and close legacy state

**Files:**
- Modify: `crates/executive/src/service/request_use_cases.rs`
- Modify: `crates/executive/src/core/sub_agent.rs`
- Modify: `scripts/architecture-check.sh`

- [ ] Report sanitized counts for open/interrupted/recovery-failed rows and make unreconciled required state unready.
- [ ] Delete remaining production `SubAgentSpawner` run tracking after all callers use G03.
- [ ] Add gates rejecting durable Agent state outside the repository and provider replay in recovery.
- [ ] Run `cargo test -p executive --test agent_recovery --test agent_cleanup --test production_health`; expect PASS.
- [ ] Commit with subject `refactor(agent): retire legacy run tracking`.

## Final verification

Run `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`; expect the architecture gate and complete workspace suite to pass before the final stage commit.

## Completion evidence

- [ ] Restart never replays ambiguous provider/tool calls.
- [ ] Explicitly resumable runtimes retain Agent identity and checkpoint lineage.
- [ ] Leases and worktrees are reclaimed only under verified policy.
- [ ] Terminal results remain inspectable after live eviction.
