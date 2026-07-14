# Aletheon M0–M1 Telegram Vertical Slice Detailed Implementation Plan

> **For agentic workers:** Implement one task at a time. Do not start the next task until the listed test and commit gate passes.

**Goal:** Preserve the existing daemon/chat/objective behavior, then add a durable owner-only Telegram text channel with restart-safe offset handling and duplicate suppression.

**Architecture:** Reuse `DaemonTurnOrchestrator::execute_turn()` as the only chat execution entry. Add shared channel DTOs in `fabric`, a SQLite channel store in `executive`, and a small HTTP-based Telegram adapter that durably records each update before routing it to the daemon.

**Tech Stack:** Rust 2021, Tokio, reqwest, rusqlite, serde/serde_json, tempfile, existing Aletheon daemon primitives.

---

## 1. Anchors and scope

Requirements touched by this plan:

- Preserve existing CLI/TUI conversation behavior: `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:52-59`.
- Telegram owner binding, long polling, DTO mapping, offset persistence, and unknown-user rejection: `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:61-70`.
- Use long polling and immutable Telegram user ID: `docs/arch/agent-google/03_CHANNEL_AND_MOBILE_COMMUNICATION.md:83-108`.
- Persist offsets, deduplicate, retry outbound delivery, retain correlation IDs, and restart gracefully: `docs/arch/agent-google/03_CHANNEL_AND_MOBILE_COMMUNICATION.md:220-232`.

Verified code this plan extends:

- Live daemon chat entry: `crates/executive/src/service/daemon_turn/execute.rs:13-100`.
- `RequestHandler` owns `turn_orchestrator`: `crates/executive/src/impl/daemon/handler/mod.rs:38-68`.
- Daemon data directory is created during initialization: `crates/executive/src/impl/daemon/handler/init.rs:203-206`.
- Existing objective persistence and recovery must not regress: `crates/executive/src/impl/goal/mod.rs:12-49`, `crates/executive/src/impl/goal/store.rs:74-111`, `crates/executive/src/impl/daemon/handler/init.rs:249-278`.
- `rusqlite`, `reqwest`, Tokio, serde, and tempfile are already available: `crates/executive/Cargo.toml:18-40`.

M1 deliberately supports only `/start`, `/chat <text>`, and plain text. Goal commands return a deterministic “not enabled until M2” response. Files, voice, images, approval buttons, Gmail, and Goal creation are outside this milestone.

## 2. Task 1 — Protect the existing baseline

**Files:**

- Create: `crates/executive/tests/agent_integration_baseline.rs`
- Read/Reuse: `crates/executive/tests/turn_service_equivalence.rs`
- Read/Reuse: `crates/executive/src/impl/goal/mod.rs`

### Step 1.1: Add the process-state regression test

- [ ] Create `crates/executive/tests/agent_integration_baseline.rs` with:

```rust
use fabric::ProcessState;

#[test]
fn generic_process_state_contract_is_unchanged() {
    assert!(ProcessState::Created.can_transition_to(ProcessState::Ready));
    assert!(ProcessState::Ready.can_transition_to(ProcessState::Running));
    assert!(ProcessState::Running.can_transition_to(ProcessState::Waiting));
    assert!(ProcessState::Waiting.can_transition_to(ProcessState::Running));
    assert!(ProcessState::Running.can_transition_to(ProcessState::Stopping));
    assert!(ProcessState::Stopping.can_transition_to(ProcessState::Exited));
    assert!(!ProcessState::Created.can_transition_to(ProcessState::Running));
}
```

### Step 1.2: Add the ObjectiveStore restart regression test

- [ ] Add this integration test to the same file; `executive::r#impl::goal::ObjectiveStore` is reachable through `executive/src/lib.rs:13-15`:

```rust
#[test]
fn objective_store_reopens_and_resumes_existing_objective() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("objectives.db");
    let id = {
        let store = executive::r#impl::goal::ObjectiveStore::open(&path).unwrap();
        store.create("preserve me", None, "session-a", "project").unwrap()
    };
    let reopened = executive::r#impl::goal::ObjectiveStore::open(&path).unwrap();
    let (active, children) = reopened.resume().unwrap().unwrap();
    assert_eq!(active.objective_id, id);
    assert_eq!(active.description, "preserve me");
    assert!(children.is_empty());
}
```

