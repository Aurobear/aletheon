# G06 SubAgent Context and Agora Projection Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Give each child Agent a bounded isolated context and route its observable work to the root only as typed Agora candidates.

**Architecture:** Executive builds `AgentContextProjection` from selected Goal, constraints, memory and evidence. A child gets a private Agora namespace; progress/evidence/results remain private until `AgentCandidateProjector` submits a visibility-scoped `WorkspaceCandidate` to C01. Large content remains an artifact reference.

**Tech Stack:** Rust, Agora typed workspace, Fabric Agent contracts, Executive control service

**Prerequisites:** G05 and C01.

**Source requirements:** `docs/plans/2026-07-15-subagent-unified-harness-plan.md:586-619`; `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:867-883`.

---

## Current-code anchors

- `AgentContextFork` supports none/last-turns/selected items at `crates/fabric/src/types/agent_control.rs:16-46`.
- Typed candidates, provenance and visibility already exist at `crates/fabric/src/types/workspace.rs:84-220`.
- `WorkspaceContent::AgentResult` exists at `crates/fabric/src/types/workspace.rs:97`.
- Agora competition is bounded at `crates/agora/src/competition/mod.rs:151-175`.

## Invariants and non-goals

- Child context is bounded, labelled and contains no raw hidden reasoning.
- Child candidates are private until C01 selection grants broader visibility.
- Large content crosses the boundary only through content-addressed artifact references.

## Key contracts

```rust
pub struct AgentContextProjection { pub goal: Option<String>, pub constraints: Vec<String>, pub items: Vec<AgentContextItem>, pub broadcast_refs: Vec<ContentId>, pub omitted_count: usize }
pub trait AgentCandidateProjector { fn project(&self, event: AgentRuntimeEvent) -> Result<Vec<WorkspaceCandidate>, AgentControlError>; }
```

### Task 1: Define bounded context projection

**Files:**
- Modify: `crates/executive/src/service/agent_control/context_fork.rs`
- Create: `crates/executive/tests/agent_context_projection.rs`

- [ ] Implement the G04 `AgentContextProjection { goal, constraints, items, broadcast_refs, omitted_count }` builder with per-kind item and byte limits.
- [ ] Implement `None`, `LastTurns { count }`, and `SelectedProjection`; label all restored content as untrusted data.
- [ ] Reject hidden reasoning and raw tool-output blocks; retain only content-addressed artifact references.
- [ ] Add deterministic ordering and UTF-8-safe truncation tests.
- [ ] Run `cargo test -p executive --test agent_context_projection`; expect PASS.
- [ ] Commit with subject `feat(agent): build bounded child context`.

### Task 2: Allocate and persist child workspace identity

**Files:**
- Modify: `crates/executive/src/service/agent_control/mod.rs`
- Modify: `crates/executive/src/service/agent_control/repository.rs`
- Test: `crates/executive/tests/agent_context_projection.rs`

- [ ] Derive one child `AgoraSpaceId` from durable Agent ID and store it with the run.
- [ ] Bind permitted broadcast epochs/content IDs to the spawn request and projection receipt.
- [ ] Subscribe only to broadcasts allowed by task, Kernel space and `VisibilityScope`.
- [ ] Test sibling and cross-root spaces cannot observe private candidates.
- [ ] Run `cargo test -p executive --test agent_context_projection workspace`; expect sibling and cross-root visibility cases to pass.
- [ ] Commit with subject `feat(agent): isolate child workspace`.

### Task 3: Project child events as candidates

**Files:**
- Create: `crates/executive/src/service/agent_control/candidate_projection.rs`
- Modify: `crates/executive/src/impl/runtime/native_cognit.rs`
- Test: `crates/executive/tests/agent_agora_projection.rs`

- [ ] Convert progress, evidence, hypotheses, criticism and terminal results into bounded `WorkspaceCandidate` values with child Process/Operation/source refs.
- [ ] Set private child visibility by default; only explicitly exportable evidence gets `AgentTree` visibility.
- [ ] Submit through C01's candidate port; never call Agora commit or broadcast directly.
- [ ] Apply TTL, source quota, content fingerprint deduplication and artifact-reference rules.
- [ ] Test unselected child output never enters root context or Dasein.
- [ ] Run `cargo test -p executive --test agent_agora_projection`; expect only explicitly exportable selected evidence to cross the child boundary.
- [ ] Commit with subject `feat(agent): submit typed child candidates`.

### Task 4: Close provenance and deletion gates

**Files:**
- Modify: `crates/executive/tests/agent_agora_projection.rs`
- Modify: `scripts/architecture-check.sh`

- [ ] Prove root broadcast ID -> child run -> child candidate -> later selection is replayable.
- [ ] Reject direct child calls to `AgoraOps::commit`, Dasein transition and global memory record.
- [ ] Run `cargo test -p executive --test agent_context_projection --test agent_agora_projection && cargo test -p agora --test candidate_selection`; expect PASS.
- [ ] Commit with subject `test(agent): lock isolated Agora participation`.

## Final verification

Run `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`; expect the architecture gate and complete workspace suite to pass before the final stage commit.

## Completion evidence

- [ ] Context is bounded, deterministic and instruction-safe.
- [ ] Child spaces and candidates preserve complete provenance.
- [ ] Only C01 selection can make child content globally available.
