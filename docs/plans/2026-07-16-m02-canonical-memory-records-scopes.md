# M02 Canonical Memory Records and Scopes Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Establish one validated, stably serialized memory record, scope and authority vocabulary while retaining adapters for existing stores.

**Architecture:** Put normalized semantics in `mnemosyne::model`, make the facade and scoped CoreMemory consume those types, and retain `RecallItem` as a compatibility projection. Storage schemas and daemon recall routing remain unchanged until M03.

**Tech Stack:** Rust, serde/serde_json, existing Mnemosyne facade and SQLite adapters.

**Prerequisites:** M01 contract suite passes with two explicit M03 targets ignored.

**Source requirements:** `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md:198-272` and `:328-353`.

---

## Current-code anchors

- Facade metadata and the coarse `All | Session(String)` scope are at `crates/mnemosyne/src/service.rs:26-129` and `:237-258`.
- A second public CoreMemory scope is at `crates/mnemosyne/src/impl/core_memory/scope.rs:19-62`.
- Fact recall constructs compatibility `RecallItem` values at `crates/mnemosyne/src/service.rs:336-400`.
- GBrain constructs the same projection at `crates/mnemosyne/src/backends/gbrain/page.rs:138-170`.

## Invariants and non-goals

- There is exactly one public `MemoryScope` enum after this slice.
- IDs, scope identifiers and record contents are non-empty; content is at most 256 KiB.
- Confidence is finite in `[0, 1]`; validity intervals are ordered.
- Scope visibility requires explicit ancestry and never infers child-to-parent promotion.
- Authority resolves cross-source conflicts before time/confidence ranking.
- No database schema, recall backend selection or prompt injection changes in M02.

## File map

- Create: `crates/mnemosyne/src/model/mod.rs`
- Create: `crates/mnemosyne/src/model/record.rs`
- Create: `crates/mnemosyne/src/model/scope.rs`
- Create: `crates/mnemosyne/tests/canonical_memory_model.rs`
- Modify: `crates/mnemosyne/src/lib.rs`
- Modify: `crates/mnemosyne/src/service.rs`
- Modify: `crates/mnemosyne/src/impl/core_memory/scope.rs`
- Modify: compatibility `RecallItem` constructors in GBrain and Executive tests.

### Task 1: Define canonical scope and explicit ancestry

- [ ] Add `Global`, `Principal(String)`, `Session(String)`, `Goal(String)`, `Agent(String)` and `Task(String)` with snake-case tagged serialization.
- [ ] Add `ScopeAncestry { principal_id, session_id, goal_id, agent_id, task_id }` and `allows(&self, ancestry)`.
- [ ] Test that Task visibility requires its exact task plus matching ancestors, Agent excludes siblings, Session excludes other sessions, and Global is visible everywhere.

Run: `cargo test -p mnemosyne --test canonical_memory_model scope_`

Expected: PASS with no implicit sibling or upward promotion.

### Task 2: Define validated normalized records

- [ ] Add `MemoryRecordId`, `MemoryKind`, `MemoryStatus`, `MemoryAuthority` and `MemoryRecord` exactly matching the source vocabulary.
- [ ] Implement `MemoryRecord::validate` for ID/content/source-event/tag bounds, metadata validity and canonical scope validity.
- [ ] Add stable JSON fixture round-trip and rejection tests for empty IDs, oversized content, invalid confidence and invalid intervals.

Run: `cargo test -p mnemosyne --test canonical_memory_model record_`

Expected: PASS; serialized enum spellings are stable snake_case values.

### Task 3: Remove the duplicate public scope

- [ ] Re-export `model::MemoryScope` from the crate root and facade.
- [ ] Change `MemoryService::consolidate` to the canonical scope and replace old `All` calls with `Global`.
- [ ] Make `ScopedCoreMemory` use canonical scopes, including explicit IDs for Session and the new Principal/Goal/Task variants.
- [ ] Preserve existing parent/child CoreMemory permissions and update tests to use explicit scope IDs.

Run: `cargo test -p mnemosyne r#impl::core_memory::scope`

Expected: all existing isolation tests PASS using the canonical type.

### Task 4: Add authority to recall projections and normalized conversion

- [ ] Add `authority: MemoryAuthority` to `RecallItem` with a safe `RawExperience` serde default.
- [ ] Classify local facts as `VerifiedLocalSemantic`, Core as `ApprovedCore`, local episodes/outcomes as `LocalEpisode`, Aletheon-owned GBrain as `AletheonExternal`, and other supplemental records as `ExternalReference`.
- [ ] Implement `TryFrom<RecallItem> for MemoryRecord` and a compatibility projection from validated `MemoryRecord`.
- [ ] Update every constructor and assert JSON round-trip preserves authority.

Run: `cargo test -p mnemosyne && cargo test -p executive gbrain_recall`

Expected: PASS; no recall projection lacks an explicit authority value in production constructors.

### Task 5: Verify and commit

```bash
cargo fmt --all -- --check
cargo clippy -p mnemosyne --all-targets -- -D warnings
cargo test -p mnemosyne
cargo test --workspace
bash tests/architecture_check.sh
bash scripts/architecture-check.sh
git diff --check
```

Expected: all commands exit 0 and architecture baselines do not grow.

Commit:

```text
feat(mnemosyne): define canonical memory records and scopes

Mnemosyne exposed two incompatible scope concepts and recall results had no
explicit authority, making isolation and conflict resolution ambiguous.

- add validated normalized record, status, kind and authority contracts
- replace both public scope enums with one ancestry-aware vocabulary
- retain store and RecallItem compatibility without changing live routing
```

## Compatibility deletion gate

`RecallItem` remains only as the facade/output adapter until M03 makes `MemoryRecord` the internal merge/ranking unit. M03 must remove backend-specific conflict ranking and prove every local hit normalizes before filtering.

## Completion evidence

- [ ] only `model::MemoryScope` is public;
- [ ] stable serialization and all validation failures are tested;
- [ ] scope ancestry rejects unrelated sessions, Agents and Tasks;
- [ ] every recall projection carries authority;
- [ ] live daemon recall behavior is unchanged;
- [ ] workspace and architecture checks pass.