### Step 1.3: Run and commit

- [ ] Run:

```bash
cargo test -p executive --test agent_integration_baseline
cargo test -p executive --test turn_service_equivalence
cargo test -p executive -- impl::goal
```

Expected: all commands exit 0.

- [ ] Commit:

```text
test(executive): protect agent integration baseline

The channel integration must not replace the existing process lifecycle or
objective restart behavior.

- lock the generic ProcessState transition contract
- cover ObjectiveStore reopen and active-objective recovery
```

## 3. Task 2 — Define code-native channel DTOs

**Files:**

- Create: `crates/fabric/src/types/channel.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`

### Step 2.1: Write DTO tests first

- [ ] Start `channel.rs` with the following tests before defining the types:

```rust
#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_text_round_trips() {
        let message = InboundMessage {
            channel_id: ChannelId("telegram".into()),
            message_id: MessageId("42".into()),
            conversation_id: ConversationId("1001".into()),
            sender_id: ExternalSenderId("telegram:7".into()),
            content: MessageContent::Text { text: "hello".into() },
            timestamp_ms: 1_720_000_000_000,
            reply_to_action: None,
            correlation_id: "telegram:42".into(),
        };
        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(serde_json::from_str::<InboundMessage>(&json).unwrap(), message);
    }

    #[test]
    fn outbound_actions_round_trip() {
        let message = OutboundMessage {
            conversation_id: ConversationId("1001".into()),
            content: MessageContent::Text { text: "continue?".into() },
            actions: vec![UserAction {
                action_id: "approve:abc".into(),
                label: "Approve".into(),
                action_type: ActionType::Approve,
            }],
            reply_to: Some(MessageId("42".into())),
            correlation_id: "approval:abc".into(),
        };
        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(serde_json::from_str::<OutboundMessage>(&json).unwrap(), message);
    }
}
```

### Step 2.2: Confirm the test fails

- [ ] Run `cargo test -p fabric -- types::channel`.

Expected: compilation fails because the channel types do not exist.

### Step 2.3: Add the minimal production types

- [ ] Add above the test module:

```rust
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExternalSenderId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageContent {
    Text { text: String },
    Command { command: String, args: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Callback,
    Url,
    Approve,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAction {
    pub action_id: String,
    pub label: String,
    pub action_type: ActionType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    pub conversation_id: ConversationId,
    pub sender_id: ExternalSenderId,
    pub content: MessageContent,
    pub timestamp_ms: i64,
    pub reply_to_action: Option<String>,
    pub correlation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub conversation_id: ConversationId,
    pub content: MessageContent,
    pub actions: Vec<UserAction>,
    pub reply_to: Option<MessageId>,
    pub correlation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ChannelHealth {
    Healthy,
    Degraded { reason: String },
    Disconnected { since_ms: i64, reason: String },
}
```

- [ ] Add `pub mod channel;` to `crates/fabric/src/types/mod.rs` and re-export these types from `crates/fabric/src/lib.rs` beside the other shared types.

### Step 2.4: Validate and commit

- [ ] Run:

```bash
cargo test -p fabric -- types::channel
cargo check -p fabric
```

Expected: both exit 0.

- [ ] Commit:

```text
feat(fabric): define external channel contracts

Channel adapters need stable provider-neutral messages before Telegram can be
wired into the daemon.

- add inbound and outbound channel DTOs
- keep external sender identity separate from PrincipalId
- include correlation and action fields for later approvals
```

## 4. Task 3 — Add a versioned channel database

**Files:**

- Create: `crates/executive/src/impl/channel/mod.rs`
- Create: `crates/executive/src/impl/channel/store.rs`
- Modify: `crates/executive/src/impl/mod.rs`

Use a dedicated `channels.db` under `DaemonConfig.data_dir`. Do not modify `objectives.db` or create a second Goal database.

### Step 3.1: Define schema and migration test

- [ ] In `store.rs`, first add a test that opens a temporary database twice and checks `PRAGMA user_version = 1` plus all four tables:

```rust
#[test]
fn migration_is_idempotent() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("channels.db");
    ChannelStore::open(&path).unwrap();
    let store = ChannelStore::open(&path).unwrap();
    assert_eq!(store.user_version().unwrap(), 1);
    for table in ["channel_inbox", "channel_outbox", "channel_cursor", "channel_binding"] {
        assert!(store.table_exists(table).unwrap(), "missing {table}");
    }
}
```

