//! Integration tests for [`ChannelDispatcher`]: durable routing with rejection
//! before LLM and outbox-only retry on send failure.

use std::sync::Arc;

use executive::r#impl::channel::dispatcher::{
    ChannelDispatcher, ChannelTransport, ChannelTurnExecutor, ProviderEnvelope,
};
use executive::r#impl::channel::store::ChannelStore;
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

/// Fake transport that records sent outbound messages.
struct FakeTransport {
    sent: Mutex<Vec<OutboundMessage>>,
}

impl FakeTransport {
    fn new() -> Self {
        Self {
            sent: Mutex::new(Vec::new()),
        }
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
        self.sent.lock().await.push(message.clone());
        Ok("fake-provider-msg-id".into())
    }
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Create an inbound text message for testing.
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

/// Create a command message from the owner.
fn owner_command(
    message_id: &str,
    correlation_id: &str,
    command: &str,
    args: &str,
) -> InboundMessage {
    make_inbound(
        message_id,
        "owner",
        correlation_id,
        MessageContent::Command {
            command: command.into(),
            args: args.into(),
        },
    )
}

/// Set up a test fixture with a bound owner and a store on a temp dir.
/// Returns the router, transport, store path, and the stored turn executor.
async fn setup() -> (
    ChannelDispatcher,
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
    let router = ChannelDispatcher::new(store, executor.clone());
    (router, executor, transport, dir)
}

/// Open a fresh read-only store on the same DB to inspect state after
/// the router has processed messages.
fn inspect(path: &std::path::Path) -> ChannelStore {
    ChannelStore::open(path).unwrap()
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

/// Unknown sender is marked rejected; executor is never called.
#[tokio::test]
async fn unknown_sender_rejected_no_executor_call() {
    let (mut router, executor, transport, dir) = setup().await;
    let db_path = dir.path().join("channels.db");

    let msg = make_inbound(
        "1",
        "stranger",
        "corr-unknown",
        MessageContent::Text {
            text: "hello".into(),
        },
    );
    let envelope = ProviderEnvelope {
        message: msg,
        next_cursor: "cursor-1".into(),
    };

    router.process(&transport, envelope).await.unwrap();

    // Executor must not have been called.
    assert!(
        executor.calls.lock().await.is_empty(),
        "executor should have zero calls for unknown sender"
    );

    // Inbox is marked rejected.
    let store = inspect(&db_path);
    assert_eq!(
        store.inbox_status("telegram", "1").unwrap().as_deref(),
        Some("rejected")
    );

    // Cursor was advanced.
    assert_eq!(
        store.cursor("telegram").unwrap().as_deref(),
        Some("cursor-1")
    );
}

/// Owner plain text invokes the executor exactly once.
#[tokio::test]
async fn owner_text_invokes_executor_once() {
    let (mut router, executor, transport, dir) = setup().await;
    let db_path = dir.path().join("channels.db");

    let msg = owner_text("2", "corr-text", "hello world");
    let envelope = ProviderEnvelope {
        message: msg,
        next_cursor: "cursor-2".into(),
    };

    router.process(&transport, envelope).await.unwrap();

    // Executor called once with the correct text.
    let calls = executor.calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0], "hello world");

    // Inbox completed.
    let store = inspect(&db_path);
    assert_eq!(
        store.inbox_status("telegram", "2").unwrap().as_deref(),
        Some("completed")
    );

    // Cursor advanced.
    assert_eq!(
        store.cursor("telegram").unwrap().as_deref(),
        Some("cursor-2")
    );
}

/// Replaying the same message is a duplicate — zero additional executor calls.
#[tokio::test]
async fn replay_same_message_no_extra_executor_call() {
    let (mut router, executor, transport, _dir) = setup().await;

    let msg = owner_text("3", "corr-replay", "first attempt");
    let envelope = ProviderEnvelope {
        message: msg.clone(),
        next_cursor: "cursor-3a".into(),
    };

    router.process(&transport, envelope).await.unwrap();
    assert_eq!(executor.calls.lock().await.len(), 1);

    // Replay with a different cursor — should be skipped.
    let replay = ProviderEnvelope {
        message: msg,
        next_cursor: "cursor-3b".into(),
    };
    router.process(&transport, replay).await.unwrap();

    // Still exactly one executor call.
    assert_eq!(executor.calls.lock().await.len(), 1);
}

