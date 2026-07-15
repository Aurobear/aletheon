# S01 Session Turn Item Contracts Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Define versioned Session/Turn/Item contracts and persist them through one canonical append API that deterministically rebuilds next-turn context.

**Architecture:** Fabric owns transport-neutral lifecycle records; Executive owns a SQLite append store with optimistic sequence checks. History items, memory, and trace remain separate: the canonical store records conversation/lifecycle facts and references, not memory payloads or telemetry blobs.

**Tech Stack:** Rust, serde, schemars, rusqlite, UUID, Fabric contracts

**Prerequisites:** E01 architecture check passes. This plan can run beside E02/E03.

**Source requirements:** `docs/plans/2026-07-15-architecture-coupling-optimization-plan.md:981-1006`.

---

## Anchors and invariants

- Existing `TurnRequest`/`TurnResult` live at `crates/fabric/src/types/turn.rs:7-55` and remain execution DTOs.
- Reuse the existing `fabric::SessionId` from `crates/fabric/src/types/space.rs:10`; creating another Session identifier is forbidden.
- Existing journal/store are `crates/executive/src/impl/session/journal.rs:15-180` and `crates/executive/src/impl/session/store.rs:11-130`.
- Append ordering is `(session_id, sequence)`; retries use stable `item_id`; conflicting sequence fails.
- Rebuilding context is pure and deterministic.
- Non-goals: turn orchestration, memory recall, and trace storage. Schema generation and typed Interact lifecycle artifacts are in scope because the source requirement explicitly requires them.

```text
Fabric lifecycle records -> SessionStore.append(expected_sequence, item)
                              |-> SQLite transaction
                              `-> load_items -> ContextProjector
```

## File map

- Create: `crates/fabric/src/types/session.rs` — versioned identifiers/records/notifications.
- Modify: `crates/fabric/src/types/mod.rs`, `crates/fabric/src/lib.rs` — exports.
- Create: `crates/fabric/tests/session_contract.rs` — serialization compatibility.
- Modify: `crates/fabric/Cargo.toml` — add workspace-compatible `schemars` dependency.
- Create: `crates/fabric/examples/export_session_schema.rs` — deterministic schema exporter.
- Create: `schemas/session-v1.schema.json` — checked-in generated schema.
- Create: `crates/interact/src/tui/session_protocol.rs` — typed lifecycle request/notification constructors.
- Modify: `crates/interact/src/tui/mod.rs` — export typed module.
- Create: `crates/interact/tests/session_protocol.rs` — JSON snapshot parity.
- Create: `crates/executive/src/impl/session/canonical_store.rs` — append/load store.
- Modify: `crates/executive/src/impl/session/mod.rs` — export.
- Create: `crates/executive/tests/session_append_store.rs` — persistence/idempotency/reopen.

### Task 1: Land versioned Fabric records

- [ ] Write serialization tests for `SessionRecord`, `TurnRecord`, `ItemRecord`, `ItemPayload::{UserMessage,AssistantMessage,ToolCall,ToolResult,SystemNotice}`, and `SessionNotification::ItemAppended` with `schema_version == 1`.
- [ ] Run `cargo test -p fabric --test session_contract`; expected FAIL: module absent.
- [ ] Add only new `TurnId(Uuid)` and `ItemId(Uuid)` types and reuse `crate::SessionId`. Implement these exact record shapes:

```rust
pub const SESSION_SCHEMA_VERSION: u16 = 1;

pub struct SessionRecord {
    pub schema_version: u16,
    pub id: SessionId,
    pub parent: Option<SessionFork>,
    pub created_at_ms: u64,
    pub status: SessionStatus,
}
pub struct SessionFork { pub session_id: SessionId, pub through_sequence: u64 }
pub enum SessionStatus { Active, Interrupted, Completed, Failed }

pub struct TurnRecord {
    pub schema_version: u16,
    pub id: TurnId,
    pub session_id: SessionId,
    pub operation_id: OperationId,
    pub started_at_ms: u64,
    pub completed_at_ms: Option<u64>,
    pub stop: Option<TurnStop>,
}
pub struct ItemRecord {
    pub schema_version: u16,
    pub id: ItemId,
    pub session_id: SessionId,
    pub turn_id: TurnId,
    pub sequence: u64,
    pub created_at_ms: u64,
    pub payload: ItemPayload,
}
```

`ItemPayload::ToolCall` and `ToolResult` both carry `call_id`; tool result also carries `is_error`, `permit_id`, and `audit_id`. Apply `#[serde(tag = "type", content = "data", rename_all = "snake_case")]` to enums. Unknown enum variants fail normal serde deserialization; forward compatibility comes from `schema_version` and explicit new protocol versions, not silently ignored facts.
- [ ] Run the test; expected PASS and stable JSON fixture equality.

### Task 2: Generate schema and typed Interact lifecycle artifacts