### Step 3.2: Implement migration 1

- [ ] Implement `ChannelStore::open()` with WAL, foreign keys, a transaction, and the following schema:

```sql
CREATE TABLE channel_inbox (
    channel_id      TEXT NOT NULL,
    message_id      TEXT NOT NULL,
    conversation_id TEXT NOT NULL,
    sender_id       TEXT NOT NULL,
    payload_json    TEXT NOT NULL,
    correlation_id  TEXT NOT NULL,
    status          TEXT NOT NULL DEFAULT 'pending'
                    CHECK(status IN ('pending','processing','completed','rejected','failed')),
    result_json     TEXT,
    attempt_count   INTEGER NOT NULL DEFAULT 0,
    created_at      TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at      TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY(channel_id, message_id)
);

CREATE TABLE channel_outbox (
    outbox_id        INTEGER PRIMARY KEY AUTOINCREMENT,
    channel_id       TEXT NOT NULL,
    conversation_id  TEXT NOT NULL,
    payload_json     TEXT NOT NULL,
    correlation_id   TEXT NOT NULL UNIQUE,
    status           TEXT NOT NULL DEFAULT 'pending'
                     CHECK(status IN ('pending','sending','sent','failed')),
    attempt_count    INTEGER NOT NULL DEFAULT 0,
    provider_message_id TEXT,
    last_error       TEXT,
    created_at       TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at       TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE channel_cursor (
    channel_id  TEXT PRIMARY KEY,
    cursor      TEXT NOT NULL,
    updated_at  TEXT NOT NULL DEFAULT (datetime('now'))
);

CREATE TABLE channel_binding (
    channel_id  TEXT NOT NULL,
    external_id TEXT NOT NULL,
    principal_id TEXT NOT NULL,
    status      TEXT NOT NULL DEFAULT 'active'
                CHECK(status IN ('pending','active','revoked')),
    created_at  TEXT NOT NULL DEFAULT (datetime('now')),
    updated_at  TEXT NOT NULL DEFAULT (datetime('now')),
    PRIMARY KEY(channel_id, external_id)
);
```

- [ ] Set `PRAGMA user_version = 1` only inside the successful migration transaction.
- [ ] Make `ChannelStore` hold `rusqlite::Connection`, matching the existing `ObjectiveStore` ownership style.

### Step 3.3: Validate and commit

- [ ] Run `cargo test -p executive -- impl::channel::store::tests::migration_is_idempotent`.

Expected: PASS.

- [ ] Commit:

```text
feat(executive): add durable channel database

Telegram offsets and messages must survive daemon restarts without affecting
the existing objective database.

- add versioned channel schema in channels.db
- store inbox, outbox, cursor, and identity bindings
- make the initial migration transactional and idempotent
```

## 5. Task 4 — Implement binding and inbox idempotency

**Files:**

- Modify: `crates/executive/src/impl/channel/store.rs`

### Step 4.1: Add binding tests

- [ ] Add tests:

```rust
#[test]
fn binding_resolves_only_active_principal() {
    let (store, _dir) = test_store();
    store.bind("telegram", "7", "owner", "active").unwrap();
    assert_eq!(store.resolve_principal("telegram", "7").unwrap().as_deref(), Some("owner"));
    assert_eq!(store.resolve_principal("telegram", "8").unwrap(), None);
}

#[test]
fn rebinding_same_external_identity_is_idempotent() {
    let (store, _dir) = test_store();
    store.bind("telegram", "7", "owner", "active").unwrap();
    store.bind("telegram", "7", "owner", "active").unwrap();
    assert_eq!(store.resolve_principal("telegram", "7").unwrap().as_deref(), Some("owner"));
}
```

### Step 4.2: Add inbox tests

- [ ] Add tests proving the first insert returns `Inserted` and the second returns `Duplicate` without replacing the original payload:

```rust
#[test]
fn duplicate_provider_message_is_not_inserted_twice() {
    let (mut store, _dir) = test_store();
    let first = sample_inbound("42", "first");
    let second = sample_inbound("42", "changed");
    assert_eq!(store.insert_inbound(&first).unwrap(), InsertOutcome::Inserted);
    assert_eq!(store.insert_inbound(&second).unwrap(), InsertOutcome::Duplicate);
    assert_eq!(store.load_inbound("telegram", "42").unwrap().unwrap().content, first.content);
}
```

