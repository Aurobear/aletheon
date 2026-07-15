# G02 Agent Control Contracts Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define one bounded, versioned Fabric control vocabulary for spawning, observing, messaging, cancelling and collecting child Agents.

**Architecture:** Reuse existing Agent/Process/Operation/Runtime/Profile identifiers and Attempt usage/evidence types. Put transport-neutral requests, snapshots and results behind `AgentControlPort`; Executive implements the port in G03 and Corpus consumes it in G05.

**Tech Stack:** Rust, serde, async-trait, existing Fabric lifecycle contracts.

**Prerequisites:** G01 baseline.

**Source requirements:** `docs/plans/2026-07-15-subagent-unified-harness-plan.md:285-372` and `:428-451`.

---

## Current-code anchors

- Canonical lifecycle IDs are exported at `crates/fabric/src/lib.rs:186-205`.
- Attempt usage/evidence contracts exist at `crates/fabric/src/types/attempt.rs:59-86`.
- Current AgentTool mirrors definitions and returns free-form text at `crates/corpus/src/tools/tools/agent_tool.rs:15-46`.
- SubAgentSpawner exposes ad-hoc spawn/wait/cancel/list operations at `crates/executive/src/core/sub_agent.rs:247-728`.

## Invariants and non-goals

- No duplicate AgentId, ProcessId, OperationId, RuntimeId or status from existing lifecycle authorities.
- Every user-controlled string/vector has a validation bound.
- Context fork defaults to selected bounded projection, never raw history.
- Wait has an explicit timeout and cross-root access carries the caller root ID.
- G02 adds unused contracts only; no production routing changes.

## File map

- Create: `crates/fabric/src/types/agent_control.rs`
- Create: `crates/fabric/tests/agent_control_contract.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`

### Task 1: Define bounded requests and context forks

- [ ] Add `AgentContextFork::{None, LastTurns { count }, SelectedProjection { items }}`.
- [ ] Add `AgentBudget`, `AgentSpawnRequest`, `AgentWaitRequest`, `AgentSendRequest` and `AgentListRequest`.
- [ ] Validate task/message/item byte sizes, item/evidence/artifact counts, positive timeout and nonzero budgets.

Run: `cargo test -p fabric --test agent_control_contract request_`

Expected: all limit boundaries and one-over-limit cases PASS.

### Task 2: Define snapshots, results and stable status

- [ ] Add `AgentRunStatus`, `AgentHandle`, `AgentSnapshot`, `AgentControlMessage`, `AgentArtifact` and `AgentResult`.
- [ ] Reuse existing IDs and `AttemptUsage`/`AttemptEvidence`.
- [ ] Add stable snake-case JSON round trips.

Run: `cargo test -p fabric --test agent_control_contract serialization_`

Expected: PASS with no duplicate ID representation.

### Task 3: Define the control port

- [ ] Add async `spawn`, `wait`, `send`, `cancel`, `inspect` and `list` methods.
- [ ] Use typed errors with stable kind plus redacted message.
- [ ] Add a mock implementation test proving object-safe `Arc<dyn AgentControlPort>` use.

Run: `cargo test -p fabric --test agent_control_contract port_`

Expected: PASS.

### Task 4: Verify and commit

```bash
cargo fmt --all -- --check
cargo clippy -p fabric --all-targets -- -D warnings
cargo test -p fabric
cargo test --workspace
bash tests/architecture_check.sh
bash scripts/architecture-check.sh
```

Commit subject: `feat(fabric): define agent control contracts`

## Compatibility deletion gate

G03 adapts SubAgentSpawner to this port. G05 deletes Corpus `AgentDefinition`/`ExecuteSubAgentFn` mirrors after all Agent tools use `AgentControlPort`.

## Completion evidence

- [ ] existing IDs are reused;
- [ ] every external field is bounded;
- [ ] serialization is stable;
- [ ] the port is object-safe;
- [ ] no production path changes;
- [ ] workspace and architecture checks pass.
