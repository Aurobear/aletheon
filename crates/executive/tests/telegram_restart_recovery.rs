//! Restart recovery tests for [`ChannelRouter`]: crash boundaries and
//! at-least-once delivery semantics.
//!
//! These tests simulate crash scenarios to verify that:
//! 1. Pending inbox messages are processed after restart.
//! 2. Pending outbox messages are sent without re-running the LLM turn.
//! 3. Duplicate provider updates do not cause double execution.
//! 4. Unknown senders are consistently rejected.
//! 5. The at-least-once outbound boundary is documented.

use std::sync::Arc;

use executive::r#impl::channel::router::{
    ChannelRouter, ChannelTransport, ChannelTurnExecutor, ProviderEnvelope,
};
use executive::r#impl::channel::store::{ChannelStore, InsertOutcome};
use fabric::channel::{
    ChannelId, ConversationId, ExternalSenderId, InboundMessage, MessageContent, MessageId,
    OutboundMessage,
};
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Fake implementations
// ---------------------------------------------------------------------------

/// Fake turn executor that records every call and returns a prefixed echo.
#[derive(Default)]
struct FakeTurnExecutor {
    calls: Mutex<Vec<String>>,
}

#[async_trait::async_trait]
impl ChannelTurnExecutor for FakeTurnExecutor {
    async fn execute(
        &self,
        _principal: &str,
        message: &str,
        _correlation_id: &str,
    ) -> anyhow::Result<String> {
        self.calls.lock().await.push(message.to_string());
        Ok(format!("reply:{}", message))
    }
}

/// Fake transport that records sent outbound messages, with an optional
/// per-call failure toggle.
struct FakeTransport {
    sent: Mutex<Vec<OutboundMessage>>,
    /// When `true`, the next `send()` call returns an error and clears
    /// the flag. When `false`, `send()` succeeds.
    fail_next: Mutex<bool>,
}

impl FakeTransport {
    fn new() -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
            fail_next: Mutex::new(false),
        }
    }

    async fn set_fail_next(&self, fail: bool) {
        *self.fail_next.lock().await = fail;
    }
}

#[async_trait::async_trait]
impl ChannelTransport for FakeTransport {
    fn channel_id(&self) -> &str {
        "telegram"
    }

    async fn receive(&self, _cursor: Option<String>) -> anyhow::Result<Vec<ProviderEnvelope>> {
        Ok(vec![])
    }

    async fn send(&self, message: &OutboundMessage) -> anyhow::Result<String> {
        let should_fail = *self.fail_next.lock().await;
        if should_fail {
            *self.fail_next.lock().await = false;
            anyhow::bail!("simulated send failure");
        }
        self.sent.lock().await.push(message.clone());
        Ok("fake-provider-msg-id".into())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create an inbound text message.
fn make_inbound(
    message_id: &str,
    sender_id: &str,
    correlation_id: &str,
    content: MessageContent,
) -> InboundMessage {
    InboundMessage {
        channel_id: ChannelId("telegram".into()),
        message_id: MessageId(message_id.into()),
        conversation_id: ConversationId("conv-1".into()),
        sender_id: ExternalSenderId(sender_id.into()),
        content,
        timestamp_ms: 1_720_000_000_000,
        reply_to_action: None,
        correlation_id: correlation_id.into(),
    }
}

/// Create a text message from the owner.
fn owner_text(message_id: &str, correlation_id: &str, text: &str) -> InboundMessage {
    make_inbound(
        message_id,
        "owner",
        correlation_id,
        MessageContent::Text { text: text.into() },
    )
}

/// Set up a test fixture: store with bound owner, executor, and transport.
/// Returns the router, executor, transport, and temp dir.
async fn setup() -> (
    ChannelRouter,
    Arc<FakeTurnExecutor>,
    FakeTransport,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("channels.db");
    let store = ChannelStore::open(&db_path).unwrap();
    store.bind("telegram", "owner", "owner", "active").unwrap();
    let executor = Arc::new(FakeTurnExecutor::default());
    let transport = FakeTransport::new();
    let router = ChannelRouter::new(store, executor.clone());
    (router, executor, transport, dir)
}

/// Open a fresh read-only store on the same DB to inspect state.
fn inspect(path: &std::path::Path) -> ChannelStore {
    ChannelStore::open(path).unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Scenario: crash after inbox insert but before turn execution.
///
/// Simulated by inserting a pending inbox row directly, then invoking
/// `recover_pending_inbox` on a fresh router. Verifies the executor is
/// called, the inbox is completed, and the outbound is sent.
#[tokio::test]
async fn crash_after_inbox_insert_recover_processes_pending() {
    // ---- Simulate crash before processing: raw insert into store ----
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("channels.db");
    let mut store = ChannelStore::open(&db_path).unwrap();
    store.bind("telegram", "owner", "owner", "active").unwrap();
    let msg = owner_text("42", "corr-recover-inbox", "hello from pending");
    let outcome = store.insert_inbound(&msg).unwrap();
    assert_eq!(outcome, InsertOutcome::Inserted);
    // Message is pending; no cursor has been advanced.
    assert_eq!(
        store.inbox_status("telegram", "42").unwrap().as_deref(),
        Some("pending")
    );
    assert_eq!(store.cursor("telegram").unwrap(), None);

    // ---- Restart: create a fresh router on the same store ----
    let store2 = ChannelStore::open(&db_path).unwrap();
    let executor = Arc::new(FakeTurnExecutor::default());
    let transport = FakeTransport::new();
    let mut router = ChannelRouter::new(store2, executor.clone());

    let count = router.recover_pending_inbox(&transport, 10).await.unwrap();
    assert_eq!(count, 1, "should recover exactly one pending message");

    // Executor was called exactly once.
    let calls = executor.calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], "hello from pending");

    // Inbox is now completed.
    let store3 = inspect(&db_path);
    assert_eq!(
        store3.inbox_status("telegram", "42").unwrap().as_deref(),
        Some("completed")
    );

    // Cursor was advanced.
    assert_eq!(store3.cursor("telegram").unwrap().as_deref(), Some("42"));

    // Outbound was sent through transport.
    let sent = transport.sent.lock().await;
    assert_eq!(sent.len(), 1, "expected one outbound message to be sent");
    assert_eq!(
        sent[0].correlation_id, "corr-recover-inbox",
        "outbound should have the correct correlation_id"
    );
}

/// Scenario: crash after turn/outbox commit but before Telegram send.
///
/// The inbox is completed, the outbox row exists with status 'pending', but
/// `transport.send()` never happened. `flush_pending_outbox` must send the
/// outbound without re-running the LLM turn.
#[tokio::test]
async fn crash_after_outbox_commit_flush_sends_without_turn() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("channels.db");