### Step 4.3: Implement minimal repository methods

- [ ] Implement:

```rust
pub enum InsertOutcome { Inserted, Duplicate }

pub fn bind(&self, channel: &str, external: &str, principal: &str, status: &str) -> anyhow::Result<()>;
pub fn resolve_principal(&self, channel: &str, external: &str) -> anyhow::Result<Option<String>>;
pub fn insert_inbound(&mut self, message: &InboundMessage) -> anyhow::Result<InsertOutcome>;
pub fn load_inbound(&self, channel: &str, message_id: &str) -> anyhow::Result<Option<InboundMessage>>;
pub fn pending_inbound(&self, channel: &str, limit: usize) -> anyhow::Result<Vec<InboundMessage>>;
```

Use `INSERT OR IGNORE`, then check the affected-row count. Do not use `INSERT OR REPLACE`.

### Step 4.4: Validate and commit

- [ ] Run `cargo test -p executive -- impl::channel::store`.

Expected: PASS.

- [ ] Commit with subject `feat(executive): persist channel identities and inbox` and a body listing active-only resolution and provider-message idempotency.

## 6. Task 5 — Commit turn outcome, outbox, and cursor atomically

**Files:**

- Modify: `crates/executive/src/impl/channel/store.rs`

### Step 5.1: Write the atomic completion test

- [ ] Add a test that inserts an inbox row, calls `complete_inbound()`, then verifies all three effects:

```rust
#[test]
fn completion_persists_result_outbox_and_cursor_together() {
    let (mut store, _dir) = test_store();
    let inbound = sample_inbound("42", "hello");
    store.insert_inbound(&inbound).unwrap();
    let outbound = OutboundMessage {
        conversation_id: inbound.conversation_id.clone(),
        content: MessageContent::Text { text: "world".into() },
        actions: vec![],
        reply_to: Some(inbound.message_id.clone()),
        correlation_id: inbound.correlation_id.clone(),
    };
    store.complete_inbound("telegram", "42", "43", &outbound).unwrap();
    assert_eq!(store.inbox_status("telegram", "42").unwrap().as_deref(), Some("completed"));
    assert_eq!(store.cursor("telegram").unwrap().as_deref(), Some("43"));
    assert_eq!(store.pending_outbox("telegram", 10).unwrap(), vec![outbound]);
}
```

### Step 5.2: Add rollback coverage

- [ ] Add a test-only fault injection immediately before cursor update. Assert that inbox remains pending and no outbox row exists when the transaction returns an error.

### Step 5.3: Implement the transaction

- [ ] Implement:

```rust
pub fn complete_inbound(
    &mut self,
    channel: &str,
    message_id: &str,
    next_cursor: &str,
    outbound: &OutboundMessage,
) -> anyhow::Result<()>;
```

Inside one `rusqlite::Transaction`:

1. Insert outbox with `ON CONFLICT(correlation_id) DO NOTHING`.
2. Update inbox to completed and store serialized result.
3. Upsert cursor.
4. Commit.

### Step 5.4: Validate and commit

- [ ] Run `cargo test -p executive -- impl::channel::store`.
- [ ] Commit with subject `feat(executive): atomically settle channel messages` and explain the crash boundary in the body.

## 7. Task 6 — Define testable channel and turn boundaries

**Files:**

- Modify: `crates/executive/src/impl/channel/mod.rs`
- Create: `crates/executive/src/impl/channel/router.rs`

### Step 6.1: Add the traits

- [ ] Define these minimal boundaries:

```rust
#[async_trait::async_trait]
pub trait ChannelTransport: Send + Sync {
    fn channel_id(&self) -> &str;
    async fn receive(&self, cursor: Option<String>) -> anyhow::Result<Vec<ProviderEnvelope>>;
    async fn send(&self, message: &OutboundMessage) -> anyhow::Result<String>;
}

pub struct ProviderEnvelope {
    pub message: InboundMessage,
    pub next_cursor: String,
}

#[async_trait::async_trait]
pub trait ChannelTurnExecutor: Send + Sync {
    async fn execute(&self, message: &str, correlation_id: &str) -> anyhow::Result<String>;
}
```

