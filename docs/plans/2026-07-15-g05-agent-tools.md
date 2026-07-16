# G05 Thin Agent Control Tools Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Replace execution-owning `AgentTool` with bounded `agent_spawn`, `agent_wait`, `agent_send`, `agent_cancel`, and `agent_list` clients of `AgentControlPort`.

**Architecture:** Corpus owns schemas and presentation only. Executive supplies an `Arc<dyn AgentControlPort>` adapter through bootstrap; caller root identity is trusted execution context, never model input. The legacy `agent` tool remains temporarily as `spawn + wait` and is removed after snapshot parity.

**Tech Stack:** Rust, async-trait, Corpus tools, Fabric AgentControlPort

**Prerequisites:** G04.

**Source requirements:** `docs/plans/2026-07-15-subagent-unified-harness-plan.md:551-584`.

---

## Current-code anchors

- Corpus `AgentTool` owns `ExecuteSubAgentFn` at `crates/corpus/src/tools/tools/agent_tool.rs:29-50`.
- Fabric already defines all request/result types and control methods at `crates/fabric/src/types/agent_control.rs:81-304`.
- Bootstrap constructs the execution closure at `crates/executive/src/impl/daemon/bootstrap/runtime.rs:47`.

## Invariants and non-goals

- Caller/root identity always comes from trusted execution context.
- Corpus remains a thin client and owns no Agent lifecycle, mailbox or Kernel state.
- Legacy `agent` compatibility is bounded and deleted after explicit migration evidence.

## Key contracts

```rust
pub struct AgentToolContext { pub caller_root_agent_id: AgentId, pub parent_agent_id: AgentId, pub parent_process_id: ProcessId }
pub struct AgentControlTools { control: Arc<dyn AgentControlPort> }
```

### Task 1: Add one context-bound Corpus adapter

**Files:**
- Create: `crates/corpus/src/tools/tools/agent_control.rs`
- Modify: `crates/corpus/src/tools/tools/mod.rs`
- Create: `crates/corpus/tests/agent_control_tools.rs`

- [ ] Add schema tests for five exact tool names, bounded inputs and structured outputs.
- [ ] Define `AgentToolContext { caller_root_agent_id, parent_agent_id, parent_process_id }` and `AgentControlTools::new(Arc<dyn AgentControlPort>)`.
- [ ] Parse model inputs into Fabric requests, but obtain all caller/parent IDs from trusted context.
- [ ] Serialize `AgentHandle`/`AgentSnapshot` as JSON values; never collapse status into free-form prose.
- [ ] Run `cargo test -p corpus --test agent_control_tools`; expect PASS after adapter implementation.
- [ ] Commit with subject `feat(corpus): add bounded agent control tools`.

### Task 2: Implement spawn, wait and list semantics

**Files:**
- Modify: `crates/corpus/src/tools/tools/agent_control.rs`
- Test: `crates/corpus/tests/agent_control_tools.rs`

- [ ] `agent_spawn` accepts profile, runtime, task, context mode, tools and budget; it always injects trusted root/parent IDs.
- [ ] `agent_wait` requires `timeout_ms > 0`, returns terminal or current snapshot, and maps typed errors without provider text.
- [ ] `agent_list` enforces `1..=MAX_LIST_ITEMS` and root scope.
- [ ] Test forged root fields are ignored/rejected and repeated wait/list are idempotent.
- [ ] Run `cargo test -p corpus --test agent_control_tools spawn_wait_list`; expect typed root-scoped results.
- [ ] Commit with subject `feat(corpus): expose spawn wait and list`.

### Task 3: Implement send and cancel semantics

**Files:**
- Modify: `crates/corpus/src/tools/tools/agent_control.rs`
- Test: `crates/corpus/tests/agent_control_tools.rs`

- [ ] `agent_send` validates message bytes, supports explicit `start_turn`, and returns the persisted sequence.
- [ ] `agent_cancel` requires only Agent ID, injects caller root, and returns the durable snapshot.
- [ ] Test unknown, cross-root and terminal Agent behavior; no tool may access mailbox or Kernel directly.
- [ ] Run `cargo test -p corpus --test agent_control_tools send_cancel`; expect cross-root and terminal writes to fail closed.
- [ ] Commit with subject `feat(corpus): expose agent send and cancel`.

### Task 4: Convert and delete legacy AgentTool execution ownership

**Files:**
- Modify: `crates/corpus/src/tools/tools/agent_tool.rs`
- Modify: `crates/executive/src/impl/daemon/bootstrap/runtime.rs`
- Modify: `scripts/architecture-check.sh`

- [ ] Reimplement `agent` as a compatibility call to `spawn` followed by a configured bounded `wait`.
- [ ] Register all five tools from one adapter and expose only policy-approved names.
- [ ] Delete `ExecuteSubAgentFn`, its constructor parameter and bootstrap closure.
- [ ] Add a gate rejecting `ExecuteSubAgentFn` and `SubAgentSpawner` imports in Corpus production code.
- [ ] Run `cargo test -p corpus --test agent_control_tools && cargo test -p executive agent_control --all-targets`; expect PASS.
- [ ] Commit with subject `refactor(corpus): make agent tools control clients`.

## Final verification

Run `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`; expect the architecture gate and complete workspace suite to pass before the final stage commit.

## Completion evidence

- [ ] Five explicit tools return typed identities and status.
- [ ] Caller identity is trusted context, not model-controlled input.
- [ ] Corpus contains no Agent execution loop, Kernel dependency or spawner dependency.
- [ ] Legacy `agent` has a documented deletion gate.
