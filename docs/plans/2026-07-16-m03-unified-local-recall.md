# M03 Unified Local Recall Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make every locally recorded eligible memory recallable through one scoped, normalized, bounded `MemoryService::recall` path.

**Architecture:** Query RecallMemory, FactStore, EpisodicMemory and CoreMemory independently, normalize each hit into the canonical vocabulary, then apply scope/temporal/authority filtering, provenance deduplication and one final budget. `CompositeMemoryService` reuses the same merge policy for supplemental hits.

**Tech Stack:** Rust, Tokio, SQLite/FTS5, canonical Mnemosyne model.

**Prerequisites:** M02 canonical records/scopes and M01 target tests.

**Source requirements:** `docs/plans/2026-07-15-mnemosyne-unified-memory-plan.md:355-395`.

---

## Current-code anchors

- Live local recall only reads `FactStore` at `crates/mnemosyne/src/service.rs:269-334`.
- Message writes enter `RecallMemory` at `crates/mnemosyne/src/service.rs:224-242`.
- Reflection/decision/outcome writes enter Episodic at `crates/mnemosyne/src/service.rs:243-266`.
- Supplemental merge policy is embedded in `crates/mnemosyne/src/composite_service.rs:223-326`.
- M01 target tests are ignored in `crates/mnemosyne/tests/unified_memory_contract.rs`.

## Invariants and non-goals

- RecallMemory messages are visible only to the exact requested Session.
- Core records outrank conflicting lower-authority records.
- Every backend hit is normalized before common filtering/ranking.
- One backend degradation returns other non-authoritative local results and records degraded source names.
- One final item/byte budget applies after local and supplemental merge.
- This slice does not yet project recall into Agora or change prompt rendering.

## File map

- Create: `crates/mnemosyne/src/recall/mod.rs`
- Create: `crates/mnemosyne/src/recall/local.rs`
- Create: `crates/mnemosyne/src/recall/merge.rs`
- Create: `crates/mnemosyne/src/recall/rank.rs`
- Modify: `crates/mnemosyne/src/impl/recall_memory.rs`
- Modify: `crates/mnemosyne/src/service.rs`
- Modify: `crates/mnemosyne/src/composite_service.rs`
- Modify: `crates/mnemosyne/tests/unified_memory_contract.rs`

### Task 1: Add exact-session RecallMemory search

- [ ] Add `search_in_session(session_id, query, limit)` using FTS5 plus exact `session_id` and a LIKE fallback with the same scope predicate.
- [ ] Test that identical text in two sessions never crosses the requested boundary.

Run: `cargo test -p mnemosyne recall_memory::tests::fts_search_in_session`

Expected: PASS; only the requested session row is returned.

### Task 2: Normalize every local backend

- [ ] Add adapters for messages, facts, reflections and non-empty Core blocks.
- [ ] Assign canonical kind, Session/Global scope, provenance, temporal status and authority.
- [ ] Query independent store locks with `tokio::join!`; capture non-authoritative backend errors as degraded-source labels.

Run: `cargo test -p mnemosyne --test unified_memory_contract recorded_ -- --include-ignored`

Expected: the former message and reflection M03 targets PASS.

### Task 3: Centralize merge, rank and final bounds

- [ ] Resolve `supersedes` before filtering.
- [ ] Deduplicate by stable `(source, source_id)` provenance key.
- [ ] Rank authority first, then temporal state, validity/observation time, confidence and stable ID.
- [ ] Apply historical filtering and exactly one item/byte limit after merge.
- [ ] Replace CompositeMemoryService's private merge implementation with the shared function.

Run: `cargo test -p mnemosyne && cargo test -p executive --test gbrain_bootstrap --test gbrain_recall_injection`

Expected: PASS, including existing GBrain dedup and deterministic-budget fixtures.

### Task 4: Activate all M01 targets and add isolation/conflict cases

- [ ] Remove both M03 `#[ignore]` attributes.
- [ ] Add unrelated-session non-leak coverage.
- [ ] Add reflection relevance, current Core authority, historical filtering and duplicate local/supplemental coverage.

Run: `cargo test -p mnemosyne --test unified_memory_contract`

Expected: all tests PASS with zero ignored tests.

### Task 5: Full verification and commit

```bash
cargo fmt --all -- --check
cargo clippy -p mnemosyne --all-targets -- -D warnings
cargo test -p mnemosyne
cargo test --workspace
bash tests/architecture_check.sh
bash scripts/architecture-check.sh
git diff --check
```

Commit subject: `feat(mnemosyne): unify local memory recall`

## Compatibility deletion gate

The FactStore-only implementation and Composite-local merge helpers are deleted in this slice. Backend schema adapters remain until M05 consolidation migrates durable writes; they may not be accessed by Executive or Cognit directly.

## Completion evidence

- [ ] messages and reflections are recallable after record/reopen;
- [ ] unrelated Session data cannot leak;
- [ ] Core authority wins conflicts;
- [ ] historical records require explicit opt-in;
- [ ] one degraded backend does not erase other local results;
- [ ] all M01 targets are active and workspace checks pass.