`ChannelTurnExecutor` prevents router tests from constructing the entire daemon. The production adapter calls `DaemonTurnOrchestrator::execute_turn()` and extracts either `result` text or a stable error.

### Step 6.2: Add command normalization

- [ ] Implement a pure helper with tests:

```rust
enum RoutedInput {
    Greeting,
    Chat(String),
    GoalUnavailable,
    Unsupported(String),
}

fn route_content(content: &MessageContent) -> RoutedInput;
```

Rules:

- `/start` → greeting without LLM.
- `/chat x` → execute `x`.
- plain text → execute as chat.
- `/goal`, `/goals`, `/status`, `/pause`, `/resume`, `/cancel`, `/approve`, `/reject` → deterministic M2-unavailable response.
- empty input → unsupported.

### Step 6.3: Validate and commit

- [ ] Run `cargo test -p executive -- impl::channel::router`.
- [ ] Commit with subject `feat(executive): define channel routing boundaries`.

## 8. Task 7 — Implement the owner-only ChannelRouter

**Files:**

- Modify: `crates/executive/src/impl/channel/router.rs`
- Create: `crates/executive/tests/channel_router.rs`

### Step 7.1: Write fake implementations

- [ ] In the integration test, create:

```rust
#[derive(Default)]
struct FakeTurnExecutor {
    calls: tokio::sync::Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl ChannelTurnExecutor for FakeTurnExecutor {
    async fn execute(&self, message: &str, _correlation_id: &str) -> anyhow::Result<String> {
        self.calls.lock().await.push(message.to_string());
        Ok(format!("reply:{message}"))
    }
}
```

Also create a fake transport whose `send()` records outbound messages and returns a fake provider message ID.

### Step 7.2: Add owner, duplicate, and command tests

- [ ] Test all of the following:

1. Unknown `sender_id` is marked rejected and the executor call list stays empty.
2. Owner plain text invokes the executor once.
3. Replaying the same message invokes the executor zero additional times.
4. `/start` returns the greeting without invoking the executor.
5. `/goal example` returns the M2-unavailable response without creating an objective.
6. An executor failure leaves the inbox retryable and does not advance cursor.

### Step 7.3: Implement processing order

- [ ] Implement `ChannelRouter::process(envelope)` in this order:

```text
insert inbox or return Duplicate
resolve active binding
unknown -> mark rejected; advance cursor; optional rejection outbox
normalize command
execute chat if required
build outbound DTO
complete inbox + outbox + cursor transaction
attempt outbox send
mark sent or failed without rolling back completed turn
```

Persist the turn outcome before network send. A send failure retries only the outbox; it must not re-run the LLM turn.

### Step 7.4: Validate and commit

- [ ] Run `cargo test -p executive --test channel_router`.

Expected: PASS.

- [ ] Commit with subject `feat(executive): route durable owner channel messages` and document rejection-before-LLM and outbox-only retry.

## 9. Task 8 — Add the Telegram HTTP transport

**Files:**

- Create: `crates/executive/src/impl/channel/telegram/mod.rs`
- Create: `crates/executive/src/impl/channel/telegram/types.rs`
- Modify: `crates/executive/src/impl/channel/mod.rs`

Do not add teloxide in M1. Use the existing `reqwest` dependency so DeepSeek does not need to resolve another framework's lifecycle or version-specific API.

### Step 8.1: Define only the Telegram JSON used by M1

- [ ] Add serde DTOs for `getUpdates` response, update, message, chat, user, and `sendMessage` response. All optional Telegram fields must use `Option`; ignore unknown fields.

- [ ] Add fixture tests for:

1. text message;
2. `/chat hello` message;
3. update without message;
4. API `{ "ok": false, "description": "..." }`.

### Step 8.2: Implement conversion

- [ ] Convert a Telegram text update as follows:

```text
channel_id      = "telegram"
message_id      = update_id decimal string
conversation_id = chat.id decimal string
sender_id       = "telegram:" + from.id
timestamp_ms    = message.date * 1000
correlation_id  = "telegram:" + update_id
next_cursor     = update_id + 1
```

Parse a leading slash command into `MessageContent::Command`; otherwise use `Text`.

### Step 8.3: Implement HTTP calls

