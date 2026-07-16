# M08 SubAgent Memory Isolation Implementation Plan

**Goal:** Give every child Agent private Agent/Task memory scopes and require reviewed promotion before broader visibility.

**Architecture:** Mnemosyne derives child access from trusted Agent process context and accepts broader-scope promotion only with the same durable G09 selection/review receipt.

**Tech Stack:** Rust, Mnemosyne scopes, Fabric Agent contracts, Agora selection receipts

**Source requirements:** `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md:551-575`.

**Prerequisites:** M04 and G09. This plan shares one implementation boundary and acceptance evidence with G09.

## Current-code anchors

- Canonical `MemoryScope` already has `Agent` and `Task` variants at `crates/mnemosyne/src/model/scope.rs:5-12`.
- `AgentContextFork` supports selected bounded projection items at `crates/fabric/src/types/agent_control.rs:16-46`.
- Canonical memory provenance is stored in `MemoryMetadata` and `MemoryRecord` at `crates/mnemosyne/src/model/record.rs:58-203`.
- G09 defines the Agent-side promotion boundary at `docs/plans/2026-07-15-g09-agent-memory-promotion.md:5-11`.

## Invariants and non-goals

- An Agent-provided scope string is never trusted.
- Child consolidation does not duplicate the root M05 pipeline.
- Ordinary Agent spawn does not create an independently persistent subject.

## Key contracts

```rust
pub struct AgentMemoryContext { pub process_id: ProcessId, pub agent_id: AgentId, pub task_id: TaskId, pub agent_scope: MemoryScope, pub task_scope: MemoryScope, pub parent_projection_receipt: String }
pub struct MemoryPromotionRequest { pub source_record: MemoryRecordId, pub child: AgentMemoryContext, pub root_content: ContentId, pub broadcast: BroadcastEpoch, pub selected_candidate: ContentId, pub reviewer: PrincipalId, pub review_receipt: String, pub target_scope: MemoryScope }
```

## Task 1: Freeze scope isolation tests

**Create:** `crates/mnemosyne/tests/agent_memory_isolation.rs`

- [ ] Assert a child can recall its Agent/Task ancestry but not sibling, parent-private or unrelated Session records.
- [ ] Assert child writes default to both current Agent identity and Task lineage.
- [ ] Assert parent projection obeys M04 item/byte limits and is read-only provenance-preserving data.

Run: `cargo test -p mnemosyne --test agent_memory_isolation`

## Task 2: Bind memory access to verified Agent context

**Modify:** `crates/mnemosyne/src/model/scope.rs`
**Modify:** `crates/mnemosyne/src/service.rs`
**Create:** `crates/mnemosyne/src/agent_scope.rs`

- [ ] Define `AgentMemoryContext` from verified ProcessId, AgentId, TaskId and parent lineage.
- [ ] Derive allowed recall/write scopes server-side instead of trusting tool-supplied scope strings.
- [ ] Reject sibling access, scope escalation and missing process bindings.
- [ ] Store child process/task identity in provenance on every child write.

Run: `cargo test -p mnemosyne --test agent_memory_isolation`

## Task 3: Implement reviewed promotion receipts

**Modify:** `crates/mnemosyne/src/promotion.rs`
**Create:** `crates/mnemosyne/tests/agent_memory_promotion.rs`

- [ ] Accept only G09 promotion requests containing child ProcessId/AgentId/TaskId, source record and root candidate IDs.
- [ ] Require root broadcast, selection and parent/consolidator review receipts.
- [ ] Create a new broader-scope record version while preserving the child source record and provenance chain.
- [ ] Make duplicate promotion idempotent and conflicting promotion fail closed.
- [ ] Keep child consolidation out of the root M05 extraction queue.

Run: `cargo test -p mnemosyne --test agent_memory_promotion`

## Task 4: Integrate G09 and enforce the subject boundary

**Modify:** `crates/executive/src/service/agent_control/memory.rs`
**Modify:** `crates/executive/src/service/agent_control/candidate_projection.rs`
**Modify:** `scripts/architecture-check.sh`

- [ ] Pass the bounded projection at spawn and the verified Agent context on child memory operations.
- [ ] Submit only explicitly visible child evidence to the root C01 candidate port.
- [ ] Reject persistent independent-subject creation unless a separate Dasein ledger, root Agora space and memory authority are provisioned.
- [ ] Add an architecture gate against direct child writes to Session/Principal/Global/Core scopes.

Run: `bash scripts/architecture-check.sh && cargo test -p mnemosyne --test agent_memory_isolation --test agent_memory_promotion`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `feat(mnemosyne): isolate and promote agent memory` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] Cross-agent and sibling leakage tests pass.
- [ ] Every promoted record links child identity, task, root candidate, broadcast and review evidence.
- [ ] G09 and M08 close against the same end-to-end promotion test.