/// `/start` returns a greeting without invoking the executor.
#[tokio::test]
async fn start_command_greeting_no_executor() {
    let (mut router, executor, transport, dir) = setup().await;
    let db_path = dir.path().join("channels.db");

    let msg = owner_command("4", "corr-start", "/start", "");
    let envelope = ProviderEnvelope {
        message: msg,
        next_cursor: "cursor-4".into(),
    };

    router.process(&transport, envelope).await.unwrap();

    // Executor must not be called for /start.
    assert!(
        executor.calls.lock().await.is_empty(),
        "/start should not invoke the executor"
    );

    // Inbox completed.
    let store = inspect(&db_path);
    assert_eq!(
        store.inbox_status("telegram", "4").unwrap().as_deref(),
        Some("completed")
    );

    // Outbound was sent through transport.
    let sent = transport.sent.lock().await;
    assert_eq!(sent.len(), 1, "expected one outbound message to be sent");
    assert!(
        sent[0].correlation_id.contains("corr-start"),
        "expected outbound correlation_id to contain corr-start"
    );
}

/// `/goal example` returns the M2-unavailable response without creating an
/// objective (no executor call).
#[tokio::test]
async fn goal_command_without_goal_executor_is_stable_no_turn() {
    let (mut router, executor, transport, dir) = setup().await;
    let db_path = dir.path().join("channels.db");

    let msg = owner_command("5", "corr-goal", "/goal", "example");
    let envelope = ProviderEnvelope {
        message: msg,
        next_cursor: "cursor-5".into(),
    };

    router.process(&transport, envelope).await.unwrap();

    // No executor invocation for /goal.
    assert!(
        executor.calls.lock().await.is_empty(),
        "/goal should not invoke the executor"
    );

    // Inbox completed.
    let store = inspect(&db_path);
    assert_eq!(
        store.inbox_status("telegram", "5").unwrap().as_deref(),
        Some("completed")
    );

    // Outbound reports the missing Goal runtime without invoking chat.
    let sent = transport.sent.lock().await;
    assert_eq!(sent.len(), 1, "expected one outbound message to be sent");
    let text = match &sent[0].content {
        MessageContent::Text { text } => text.clone(),
        _ => String::new(),
    };
    assert!(
        text.contains("Goal runtime is not configured"),
        "expected stable configuration error, got: {text}"
    );
}

/// Executor failure leaves the inbox retryable and does not advance the
/// cursor.
#[tokio::test]
async fn executor_failure_inbox_retryable_cursor_unchanged() {
    let (_router, _executor, transport, dir) = setup().await;
    let db_path = dir.path().join("channels.db");

    // Only this test: point executor to the fail-mode instance.
    struct FailingExecutor;
    #[async_trait::async_trait]
    impl ChannelTurnExecutor for FailingExecutor {
        async fn execute(
            &self,
            _principal: &str,
            _message: &str,
            _correlation_id: &str,
        ) -> anyhow::Result<String> {
            anyhow::bail!("simulated ai failure")
        }
    }
    let failing_executor = Arc::new(FailingExecutor);
    let store = ChannelStore::open(&db_path).unwrap();
    let mut router = ChannelDispatcher::new(store, failing_executor);

    let msg = owner_text("6", "corr-fail", "trigger failure");
    let envelope = ProviderEnvelope {
        message: msg,
        next_cursor: "cursor-6".into(),
    };

    let result = router.process(&transport, envelope).await;
    assert!(
        result.is_err(),
        "process should return error on executor failure"
    );

    // Cursor must not have advanced (the failing test store is separate).
    let store = inspect(&db_path);
    assert_eq!(
        store.cursor("telegram").unwrap().as_deref(),
        None,
        "cursor must not advance on executor failure"
    );

    // Inbox status is 'failed' (retryable).
    assert_eq!(
        store.inbox_status("telegram", "6").unwrap().as_deref(),
        Some("failed")
    );
}
