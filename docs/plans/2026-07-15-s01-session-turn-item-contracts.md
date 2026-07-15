# S01 Session Turn Item Contracts Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define versioned Session/Turn/Item contracts and persist them through one canonical append API that deterministically rebuilds next-turn context.

**Architecture:** Fabric owns transport-neutral lifecycle records; Executive owns a SQLite append store with optimistic sequence checks. History items, memory, and trace remain separate: the canonical store records conversation/lifecycle facts and references, not memory payloads or telemetry blobs.

**Tech Stack:** Rust, serde, rusqlite, UUID, Fabric contracts

**Prerequisites:** E01 architecture check passes. This plan can run beside E02/E03.

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:981-1006`.

---

## Anchors and invariants

- Existing `TurnRequest`/`TurnResult` live at `crates/fabric/src/types/turn.rs:7-55` and remain execution DTOs.
- Existing journal/store are `crates/executive/src/impl/session/journal.rs:15-180` and `crates/executive/src/impl/session/store.rs:11-130`.
- Append ordering is `(session_id, sequence)`; retries use stable `item_id`; conflicting sequence fails.
- Rebuilding context is pure and deterministic.
- Non-goals: turn orchestration, memory recall, trace storage, client code generation, resume/fork APIs (later S02/V plans).

```text
Fabric lifecycle records -> SessionStore.append(expected_sequence, item)
                              |-> SQLite transaction
                              `-> load_items -> ContextProjector
```

## File map

- Create: `crates/fabric/src/types/session.rs` — versioned identifiers/records/notifications.
- Modify: `crates/fabric/src/types/mod.rs`, `crates/fabric/src/lib.rs` — exports.
- Create: `crates/fabric/tests/session_contract.rs` — serialization compatibility.
- Create: `crates/executive/src/impl/session/canonical_store.rs` — append/load store.
- Modify: `crates/executive/src/impl/session/mod.rs` — export.
- Create: `crates/executive/tests/session_append_store.rs` — persistence/idempotency/reopen.

### Task 1: Land versioned Fabric records

- [ ] Write serialization tests for `SessionRecord`, `TurnRecord`, `ItemRecord`, `ItemPayload::{UserMessage,AssistantMessage,ToolCall,ToolResult,SystemNotice}`, and `SessionNotification::ItemAppended` with `schema_version == 1`.
- [ ] Run `cargo test -p fabric --test session_contract`; expected FAIL: module absent.
- [ ] Add newtypes `SessionId`, `TurnId`, `ItemId`; records with IDs, parent/fork lineage, status, monotonic `sequence: u64`, timestamps, and payload. Use `#[serde(tag = "type", content = "data", rename_all = "snake_case")]` on enums and deny unknown enum variants only in tests, not production deserialization.
- [ ] Run the test; expected PASS and stable JSON fixture equality.

### Task 2: Define the canonical append port

- [ ] Add compile tests for:

```rust
#[async_trait]
pub trait SessionAppendStore: Send + Sync {
    async fn create(&self, session: SessionRecord) -> Result<()>;
    async fn append(&self, session: &SessionId, expected_sequence: u64, item: ItemRecord) -> Result<AppendOutcome>;
    async fn load_session(&self, session: &SessionId) -> Result<Option<SessionRecord>>;
    async fn load_items(&self, session: &SessionId, after: Option<u64>) -> Result<Vec<ItemRecord>>;
}
```

- [ ] Run `cargo test -p fabric --test session_contract append_store_is_object_safe`; expected FAIL.
- [ ] Add the port and `AppendOutcome::{Appended,AlreadyPresent}` to `types/session.rs`; verify object safety and exports.

### Task 3: Implement transactional SQLite append

- [ ] Create tests using a temp database: create/reopen, append sequence 1, retry same ID returns `AlreadyPresent`, different ID at sequence 1 returns conflict, sequence 3 after 1 returns expected-sequence error.
- [ ] Run `cargo test -p executive --test session_append_store`; expected FAIL: store absent.
- [ ] Implement schema:

```sql
CREATE TABLE sessions(session_id TEXT PRIMARY KEY, schema_version INTEGER NOT NULL, record_json TEXT NOT NULL, next_sequence INTEGER NOT NULL);
CREATE TABLE session_items(session_id TEXT NOT NULL, sequence INTEGER NOT NULL, item_id TEXT NOT NULL UNIQUE, turn_id TEXT NOT NULL, item_json TEXT NOT NULL, PRIMARY KEY(session_id, sequence));
```

Inside one `TransactionBehavior::Immediate` transaction: read `next_sequence`, check idempotency by `item_id`, compare expected/current sequence, insert item, increment sequence, commit.
- [ ] Run the focused test; expected PASS before and after reopening the file.

### Task 4: Deterministically project next-turn context

- [ ] Add a test with system, user, tool-call, tool-result, assistant items and assert two independent projections are byte-identical and ordered by sequence.
- [ ] Run it; expected FAIL: projector absent.
- [ ] Add `project_messages(items: &[ItemRecord]) -> Result<Vec<LlmMessage>>` in Executive session module. Reject duplicate/non-increasing sequence; map only conversational item payloads; retain tool call/result correlation ID; ignore lifecycle notifications.
- [ ] Run the exact test; expected PASS.

### Task 5: Adapt, but do not yet delete, legacy persistence

- [ ] Add an adapter test proving an existing `SessionEvent::UserMessage` and `AssistantMessage` append as equivalent canonical items without writing memory/trace tables.
- [ ] Implement `LegacyJournalProjector` beside `journal.rs`; mark it `pub(crate)` and use it only where the old SessionManager persists history.
- [ ] Run `cargo test -p executive --test session_append_store`; expected PASS and one canonical row per event.

### Task 6: Verify and commit

- [ ] Run `cargo fmt --all -- --check && cargo test -p fabric --test session_contract && cargo test -p executive --test session_append_store && cargo test -p executive --test turn_service_equivalence && bash scripts/architecture-check.sh`.
- [ ] Expected: all pass; history tests do not touch Mnemosyne or trace storage.
- [ ] Commit:

```text
feat(session): add canonical versioned append history

Session history was represented by local event shapes without a stable ordering
or replay contract. Add versioned Fabric records and one transactional append
store that deterministically projects the next turn context.

- define Session, Turn, Item, and notification contracts
- enforce idempotent optimistic sequence appends
- keep history separate from memory and trace data
```

## Compatibility deletion gate and evidence

Delete `LegacyJournalProjector` after S02 migrates both daemon and exec and a restart test reads only the canonical store. Keep legacy tables read-only for one release, then remove them under V02 migration evidence.

- [ ] Stable schema-v1 fixtures round-trip.
- [ ] Retry is idempotent; conflicting/gapped sequence fails.
- [ ] Reopen and deterministic projection tests pass.
- [ ] Memory and trace stores receive no canonical history payload.