    // ---- Simulate a successful turn that crashed before send ----
    // Process a message normally but use a transport that fails the send.
    do_crash_after_outbox_commit(&db_path).await;

    // ---- Restart: create a fresh router and flush the outbox ----
    let store2 = ChannelStore::open(&db_path).unwrap();
    let executor = Arc::new(FakeTurnExecutor::default());
    let transport = FakeTransport::new();
    let router = ChannelRouter::new(store2, executor.clone());

    let count = router.flush_pending_outbox(&transport, 10).await.unwrap();
    assert_eq!(count, 1, "should flush exactly one pending outbox message");

    // Outbound was sent through transport (now succeeding).
    let sent = transport.sent.lock().await;
    assert_eq!(sent.len(), 1, "expected one outbound message to be sent");
    assert_eq!(
        sent[0].correlation_id, "corr-crash-outbox",
        "outbound should have the correct correlation_id"
    );

    // Executor was NOT called during recovery.
    assert!(
        executor.calls.lock().await.is_empty(),
        "executor must not be called during outbox-only recovery"
    );
}

/// Helper: process a message through a router whose transport fails on send,
/// leaving the inbox completed and the outbox pending.
async fn do_crash_after_outbox_commit(db_path: &std::path::Path) {
    let store = ChannelStore::open(db_path).unwrap();
    store.bind("telegram", "owner", "owner", "active").unwrap();
    let executor = Arc::new(FakeTurnExecutor::default());
    let transport = FakeTransport::new();
    transport.set_fail_next(true).await;
    let mut router = ChannelRouter::new(store, executor.clone());

    let msg = owner_text("99", "corr-crash-outbox", "message before crash");
    let envelope = ProviderEnvelope {
        message: msg,
        next_cursor: "cursor-99".into(),
    };

    router.process(&transport, envelope).await.unwrap();

    // Verify state: inbox completed, outbox in failed state
    // (because transport.send() returned an error).
    let store_inspect = ChannelStore::open(db_path).unwrap();
    assert_eq!(
        store_inspect
            .inbox_status("telegram", "99")
            .unwrap()
            .as_deref(),
        Some("completed")
    );
    // Outbox should exist with 'failed' status due to the failed send.
    // The pending_outbox query now includes 'failed' rows, so it will
    // be picked up by flush_pending_outbox.
}

