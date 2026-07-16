# G07 Live Agent Mailbox Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Connect each live Agent session to bounded, priority-aware, restart-safe control messaging.

**Architecture:** Reuse the Kernel-owned `InProcessMailboxService` and `EnvelopeV2`. `AgentControlService` persists required message metadata before delivery, while a per-run receiver multiplexes normal input and high-priority cancel/interrupt signals into Native Cognit sessions.

**Tech Stack:** Rust, Tokio, EnvelopeV2, Kernel mailbox, SQLite

**Prerequisites:** G06.

**Source requirements:** `docs/plans/2026-07-15-subagent-unified-harness-plan.md:621-640`.

---

## Current-code anchors

- Kernel exposes the one mailbox service at `crates/kernel/src/runtime.rs:189`.
- `EnvelopeV2` mailbox send/receive is defined at `crates/fabric/src/ipc/mailbox.rs:57-116`.
- Priority queues are implemented at `crates/fabric/src/ipc/mailbox.rs:172-255`.
- `SubAgentSpawner` currently registers in-process mailboxes at `crates/executive/src/core/sub_agent.rs:320` and `:438` without durable AgentControl semantics.

## Invariants and non-goals

- Durable sequence allocation occurs before live delivery.
- Root/topology policy is checked server-side for every route.
- Terminal Agents remain inspectable but reject new writes.

## Key contracts

```rust
pub enum AgentMessageKind { Input, Progress, Result, Signal, Request, Response }
pub struct AgentMessageReceipt { pub agent_id: AgentId, pub sequence: u64, pub delivery: DeliveryState }
```

### Task 1: Define schemas and durable message rows

**Files:**
- Modify: `crates/fabric/src/types/agent_control.rs`
- Create: `crates/executive/src/service/agent_control/migrations/002_agent_messages.sql`
- Modify: `crates/executive/src/service/agent_control/repository.rs`
- Create: `crates/executive/tests/agent_mailbox.rs`

- [ ] Add explicit `AgentMessageKind::{Input,Progress,Result,Signal,Request,Response}` and versioned payload validation.
- [ ] Persist `(agent_id, sequence, kind, payload_ref, delivery_state, created_at_ms)` with unique per-Agent sequence.
- [ ] Test monotonic sequences, bounded bytes, restart reopen and duplicate delivery IDs.
- [ ] Run `cargo test -p executive --test agent_mailbox repository`; expect stable sequences after reopen.
- [ ] Commit with subject `feat(agent): persist mailbox messages`.

### Task 2: Connect live receive loop

**Files:**
- Create: `crates/executive/src/service/agent_control/mailbox.rs`
- Modify: `crates/executive/src/impl/runtime/native_cognit.rs`
- Test: `crates/executive/tests/agent_mailbox.rs`

- [ ] Register one mailbox under the Kernel-created target before runtime launch and transfer its receiver to the live run.
- [ ] Multiplex normal inputs into optional child turns and high-priority cancel/interrupt into the operation token.
- [ ] Persist the message before routing and mark delivered only after mailbox receipt.
- [ ] Enforce mailbox capacity and return typed overload without dropping high-priority signals.
- [ ] Run `cargo test -p executive --test agent_mailbox live_delivery`; expect durable-before-delivery ordering and priority preservation.
- [ ] Commit with subject `feat(agent): connect mailbox to live session`.

### Task 3: Enforce topology and terminal semantics

**Files:**
- Modify: `crates/executive/src/service/agent_control/mod.rs`
- Modify: `crates/executive/src/service/agent_control/mailbox.rs`
- Test: `crates/executive/tests/agent_mailbox.rs`

- [ ] Permit parent-child messaging; route sibling messages only through explicit parent policy.
- [ ] Reject unknown/cross-root targets and all post-terminal writes; retain inspect/audit reads.
- [ ] Make duplicate delivery idempotent and request/response correlation bounded by deadline.
- [ ] Test parent-child, permitted sibling, forbidden sibling, unknown, overload and terminal cases.
- [ ] Run `cargo test -p executive --test agent_mailbox topology`; expect only policy-authorized routes to succeed.
- [ ] Commit with subject `feat(agent): enforce mailbox policy`.

### Task 4: Remove the compatibility mailbox registry

**Files:**
- Modify: `crates/executive/src/core/sub_agent.rs`
- Modify: `scripts/architecture-check.sh`

- [ ] Delete `SubAgentSpawner` mailbox ownership after G03 callers migrate.
- [ ] Add a gate allowing mailbox registration only in Kernel runtime and AgentControl mailbox adapter.
- [ ] Run `cargo test -p executive --test agent_mailbox && cargo test -p fabric mailbox --lib`; expect PASS.
- [ ] Commit with subject `refactor(agent): unify mailbox ownership`.

## Final verification

Run `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`; expect the architecture gate and complete workspace suite to pass before the final stage commit.

## Completion evidence

- [ ] One Kernel mailbox registry serves every live Agent.
- [ ] Required messages survive restart and sequences are stable.
- [ ] Cancellation remains high priority under normal-message overload.
