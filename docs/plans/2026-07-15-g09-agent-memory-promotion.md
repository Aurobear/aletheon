# G09 Agent Memory Isolation and Promotion Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Keep child experience private to Agent/Task scopes and promote selected evidence only through root Agora selection plus parent/consolidator policy.

**Architecture:** This plan shares one implementation boundary with M08. Spawn receives a bounded approved parent `MemoryProjection`; child writes use Agent/Task scope. Promotion is a durable request binding child identity, root broadcast, candidate, selection and review receipts before Mnemosyne records any broader scope.

**Tech Stack:** Rust, Mnemosyne canonical records, Agora candidates, AgentControlService

**Prerequisites:** G08 and M04.

**Shared completion:** This plan closes M08 jointly.

**Source requirements:** `docs/plans/2026-07-15-subagent-unified-harness-plan.md:664-688`; `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md:551-575`.

---

## Current-code anchors

- Agent and Task scopes already exist at `crates/mnemosyne/src/model/scope.rs:5-51`.
- `AgentResult` currently contains output, usage, evidence and artifacts at `crates/fabric/src/types/agent_control.rs:217-239` but no promotion receipt.
- `WorkspaceContent::AgentResult` and provenance exist at `crates/fabric/src/types/workspace.rs:84-119`.

## Invariants and non-goals

- Child writes remain Agent/Task scoped until root selection and review complete.
- Promotion creates a provenance-linked version rather than rewriting the child source.
- Ordinary Agent execution never creates independent Dasein or root Agora authority.

## Key contracts

```rust
pub struct AgentMemoryContext { pub process_id: ProcessId, pub agent_id: AgentId, pub task_id: TaskId, pub agent_scope: MemoryScope, pub task_scope: MemoryScope, pub parent_projection_receipt: String }
pub struct MemoryPromotionRequest { pub source_record: MemoryRecordId, pub child: AgentMemoryContext, pub root_content: ContentId, pub broadcast: BroadcastEpoch, pub selected_candidate: ContentId, pub reviewer: PrincipalId, pub review_receipt: String, pub target_scope: MemoryScope }
pub struct MemoryPromotionReceipt { pub request_hash: String, pub resulting_record: MemoryRecordId, pub resulting_version: u64, pub decision: PromotionDecision }
```

### Task 1: Bind child memory context and writes

**Files:**
- Modify: `crates/fabric/src/types/agent_control.rs`
- Modify: `crates/executive/src/service/agent_control/context_fork.rs`
- Create: `crates/executive/src/service/agent_control/memory.rs`
- Create: `crates/executive/tests/agent_memory_isolation.rs`

- [ ] Add `AgentMemoryContext { process_id, agent_id, task_id, agent_scope, task_scope, parent_projection_receipt }` to the trusted runtime input.
- [ ] Fork only selected M04 records within byte/item limits; preserve record IDs, authority and provenance.
- [ ] Record child actions/results as canonical experiences under child Task/Agent scope.
- [ ] Test child writes are invisible to Session/Global recall without promotion.
- [ ] Run `cargo test -p executive --test agent_memory_isolation`; expect sibling and broader-scope recall to exclude child records.
- [ ] Commit with subject `feat(agent): isolate child memory`.

### Task 2: Define durable promotion request and receipt

**Files:**
- Modify: `crates/mnemosyne/src/model/record.rs`
- Create: `crates/mnemosyne/src/promotion.rs`
- Modify: `crates/mnemosyne/src/lib.rs`
- Test: `crates/mnemosyne/tests/agent_memory_promotion.rs`

- [ ] Define `MemoryPromotionRequest` containing source record, child Agent/Process, task, root broadcast/content, selected child candidate, target scope and reviewer.
- [ ] Define `MemoryPromotionReceipt` containing stable request hash, resulting record ID/version and decision.
- [ ] Reject target Core/Dasein mutation, missing selection, provenance mismatch and broader scope without policy approval.
- [ ] Make repeated identical requests idempotent and conflicting requests explicit.
- [ ] Run `cargo test -p mnemosyne --test agent_memory_promotion`; expect only receipt-complete promotion to succeed.
- [ ] Commit with subject `feat(mnemosyne): govern memory promotion`.

### Task 3: Connect Agora selection and parent review

**Files:**
- Modify: `crates/executive/src/service/agent_control/candidate_projection.rs`
- Modify: `crates/executive/src/service/agent_control/memory.rs`
- Test: `crates/executive/tests/agent_memory_promotion.rs`

- [ ] Return memory candidates in terminal Agent result without recording them broadly.
- [ ] Create a promotion request only after root C01 selection and explicit parent/consolidator decision.
- [ ] Preserve root broadcast -> child run -> candidate -> selection -> promotion receipt lineage.
- [ ] Keep rejected/unselected work available only in child audit scope.
- [ ] Run `cargo test -p executive --test agent_memory_promotion`; expect complete lineage and restart-idempotent results.
- [ ] Commit with subject `feat(agent): promote reviewed child evidence`.

### Task 4: Lock persistent-subject and duplication boundaries

**Files:**
- Modify: `scripts/architecture-check.sh`
- Modify: `crates/executive/tests/agent_memory_isolation.rs`

- [ ] Reject child direct calls to Global/Core record, Dasein transitions and root memory consolidation.
- [ ] Prove ordinary Agents have no independent Dasein ledger or root Agora authority.
- [ ] Run `cargo test -p mnemosyne --test agent_memory_promotion && cargo test -p executive --test agent_memory_isolation --test agent_memory_promotion`; expect PASS.
- [ ] Commit with subject `test(agent): lock memory isolation boundary`.

## Final verification

Run `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`; expect the architecture gate and complete workspace suite to pass before the final stage commit.

## Completion evidence

- [ ] Child recall/writes default to Agent/Task scope.
- [ ] Promotion requires root selection and review and is restart-idempotent.
- [ ] Provenance retains every source Agent, process, task, broadcast and receipt.
- [ ] M08 coverage row is closed by the same evidence.