- [ ] Implement `TelegramTransport` with `reqwest::Client`, bot token, base URL, poll timeout, and cancellation token.
- [ ] `receive(cursor)` calls `GET {base}/bot{token}/getUpdates` with `offset`, `timeout`, and `allowed_updates=["message"]`.
- [ ] `send()` calls `POST {base}/bot{token}/sendMessage` with `chat_id` and plain text.
- [ ] Never log the full URL because it contains the bot token.
- [ ] Return sanitized errors without response bodies that may contain sensitive text.

### Step 8.4: Validate with a mock HTTP server

- [ ] Use a tiny local Tokio `TcpListener` fixture in the test so no new production or dev dependency is added.
- [ ] Verify offset, timeout, token redaction, conversion, and send payload.
- [ ] Run `cargo test -p executive -- impl::channel::telegram`.
- [ ] Commit with subject `feat(executive): add Telegram long-poll transport`.

## 10. Task 9 — Add explicit Telegram configuration

**Files:**

- Modify: `crates/cognit/src/config/mod.rs`
- Modify: `crates/executive/src/core/runtime_core.rs`
- Modify: `crates/executive/src/impl/daemon/mod.rs`
- Modify: `docs/design/executive/daemon.md`

### Step 9.1: Define configuration types

- [ ] Add:

```rust
#[derive(Debug, Clone, Default, serde::Deserialize)]
pub struct TelegramConfig {
    #[serde(default)]
    pub enabled: bool,
    pub bot_token_env: Option<String>,
    pub owner_user_id: Option<i64>,
    #[serde(default = "default_poll_timeout_secs")]
    pub poll_timeout_secs: u64,
}
```

Default behavior is disabled. The config stores the environment-variable name, never the token.

### Step 9.2: Add validation tests

- [ ] Test:

- disabled config needs no token or owner;
- enabled config requires non-empty `bot_token_env`;
- enabled config requires `owner_user_id`;
- missing environment variable fails startup with the variable name but not a secret value;
- poll timeout is bounded to 1–50 seconds.

### Step 9.3: Propagate config

- [ ] Add `telegram: TelegramConfig` to `cognit::config::AppConfig`, propagate it into `executive::impl::daemon::DaemonConfig`, and construct it in `RuntimeCore::bootstrap()`.
- [ ] Keep all existing call sites compiling by supplying the default disabled configuration in tests.
- [ ] Run:

```bash
cargo test -p executive -- core::config
cargo check -p executive
```

- [ ] Commit with subject `feat(executive): configure owner-only Telegram channel`.

## 11. Task 10 — Wire Telegram into daemon lifecycle

**Files:**

- Modify: `crates/executive/src/impl/daemon/handler/init.rs`
- Modify: `crates/executive/src/impl/daemon/handler/mod.rs`
- Create: `crates/executive/src/impl/channel/daemon_adapter.rs`

### Step 10.1: Implement the production turn adapter

- [ ] Wrap `Arc<DaemonTurnOrchestrator>`:

```rust
pub struct DaemonChannelTurnExecutor {
    orchestrator: Arc<DaemonTurnOrchestrator>,
}
```

Its `execute()` calls:

```rust
let response = self.orchestrator
    .execute_turn(serde_json::Value::String(correlation_id.to_string()), message)
    .await;
```

Extract the assistant text using the same JSON-RPC result shape asserted by existing daemon turn tests. Do not invent a second `TurnRequest` builder.

### Step 10.2: Initialize after orchestrator construction

- [ ] During handler initialization:

1. Open `${data_dir}/channels.db`.
2. Upsert the configured owner binding `("telegram", owner_user_id, "owner")`.
3. Resolve bot token from the configured environment variable.
4. Build `TelegramTransport`, `ChannelRouter`, and the daemon adapter.
5. Spawn one cancellation-aware poll loop only when Telegram is enabled.
6. Store its `JoinHandle` or `JoinSet` with an owner that participates in daemon shutdown.

### Step 10.3: Poll-loop behavior

- [ ] Implement:

```text
read durable cursor
receive updates
process each update in update_id order
on success reset backoff
on provider/network error sleep with jittered 1s, 2s, 4s ... 60s backoff
on cancellation stop receiving, finish the current store transaction, exit
```

Do not process multiple updates from the same Telegram chat concurrently in M1.

### Step 10.4: Add disabled/enabled lifecycle tests