/// Scenario: crash after Telegram send but before marking sent.
///
/// The outbox row is marked 'pending' even though the send actually succeeded
/// (we simulate by artificially resetting the status). On recovery,
/// `flush_pending_outbox` re-sends, which may duplicate the outbound reply
/// at the provider. The LLM turn is never duplicated because the inbox is
/// already completed.
///
/// This test documents the at-least-once boundary.
#[tokio::test]
async fn crash_after_send_before_mark_retry_may_duplicate_outbound() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("channels.db");

    // Process a message normally (transport succeeds).
    let store = ChannelStore::open(&db_path).unwrap();
    store.bind("telegram", "owner", "owner", "active").unwrap();
    let executor = Arc::new(FakeTurnExecutor::default());
    let transport = FakeTransport::new();
    let mut router = ChannelRouter::new(store, executor.clone());

    let msg = owner_text("50", "corr-send-then-crash", "at least once");
    let envelope = ProviderEnvelope {
        message: msg,
        next_cursor: "cursor-50".into(),
    };

    router.process(&transport, envelope).await.unwrap();

    // Verify normal state: sent 1, inbox completed.
    assert_eq!(transport.sent.lock().await.len(), 1);
    let store_check = inspect(&db_path);
    assert_eq!(
        store_check
            .inbox_status("telegram", "50")
            .unwrap()
            .as_deref(),
        Some("completed")
    );

    // ---- Simulate crash after send but before the status update ----
    // Manually reset the outbox status back to 'pending'.
    store_check
        .set_outbox_status("corr-send-then-crash", "pending")
        .unwrap();

    // ---- Restart: flush pending outbox ----
    let store2 = ChannelStore::open(&db_path).unwrap();
    let executor2 = Arc::new(FakeTurnExecutor::default());
    let transport2 = FakeTransport::new();
    let router2 = ChannelRouter::new(store2, executor2.clone());

    let count = router2.flush_pending_outbox(&transport2, 10).await.unwrap();
    assert_eq!(count, 1, "should flush the artificially-pending outbox");

    // The outbound was sent again — this is the at-least-once boundary.
    let sent2 = transport2.sent.lock().await;
    assert_eq!(sent2.len(), 1, "outbox was re-sent (at-least-once)");

    // Executor was NOT called again — LLM turn is never duplicated.
    assert!(
        executor2.calls.lock().await.is_empty(),
        "executor must not be called during outbox-only recovery"
    );

    // Inbox is still completed (not re-processed).
    let store3 = inspect(&db_path);
    assert_eq!(
        store3.inbox_status("telegram", "50").unwrap().as_deref(),
        Some("completed")
    );
}

/// Scenario: duplicate update after completed inbox.
///
/// If the provider replays an update whose (channel_id, message_id) is
/// already completed, the router silently skips it: no turn execution,
/// no second outbox insert.
#[tokio::test]
async fn duplicate_update_after_completed_inbox_no_turn_no_outbox() {
    let (mut router, executor, transport, dir) = setup().await;
    let db_path = dir.path().join("channels.db");

    let msg = owner_text("10", "corr-dup-after-comp", "first");
    let envelope1 = ProviderEnvelope {
        message: msg.clone(),
        next_cursor: "cursor-10a".into(),
    };

    router.process(&transport, envelope1).await.unwrap();
    assert_eq!(executor.calls.lock().await.len(), 1);

    // Replay the same message.
    let envelope2 = ProviderEnvelope {
        message: msg,
        next_cursor: "cursor-10b".into(),
    };
    router.process(&transport, envelope2).await.unwrap();

    // Still exactly one executor call.
    assert_eq!(
        executor.calls.lock().await.len(),
        1,
        "executor should still have one call after duplicate"
    );

    // Outbox has exactly one row (no duplicate insert due to
    // correlation_id uniqueness constraint with ON CONFLICT DO NOTHING).
    let store = inspect(&db_path);
    assert_eq!(
        store.outbox_count("telegram").unwrap(),
        1,
        "only one outbox row should exist for this correlation_id"
    );
}

/// Scenario: unknown sender replay after restart.
///
/// On recovery, pending inbox messages from unknown senders are rejected
/// and the executor is never invoked.
#[tokio::test]
async fn unknown_sender_recovery_rejected_no_executor() {
    let dir = tempfile::tempdir().unwrap();
    let db_path = dir.path().join("channels.db");

    // Insert a pending message from an unbound sender.
    let mut store = ChannelStore::open(&db_path).unwrap();
    // No binding for "stranger" — only "owner" is bound.
    store.bind("telegram", "owner", "owner", "active").unwrap();
    let msg = make_inbound(
        "77",
        "stranger",
        "corr-stranger",
        MessageContent::Text {
            text: "should be rejected".into(),
        },
    );
    store.insert_inbound(&msg).unwrap();
    assert_eq!(
        store.inbox_status("telegram", "77").unwrap().as_deref(),
        Some("pending")
    );

    // ---- Restart: recover pending inbox ----
    let store2 = ChannelStore::open(&db_path).unwrap();
    let executor = Arc::new(FakeTurnExecutor::default());
    let transport = FakeTransport::new();
    let mut router = ChannelRouter::new(store2, executor.clone());

    let count = router.recover_pending_inbox(&transport, 10).await.unwrap();
    assert_eq!(count, 1, "should process the pending message (reject it)");

    // Executor was never called.
    assert!(
        executor.calls.lock().await.is_empty(),
        "executor must not be called for unknown sender"
    );

    // Inbox is now rejected.
    let store3 = inspect(&db_path);
    assert_eq!(
        store3.inbox_status("telegram", "77").unwrap().as_deref(),
        Some("rejected")
    );

    // Cursor was advanced (so the same message is not re-fetched).
    assert_eq!(store3.cursor("telegram").unwrap().as_deref(), Some("77"));

    // No outbound was sent.
    assert!(
        transport.sent.lock().await.is_empty(),
        "no outbound should be sent for unknown sender"
    );
}