- [ ] Add `JsonSchema` derives to the lifecycle records and a failing test that runs `cargo run -p fabric --example export_session_schema`, compares stdout to `schemas/session-v1.schema.json`, and deserializes the fixture back into the root protocol type.
- [ ] Add `schemars = "1"` to Fabric and implement the exporter with `schemars::schema_for!(SessionProtocolV1)` plus `serde_json::to_string_pretty`; append exactly one newline so repeated generation is byte-identical.
- [ ] Create typed `SessionRpcRequest::{Resume,Fork,Interrupt,Replay}` parameter structs and `SessionClientNotification` in `crates/interact/src/tui/session_protocol.rs`; constructors serialize Fabric IDs and never use string-index mutation.
- [ ] Add snapshots for the four request methods and `item_appended` notification in `crates/interact/tests/session_protocol.rs`.
- [ ] Run `cargo run -q -p fabric --example export_session_schema > /tmp/session-schema.json && diff -u schemas/session-v1.schema.json /tmp/session-schema.json && cargo test -p interact --test session_protocol`; expected PASS.

### Task 3: Define the canonical append port

- [ ] Add compile tests for:

```rust
#[async_trait]
pub trait SessionAppendStore: Send + Sync {
    async fn create(&self, session: SessionRecord) -> Result<()>;
    async fn append(&self, session: &SessionId, expected_sequence: u64, item: ItemRecord) -> Result<AppendOutcome>;
    async fn fork(&self, parent: &SessionId, through_sequence: u64, child: SessionRecord) -> Result<()>;
    async fn load_session(&self, session: &SessionId) -> Result<Option<SessionRecord>>;
    async fn load_items(&self, session: &SessionId, after: Option<u64>) -> Result<Vec<ItemRecord>>;
}
```

- [ ] Run `cargo test -p fabric --test session_contract append_store_is_object_safe`; expected FAIL.
- [ ] Add the port and `AppendOutcome::{Appended,AlreadyPresent}` to `types/session.rs`; verify object safety and exports.

### Task 4: Implement transactional SQLite append

- [ ] Create tests using a temp database: create/reopen, append sequence 1, retry same ID returns `AlreadyPresent`, different ID at sequence 1 returns conflict, sequence 3 after 1 returns expected-sequence error.
- [ ] Run `cargo test -p executive --test session_append_store`; expected FAIL: store absent.
- [ ] Implement schema:

```sql
CREATE TABLE sessions(session_id TEXT PRIMARY KEY, schema_version INTEGER NOT NULL, record_json TEXT NOT NULL, next_sequence INTEGER NOT NULL);
CREATE TABLE session_items(session_id TEXT NOT NULL, sequence INTEGER NOT NULL, item_id TEXT NOT NULL UNIQUE, turn_id TEXT NOT NULL, item_json TEXT NOT NULL, PRIMARY KEY(session_id, sequence));
```

Inside one `TransactionBehavior::Immediate` transaction: read `next_sequence`, check idempotency by `item_id`, compare expected/current sequence, insert item, increment sequence, commit. Implement `fork` in one immediate transaction: verify the parent's sequence exists, insert the child record, copy rows `sequence <= through_sequence` with new child item IDs but identical payload/turn lineage, and set the child's `next_sequence` to `through_sequence + 1`.
- [ ] Run the focused test; expected PASS before and after reopening the file.

### Task 5: Deterministically project next-turn context

- [ ] Add a test with system, user, tool-call, tool-result, assistant items and assert two independent projections are byte-identical and ordered by sequence.
- [ ] Run it; expected FAIL: projector absent.
- [ ] Add `project_messages(items: &[ItemRecord]) -> Result<Vec<LlmMessage>>` in Executive session module. Reject duplicate/non-increasing sequence; map only conversational item payloads; retain tool call/result correlation ID; ignore lifecycle notifications.
- [ ] Run the exact test; expected PASS.

### Task 6: Adapt, but do not yet delete, legacy persistence

- [ ] Add an adapter test proving an existing `SessionEvent::UserMessage` and `AssistantMessage` append as equivalent canonical items without writing memory/trace tables.
- [ ] Implement `LegacyJournalProjector` beside `journal.rs`; mark it `pub(crate)` and use it only where the old SessionManager persists history.
- [ ] Run `cargo test -p executive --test session_append_store`; expected PASS and one canonical row per event.

### Task 7: Verify and commit

- [ ] Run `cargo fmt --all -- --check && cargo test -p fabric --test session_contract && cargo run -q -p fabric --example export_session_schema > /tmp/session-schema.json && diff -u schemas/session-v1.schema.json /tmp/session-schema.json && cargo test -p interact --test session_protocol && cargo test -p executive --test session_append_store && cargo test -p executive --test turn_service_equivalence && bash scripts/architecture-check.sh`.
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
- [ ] Generated JSON Schema is byte-identical and Interact uses typed lifecycle constructors.
- [ ] Retry is idempotent; conflicting/gapped sequence fails.
- [ ] Reopen and deterministic projection tests pass.
- [ ] Memory and trace stores receive no canonical history payload.
