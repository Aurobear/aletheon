# Aletheon Memory Workspace Foundation Implementation Plan

> **For agentic workers:** Use `flow-feature` or `plans` to execute this plan task by task; keep the identity consolidation and Mnemosyne scope extension in one reviewable foundation commit.

**Goal:** Establish one host-owned workspace identity and a non-forgeable workspace memory key, then make workspace an authorization-bearing Mnemosyne scope used by the common recall prefilter.

**Architecture:** Fabric owns the canonical `WorkspaceIdentity` shared by checkpoint and trust modules. Mnemosyne derives an opaque `WorkspaceMemoryKey` from the verified repository fingerprint, or from a caller-supplied durable machine installation ID plus canonical path when no repository identity exists. `MemoryScope::Workspace` participates in the same exact ancestry and backend predicate pipeline as principal/session/goal/agent/task; prompt text never contributes scope.

**Tech Stack:** Rust 1.85, serde, SHA-256, Mnemosyne recall pipeline, SQLite vector metadata filters.

---

## Requirement and code anchors

- Consolidate the duplicate Fabric identity and derive repository/local workspace keys (`docs/plans/2026-08-01-unified-memory-gateway-design.md:section 4.1`).
- Add workspace to the host-derived memory ancestry (`docs/plans/2026-08-01-unified-memory-gateway-design.md:section 4.2`).
- The duplicate types currently live at `crates/fabric/src/types/workspace_checkpoint.rs:34-47` and `crates/fabric/src/types/workspace_trust.rs:53-62`.
- `MemoryScope` and `ScopeAncestry` currently omit workspace at `crates/mnemosyne/src/model/scope.rs:3-53`.
- The common backend predicate is generated at `crates/mnemosyne/src/recall/pipeline.rs:25-65` and serialized independently by the SQLite vector backend at `crates/mnemosyne/src/backends/vector_sqlite.rs:282-290`.

### Task 1: Consolidate the Fabric workspace identity

**Files:**
- Create: `crates/fabric/src/types/workspace_identity.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/types/workspace_checkpoint.rs`
- Modify: `crates/fabric/src/types/workspace_trust.rs`
- Modify: `crates/fabric/src/lib.rs`

- [ ] Add a failing unit test proving checkpoint and trust module imports resolve to the same concrete type.
- [ ] Move `WorkspaceIdentity` and its exact-match helper into `workspace_identity.rs` without changing its serialized field shape.
- [ ] Re-export the canonical type from both old modules so downstream paths remain source-compatible.
- [ ] Re-export the canonical module and type from Fabric's public API.
- [ ] Run `bash scripts/cargo-agent.sh test -p fabric workspace_identity -- --nocapture`.

### Task 2: Derive opaque workspace memory keys

**Files:**
- Create: `crates/mnemosyne/src/workspace.rs`
- Modify: `crates/mnemosyne/src/lib.rs`
- Create: `crates/mnemosyne/tests/workspace_memory_scope.rs`

- [ ] Write tests that a repository fingerprint produces `ws:repo:<fingerprint>` independent of path and machine ID.
- [ ] Write tests that a non-repository workspace produces deterministic `ws:local:<sha256>` keys, changes across canonical paths or machine installation IDs, and never contains the canonical path or installation ID.
- [ ] Write validation tests rejecting an empty machine installation ID and empty repository fingerprint.
- [ ] Implement a serializable `WorkspaceMemoryKey` newtype with a bounded `derive` constructor taking only canonical `WorkspaceIdentity` and a durable machine installation ID string.
- [ ] Hash local identity material with domain separation and explicit byte boundaries to prevent concatenation ambiguity.
- [ ] Run `bash scripts/cargo-agent.sh test -p mnemosyne --test workspace_memory_scope workspace_memory_key -- --nocapture`.

### Task 3: Add workspace to governed recall scope

**Files:**
- Modify: `crates/mnemosyne/src/model/scope.rs`
- Modify: `crates/mnemosyne/src/observability.rs`
- Modify: `crates/mnemosyne/src/recall/pipeline.rs`
- Modify: `crates/mnemosyne/src/backends/vector_sqlite.rs`
- Modify: `crates/mnemosyne/src/agent_scope.rs`
- Modify: `crates/mnemosyne/tests/canonical_memory_model.rs`
- Modify: `crates/mnemosyne/tests/workspace_memory_scope.rs`

- [ ] Add a failing test showing an exact workspace ancestry grants visibility and a different workspace does not.
- [ ] Add a failing predicate test showing `workspace:<opaque-key>` reaches the backend filter and is deduplicated with all other verified ancestry members.
- [ ] Add `MemoryScope::Workspace(String)` and `ScopeAncestry.workspace_id` with the existing ID validation and exact-match behavior.
- [ ] Add the closed `Workspace` metrics label and workspace key serialization to every exhaustive scope match.
- [ ] Ensure child-only ancestry explicitly has no workspace until the host injects one; do not infer it from task content.
- [ ] Run `bash scripts/cargo-agent.sh test -p mnemosyne --test workspace_memory_scope -- --nocapture`.
- [ ] Run `bash scripts/cargo-agent.sh test -p mnemosyne --test canonical_memory_model -- --nocapture`.

### Task 4: Foundation verification and commit

**Files:**
- Verify all files above.

- [ ] Run `bash scripts/cargo-agent.sh test -p fabric workspace_identity -- --nocapture`.
- [ ] Run `bash scripts/cargo-agent.sh test -p mnemosyne --test workspace_memory_scope -- --nocapture`.
- [ ] Run `bash scripts/cargo-agent.sh test -p mnemosyne --test canonical_memory_model -- --nocapture`.
- [ ] Run `bash scripts/cargo-agent.sh check -p mnemosyne` to catch exhaustive matches outside the focused tests.
- [ ] Run `bash scripts/cargo-agent.sh fmt --all -- --check` after formatting with the wrapper if needed.
- [ ] Inspect `git diff --check` and the staged diff.
- [ ] Commit the foundation with a conventional subject and a body describing the single identity, opaque derivation, and governed scope propagation.

## Acceptance

- Checkpoint, trust, and future memory code use one concrete `WorkspaceIdentity`.
- Repository identity is stable across checkout paths; local identity is machine-and-path scoped and opaque.
- Workspace visibility is exact, host-ancestry based, and reaches lexical/vector prefilters before retrieval.
- Existing serialized checkpoint/trust identity fields remain unchanged.
- Existing principal/session/goal/agent/task scope behavior remains compatible.
