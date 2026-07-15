# D03 Dasein Ledger, Replay and Causal Lineage Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Make accepted Dasein transitions durable, checksum-verifiable and deterministically replayable, while replacing wall-clock identity continuity with explicit causal lineage.

**Architecture:** A SQLite `SelfLedger` is injected into the D02 reducer. Each validated request is appended with a chained checksum before its now-infallible state application, so a crash can lose only the in-memory projection and restart replay repairs it. Periodic snapshots contain a schema/reducer version plus the verified event prefix from which all Dasein component state is explicitly derived; later ledger events replay after that prefix. Legacy SelfField constitutional tables remain canonical during this slice and their identity mutations gain parent-version lineage rather than wall-gap semantics.

**Tech Stack:** Rust, rusqlite transactions, serde JSON, SHA-256, Tokio reducer lock, existing `SelfFieldStore`.

**Prerequisites:** D02 (`32d6ac6`).

**Source requirements:** `docs/plans/2026-07-15-dasein-agora-conscious-core-plan.md:647-675`, `:782-804`, specifically ledger/snapshot/checksum/replay at `:792`, causal lineage at `:793`, deterministic restart at `:800`, and idempotence at `:801`.

---

## Current-code anchors

- Persistence stores and restores only mood via a raw setter at `crates/dasein/src/dasein/persistence.rs:6-27`.
- `SelfFieldStore` has only a generic `dasein_state` key/value table at `crates/dasein/src/core/store.rs:68-72`.
- D02 keeps version, receipts and narrative references only in memory at `crates/dasein/src/dasein/reducer.rs:20-65`.
- SelfField starts Sorge before loading the old mood record at `crates/dasein/src/core/mod.rs:331-345`.
- `ContinuityLayer::is_continuous` treats a wall-clock gap as an identity break at `crates/dasein/src/core/continuity.rs:50-63`.

## Invariants and non-goals

- Event sequence and self version are contiguous and unique; event IDs are globally idempotent.
- Every event checksum covers the previous checksum and canonical durable fields.
- Checksum, sequence, version or payload corruption fails startup closed; it is never skipped.
- Replay uses the same reducer as live transitions without appending duplicate ledger rows.
- Full Dasein lived/reflective state is declared derived from the verified ordered event stream in D03; snapshot event prefixes are an acceleration/checkpoint format, not a second authority.
- Restart records an explicit `ResumedAfterInterval` lived experience after verified replay; elapsed wall time never decides identity continuity.
- D03 does not migrate the separate legacy constitutional SelfField tables into the Dasein ledger; their eventual canonical merge remains part of F01/X02, but their continuity verdict becomes causal now.

## File map

- Create: `crates/dasein/src/dasein/ledger.rs`
- Create: `crates/dasein/tests/dasein_ledger_replay.rs`
- Modify: `crates/fabric/src/dasein/transition.rs`
- Modify: `crates/dasein/src/core/store.rs`
- Modify: `crates/dasein/src/core/continuity.rs`
- Modify: `crates/dasein/src/core/mod.rs`
- Modify: `crates/dasein/src/dasein/mod.rs`
- Modify: `crates/dasein/src/dasein/reducer.rs`
- Replace: `crates/dasein/src/dasein/persistence.rs`

### Task 1: Define the durable event and lineage contracts

- [x] Add `SelfEventV1` with schema/reducer version, request, receipt version range and checksum.
- [x] Add `SelfLineageV1` with version, parent version, mutation/approval references and checksum.
- [x] Add `ResumedAfterInterval` as a structured lived experience without identity-break semantics.
- [x] Round-trip and reject unsupported schema/reducer versions.

Run: `cargo test -p fabric dasein::transition::tests::durable_`

Expected: stable serde and version validation pass.

### Task 2: Add an append-only checksum ledger

- [x] Create `self_events`, `self_snapshots`, and `self_lineage` tables with uniqueness/contiguity constraints.
- [x] Canonically encode each durable event and chain SHA-256 from the previous checksum.
- [x] Append only after request validation and reducer preflight, before in-memory application.
- [x] Treat an existing matching event ID as idempotent and a mismatching duplicate as corruption.

Run: `cargo test -p dasein --test dasein_ledger_replay ledger_`

Expected: append/reopen/idempotence/corruption cases are deterministic.

### Task 3: Replay projections and checkpoints

- [x] Extract reducer preflight so every fallible check happens before ledger append.
- [x] Replay verified events through the exact D02 state application with persistence disabled.
- [x] Store a versioned checkpoint containing the verified event prefix and checksum.
- [x] Verify the newest checkpoint against the ledger prefix, reconstruct that event-derived projection, then apply the suffix.
- [x] Restore version, mood, temporality, world, self model, care-derived scheduling and narrative references as event-derived projections.

Run: `cargo test -p dasein --test dasein_ledger_replay replay_`

Expected: before/after context JSON and version are byte-equivalent, including after checkpoint plus suffix.

### Task 4: Make startup ordering and resumption truthful

- [x] Construct the reducer with the same `SelfFieldStore` ledger used by SelfField.
- [x] Verify/replay before starting Sorge.
- [x] Append one explicit resumption transition after replay when prior events exist.
- [x] Remove raw mood save/load and ensure shutdown checkpointing occurs after Sorge stops.

Run: `cargo test -p dasein --test dasein_ledger_replay restart_`

Expected: no background transition races load/checkpoint and restart adds exactly one resumption event.

### Task 5: Replace wall-gap continuity with causal lineage

- [x] Extend continuity records with parent identity version plus mutation/approval references.
- [x] Define mutation continuity as a single parent chain rooted at initialization; same-version legacy checkpoints remain observations, not new lineage nodes.
- [x] Infer causal parents and checksums when loading legacy rows; elapsed time is ignored.
- [x] Test long wall gaps remain continuous while missing/wrong parents fail closed.

Run: `cargo test -p dasein core::continuity`

Expected: causal linkage alone determines continuity.

### Task 6: Verify and commit

```bash
cargo fmt --all -- --check
cargo clippy -p fabric -p dasein --all-targets -- -D warnings
cargo test -p fabric
cargo test -p dasein
cargo test --workspace
bash tests/architecture_check.sh
bash scripts/architecture-check.sh
```

Commit subject: `feat(dasein): persist and replay the self ledger`

## Compatibility deletion gate

- The old `dasein_state` table is read only for one migration release and is never written after D03.
- Legacy constitutional layer tables remain until F01/X02 migrate their mutation commands into the canonical reducer.
- Raw component restore functions are `pub(crate)` and callable only by verified replay; static checks must reject other production callers.
- The old wall-gap `max_gap` config remains accepted but ignored until Q01 removes it from the public schema.

## Completion evidence

- [x] ledger append/reopen and hash-chain verification pass;
- [x] duplicate IDs survive restart without reapplication;
- [x] checkpoint plus suffix replay matches live state byte-for-byte;
- [x] event or checkpoint corruption prevents startup;
- [x] startup replays before Sorge and records resumption;
- [x] wall gaps do not break identity but causal gaps do;
- [x] no production raw mood persistence remains;
- [x] workspace and architecture checks pass.