- [ ] Verify disabled Telegram opens no network connection.
- [ ] Verify enabled Telegram seeds owner binding and resumes stored cursor.
- [ ] Verify cancellation terminates the loop within five seconds with no orphan task.

### Step 10.5: Validate and commit

- [ ] Run:

```bash
cargo test -p executive -- impl::channel
cargo test -p executive --test channel_router
cargo check -p executive
```

Expected: all exit 0.

- [ ] Commit with subject `feat(executive): run Telegram channel with daemon` and describe startup/shutdown ownership.

## 12. Task 11 — Restart and outbox recovery

**Files:**

- Modify: `crates/executive/src/impl/channel/router.rs`
- Modify: `crates/executive/src/impl/channel/store.rs`
- Create: `crates/executive/tests/telegram_restart_recovery.rs`

### Step 11.1: Test crash boundaries

- [ ] Add deterministic tests for:

1. Crash after inbox insert but before turn execution: restart processes the pending inbox once.
2. Crash after turn/outbox commit but before Telegram send: restart sends outbox without re-running the turn.
3. Crash after Telegram send but before marking sent: retry may duplicate the outbound reply, but never duplicates the LLM turn; document this provider limitation.
4. Duplicate update after completed inbox: no turn and no second outbox insert.
5. Unknown sender replay: remains rejected and never reaches the executor.

### Step 11.2: Implement recovery entry points

- [ ] Add:

```rust
pub async fn recover_pending_inbox(&self, limit: usize) -> anyhow::Result<usize>;
pub async fn flush_pending_outbox(&self, limit: usize) -> anyhow::Result<usize>;
```

Run both before the first `getUpdates` call. Bound each startup batch; continue remaining recovery through the normal loop.

### Step 11.3: Validate and commit

- [ ] Run `cargo test -p executive --test telegram_restart_recovery`.
- [ ] Commit with subject `fix(executive): recover interrupted Telegram delivery` and list the documented at-least-once boundary.

## 13. Task 12 — M0/M1 final verification

### Step 12.1: Deterministic checks

- [ ] Run:

```bash
cargo fmt --all -- --check
cargo test -p fabric -- types::channel
cargo test -p executive -- impl::goal
cargo test -p executive -- impl::channel
cargo test -p executive --test agent_integration_baseline
cargo test -p executive --test channel_router
cargo test -p executive --test telegram_restart_recovery
cargo test --workspace
cargo build --workspace
```

Expected: every command exits 0.

### Step 12.2: Security inspection

- [ ] Run:

```bash
rg -n "bot_token|TelegramConfig|telegram" crates/executive/src
```

Confirm:

- no token literal or token value is logged;
- unknown users are rejected before `ChannelTurnExecutor::execute()`;
- Telegram is disabled by default;
- no public listener or webhook was added;
- no Goal-specific `ProcessState` variant was added;
- `ObjectiveStore` and its existing recovery tests remain intact.

### Step 12.3: Manual smoke test

- [ ] With a test bot and owner ID configured outside the repository:

1. Start daemon.
2. Owner sends `/start`; receives greeting.
3. Owner sends `/chat reply with pong`; receives response.
4. Unknown account sends text; receives rejection and causes no LLM event.
5. Stop daemon immediately after an update arrives, restart, and confirm at most one LLM turn occurred.
6. Send `/goal test`; receive the M2-unavailable response.

### Step 12.4: Final commit or handoff

- [ ] If verification required fixes, commit them separately with a conventional subject and problem/solution body.
- [ ] Produce a handoff containing `STATUS`, `SUMMARY`, `CHANGED_FILES`, and `FAILURES` for the reviewer.

## 14. DeepSeek execution guardrails

Give DeepSeek exactly one numbered task at a time with these rules:

```text
Write scope: only files listed in the current task.
Blocked actions: no git reset/checkout, no dependency installation, no network calls,
no edits to ProcessState or ObjectiveStore unless the task explicitly lists them.
Method: write the named failing test first, run it, implement the minimum change,
run the scoped test, inspect git diff, then stop.
Evidence: return command, exit code, changed files, and any remaining failure.
```

Do not ask DeepSeek to implement the entire document in one run. Recommended batches are Tasks 1–2, 3–5, 6–7, 8–9, 10–11, then Task 12 review.
