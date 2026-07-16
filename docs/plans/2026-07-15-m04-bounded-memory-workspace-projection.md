# M04 Bounded Memory Workspace Projection Implementation Plan

**Goal:** Make one bounded, labelled memory projection the only path from Mnemosyne recall into the conscious workspace.

**Architecture:** Mnemosyne owns a pure projector from canonical `RecallSet` to bounded Fabric candidates; Executive only submits candidates and C01 alone selects globally visible memory.

**Tech Stack:** Rust, Fabric workspace contracts, Mnemosyne recall facade, Executive integration tests

**Source requirements:** `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md:397-451`.

**Prerequisites:** M03 and C01.

## Current-code anchors

- `MemoryService::recall` is the canonical recall facade at `crates/mnemosyne/src/service.rs:186-190`.
- `RecallItem` and `RecallSet` already carry canonical records and health at `crates/mnemosyne/src/service.rs:110-176`.
- Workspace content, provenance, visibility and salience contracts exist at `crates/fabric/src/types/workspace.rs:44-220`.
- Executive still owns a concrete `RecallMemory` handle in memory assembly at `crates/executive/src/core/memory_group.rs:13-17`.

## Invariants and non-goals

- No memory backend becomes an Agora dependency.
- No recalled text is promoted to system-role instructions.
- Constitutional memory uses an explicit policy receipt rather than a hidden bypass.

## Key contracts

```rust
pub struct MemoryProjectionLimits { pub max_items: usize, pub max_total_bytes: usize, pub max_item_bytes: usize }
pub struct MemoryProjection { pub records: Vec<ProjectedMemory>, pub omitted_count: usize, pub degraded_sources: Vec<String> }
pub trait MemoryWorkspaceProjector { fn project(&self, recall: &RecallSet, limits: MemoryProjectionLimits) -> anyhow::Result<MemoryProjection>; }
```

## Task 1: Freeze projection limits with failing tests

**Create:** `crates/mnemosyne/tests/bounded_workspace_projection.rs`

- [ ] Cover the 8-item and 16-KiB defaults, a hard per-item byte limit, deterministic ordering and omitted count.
- [ ] Assert that projected text is labelled data and cannot add system-role instructions.
- [ ] Assert source, observed time, temporal state, confidence, authority and scope survive projection.

Run: `cargo test -p mnemosyne --test bounded_workspace_projection`

Expected before implementation: compilation fails because the projector does not exist.

## Task 2: Add the pure bounded projector

**Create:** `crates/mnemosyne/src/projection.rs`
**Modify:** `crates/mnemosyne/src/lib.rs`

- [ ] Define `MemoryProjectionLimits`, `ProjectedMemory`, `MemoryProjection` and `MemoryWorkspaceProjector`.
- [ ] Accept only a `RecallSet`; do not expose local backend types.
- [ ] Sort by authority, recall score, observed time and record ID so replay is deterministic.
- [ ] Truncate on UTF-8 boundaries, report omitted items and retain degraded source health.
- [ ] Convert eligible records to `WorkspaceCandidate` with typed provenance and private visibility.

Run: `cargo test -p mnemosyne --test bounded_workspace_projection`

Expected: all projection contract tests pass.

## Task 3: Replace parallel Executive memory injection

**Modify:** `crates/executive/src/core/memory_group.rs`
**Modify:** `crates/executive/src/service/daemon_turn/orchestrator.rs`
**Create:** `crates/executive/tests/memory_workspace_entry.rs`

- [ ] Recall once through `MemoryService`, build one projection, and submit candidates through the C01 candidate port.
- [ ] Remove direct Core/composite recall text concatenation from production turn assembly.
- [ ] Allow constitutional records only through an explicit Dasein/Core policy branch with an auditable reason.
- [ ] Make Cognit receive selected memory via the C01 context projection, never by querying a memory backend.
- [ ] Persist selected content ID and broadcast epoch against the source memory record.

Run: `cargo test -p executive --test memory_workspace_entry`

Expected: unselected memories never appear in model context; selected labelled memories do.

## Task 4: Add regression and deletion gates

**Modify:** `scripts/architecture-check.sh`
**Modify:** `docs/design/architecture-overview.md`

- [ ] Reject new Executive/Cognit imports of Mnemosyne backend modules.
- [ ] Reject direct prompt insertion from CoreMemory, FactStore or GBrain results.
- [ ] Document `recall -> bounded candidates -> C01 selection -> context` as the sole production path.

Run: `bash scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

## Final verification and commit

Run: `scripts/architecture-check.sh && cargo test --workspace --all-targets --no-fail-fast`

Inspect the staged diff, then commit with subject `feat(mnemosyne): project bounded memory candidates` and a body that records the source requirement, authority/bypass problem, implemented boundaries, focused tests and deletion evidence.

## Completion evidence

- [ ] Limit and prompt-boundary tests pass.
- [ ] Every active memory has a record ID, candidate content ID and broadcast epoch.
- [ ] No production caller bypasses `MemoryService` or C01 selection.
