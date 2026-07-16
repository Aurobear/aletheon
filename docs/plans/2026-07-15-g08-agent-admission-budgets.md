# G08 Multi-Agent Admission and Hierarchical Budget Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Bound Agent tree size, concurrency, depth, rollout usage, cost and storage before any child resources are created.

**Architecture:** A root-scoped `AgentAdmissionController` reserves tree and execution capacity before G03 spawn. Kernel hierarchical budget reservations remain authoritative for tokens/cost/tool use; Executive adds Agent topology, role and storage policy, represented by an RAII lease transferred to the runtime task.

**Tech Stack:** Rust, Kernel hierarchical budgets, Tokio semaphore, typed config

**Prerequisites:** G07 and K02.

**Source requirements:** `docs/plans/2026-07-15-subagent-unified-harness-plan.md:642-662`.

---

## Current-code anchors

- `AgentBudget` already validates token/tool/time/cost/depth fields at `crates/fabric/src/types/agent_control.rs:49-79`.
- Kernel hierarchical-budget behavior is exercised from `crates/kernel/tests/hierarchical_budget.rs:1-7`.
- G03 admission hook belongs before current spawner resource creation at `crates/executive/src/core/sub_agent.rs:248-330`.
- Pi runtime constructs a worktree manager with a disk budget at `crates/executive/src/impl/runtime/pi.rs:104-135`.

## Invariants and non-goals

- Admission reserves topology, runtime and storage capacity before child resource creation.
- Every reservation settles or revokes exactly once.
- Internal memory/consolidation roles cannot recursively delegate.

## Key contracts

```rust
pub trait AgentAdmissionController: Send + Sync { fn reserve(&self, request: AgentAdmissionRequest) -> Result<AgentAdmissionLease, AgentControlError>; }
pub struct AgentAdmissionLease { /* root/topology/kernel/storage reservations; settled exactly once */ }
```

### Task 1: Add typed tree admission configuration

**Files:**
- Modify: `crates/cognit/src/config/mod.rs`
- Modify: `config/default.toml`
- Create: `crates/executive/src/service/agent_control/admission.rs`
- Create: `crates/executive/tests/agent_admission.rs`

- [ ] Add nonzero bounds for `max_agents_per_root`, `max_running_agents`, `max_depth`, `max_queued_per_root`, and sibling fairness quantum.
- [ ] Validate production config rejects zero, overflow and a child budget larger than its root allowance.
- [ ] Define `AgentAdmissionLease` with explicit `settle(usage)` and `revoke()`; Drop performs idempotent revoke only as a safety net.
- [ ] Run `cargo test -p executive --test agent_admission config`; expect all configuration boundary cases to pass.
- [ ] Commit with subject `feat(agent): configure bounded admission`.

### Task 2: Reserve topology and Kernel budget atomically

**Files:**
- Modify: `crates/executive/src/service/agent_control/admission.rs`
- Modify: `crates/executive/src/service/agent_control/mod.rs`
- Test: `crates/executive/tests/agent_admission.rs`

- [ ] Under one root lock, validate parent/root/depth, reserve queued/resident capacity, then reserve a child Kernel budget under the parent operation.
- [ ] Roll back all earlier reservations if any later check fails.
- [ ] Transfer the lease to the runtime task; queued/running/resident-idle counters transition explicitly.
- [ ] Add concurrent sibling tests proving no oversubscription and fair eventual admission.
- [ ] Run `cargo test -p executive --test agent_admission reservation`; expect no oversubscription under concurrent starts.
- [ ] Commit with subject `feat(agent): reserve hierarchical capacity`.

### Task 3: Enforce internal-role and storage policy

**Files:**
- Modify: `crates/executive/src/service/agent_control/admission.rs`
- Modify: `crates/executive/src/impl/runtime/pi.rs`
- Test: `crates/executive/tests/agent_admission.rs`

- [ ] Mark memory workers and internal consolidators non-delegating regardless of model request.
- [ ] Reserve worktree bytes/items before Pi runtime launch and bind release to verified terminal cleanup.
- [ ] Reject a child whose allowed tools or runtime require unavailable policy capacity.
- [ ] Test recursive memory-worker delegation and disk quota fail before Kernel process creation.
- [ ] Run `cargo test -p executive --test agent_admission policy`; expect denial before any child runtime resource is created.
- [ ] Commit with subject `feat(agent): enforce role and storage admission`.

### Task 4: Settle usage and expose bounded metrics

**Files:**
- Modify: `crates/executive/src/service/agent_control/execution.rs`
- Modify: `crates/executive/src/service/agent_control/admission.rs`
- Test: `crates/executive/tests/agent_admission.rs`

- [ ] Settle actual usage exactly once through child and ancestor budgets; revoke on cancellation/start failure.
- [ ] Expose counts and remaining budget, never prompt or output content.
- [ ] Run `cargo test -p executive --test agent_admission && cargo test -p aletheon-kernel --test hierarchical_budget`; expect PASS.
- [ ] Commit with subject `feat(agent): settle tree usage`.

## Final verification

Run `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`; expect the architecture gate and complete workspace suite to pass before the final stage commit.

## Completion evidence

- [ ] Capacity is reserved before Process/Operation/Space/mailbox creation.
- [ ] Tree, running, queued, depth, cost, token, tool and storage bounds are enforced.
- [ ] Concurrent failures release every reservation exactly once.
