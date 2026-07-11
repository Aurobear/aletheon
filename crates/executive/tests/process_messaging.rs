//! Inter-process messaging integration tests (Phase 4C).
//!
//! Validates that the ProcessTable + MailboxService + EnvelopeV2 stack works
//! end-to-end: spawning processes with mailboxes, routing messages between them,
//! and handling edge cases (unknown targets, backpressure, closed mailboxes).

use executive::kernel::chronos::TestClock;
use executive::kernel::process::ProcessTable;
use fabric::ipc::envelope_v2::{DeliveryPattern, EnvelopeV2, SchemaId, Target};
use fabric::ipc::mailbox::{
    DeliveryReceipt, InProcessMailbox, InProcessMailboxService, Mailbox, MailboxService,
};
use fabric::types::process::{NamespaceId, SpawnSpec};
use fabric::{ProcessManager, ProcessSignal};
use std::sync::Arc;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_test_envelope(source: &str, target: &str, body: &str) -> EnvelopeV2 {
    EnvelopeV2::new(
        SchemaId::from("aletheon.test.process-msg/v1"),
        Target::from(source),
        Target::from(target),
        DeliveryPattern::Direct,
        NamespaceId("process-messaging-test".into()),
        serde_json::json!({"body": body}),
    )
}

/// Spawn a process in the process table and register its mailbox.
async fn spawn_with_mailbox(
    table: &ProcessTable,
    svc: &InProcessMailboxService,
    agent_name: &str,
    namespace: &str,
) -> (fabric::ProcessId, Arc<dyn Mailbox>) {
    let handle = table
        .spawn(SpawnSpec {
            namespace: NamespaceId(namespace.to_string()),
            ..SpawnSpec::default()
        })
        .await
        .unwrap();
    let pid = handle.id;

    let mailbox: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::with_capacity(64));
    svc.register(Target::from(agent_name), mailbox.clone())
        .await
        .unwrap();

    (pid, mailbox)
}

// ---------------------------------------------------------------------------
// Single-process messaging
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spawned_process_has_mailbox_registered() {
    let clock = Arc::new(TestClock::default());
    let table = ProcessTable::new(clock);
    let svc = InProcessMailboxService::new();

    let (_pid, _mb) = spawn_with_mailbox(&table, &svc, "agent-1", "ns-1").await;
    assert_eq!(svc.len().await, 1);
}

#[tokio::test]
async fn route_between_two_processes() {
    let clock = Arc::new(TestClock::default());
    let table = ProcessTable::new(clock);
    let svc = InProcessMailboxService::new();

    let (_pid_a, _mb_a) = spawn_with_mailbox(&table, &svc, "alice", "test-ns").await;
    let (_pid_b, mb_b) = spawn_with_mailbox(&table, &svc, "bob", "test-ns").await;

    // Alice sends to Bob.
    let env = make_test_envelope("alice", "bob", "hello bob");
    let receipt = svc.route(env).await;
    assert!(receipt.is_ok());

    // Bob receives.
    let received = mb_b.recv().await.expect("bob should receive message");
    assert_eq!(received.source, Target::from("alice"));
    assert_eq!(received.payload["body"], "hello bob");
}

// ---------------------------------------------------------------------------
// Process lifecycle + messaging
// ---------------------------------------------------------------------------

#[tokio::test]
async fn message_to_terminated_process_still_delivered_if_mailbox_open() {
    let clock = Arc::new(TestClock::default());
    let table = ProcessTable::new(clock);
    let svc = InProcessMailboxService::new();

    let (pid, mb) = spawn_with_mailbox(&table, &svc, "short-lived", "test-ns").await;

    // Terminate the process.
    table.signal(pid, ProcessSignal::Terminate).await.unwrap();

    // Mailbox is still open — send should succeed.
    let env = make_test_envelope("kernel", "short-lived", "farewell");
    let receipt = svc.route(env).await;
    assert!(
        receipt.is_ok(),
        "delivery to terminated process should succeed: {receipt:?}"
    );

    let received = mb.recv().await.expect("mailbox still has pending messages");
    assert_eq!(received.payload["body"], "farewell");
}

#[tokio::test]
async fn unregistered_mailbox_returns_no_such_mailbox() {
    let svc = InProcessMailboxService::new();
    let env = make_test_envelope("kernel", "nonexistent", "hello?");
    let receipt = svc.route(env).await;
    assert!(
        matches!(receipt, DeliveryReceipt::NoSuchMailbox { .. }),
        "expected NoSuchMailbox, got {receipt:?}"
    );
}

// ---------------------------------------------------------------------------
// Backpressure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_mailbox_rejects_with_backpressure() {
    let svc = InProcessMailboxService::new();
    let mb: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::with_capacity(1));
    svc.register(Target::from("full-box"), mb.clone())
        .await
        .unwrap();

    // Fill the buffer.
    let r1 = svc
        .route(make_test_envelope("kernel", "full-box", "msg-1"))
        .await;
    assert!(r1.is_ok());

    // Buffer full — no consumer.
    let r2 = svc
        .route(make_test_envelope("kernel", "full-box", "msg-2"))
        .await;
    assert!(
        matches!(r2, DeliveryReceipt::Rejected { .. }),
        "expected Rejected, got {r2:?}"
    );

    // Drain and retry.
    let _ = mb.recv().await;
    let r3 = svc
        .route(make_test_envelope("kernel", "full-box", "msg-3"))
        .await;
    assert!(r3.is_ok());
}

// ---------------------------------------------------------------------------
// Multi-process fan-out
// ---------------------------------------------------------------------------

#[tokio::test]
async fn fan_out_to_multiple_processes() {
    let clock = Arc::new(TestClock::default());
    let table = ProcessTable::new(clock);
    let svc = InProcessMailboxService::new();

    let n = 5;
    let mut mailboxes = vec![];
    for i in 0..n {
        let name = format!("worker-{i}");
        let (_pid, mb) = spawn_with_mailbox(&table, &svc, &name, "fanout-ns").await;
        mailboxes.push(mb);
    }

    // Send one message to each worker.
    for i in 0..n {
        let env = make_test_envelope("dispatcher", &format!("worker-{i}"), &format!("job-{i}"));
        let receipt = svc.route(env).await;
        assert!(receipt.is_ok(), "route to worker-{i} failed: {receipt:?}");
    }

    // Each worker receives exactly one message.
    for (i, mb) in mailboxes.iter().enumerate() {
        let msg = mb
            .recv()
            .await
            .unwrap_or_else(|| panic!("worker-{i} should have a message"));
        assert_eq!(msg.payload["body"], format!("job-{i}"));
    }
}

// ---------------------------------------------------------------------------
// Envelope metadata preservation
// ---------------------------------------------------------------------------

#[tokio::test]
async fn envelope_metadata_preserved_across_route() {
    let svc = InProcessMailboxService::new();
    let mb: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::with_capacity(8));
    svc.register(Target::from("receiver"), mb.clone())
        .await
        .unwrap();

    let env = EnvelopeV2::new(
        SchemaId::from("aletheon.turn.request/v1"),
        Target::from("executive"),
        Target::from("receiver"),
        DeliveryPattern::RequestResponse,
        NamespaceId("meta-test".into()),
        serde_json::json!({"prompt": "test"}),
    )
    .with_priority(200)
    .with_logical_time(42);

    let original_id = env.id;
    let original_schema = env.schema.clone();

    svc.route(env).await;

    let received = mb.recv().await.expect("should receive");
    assert_eq!(received.id, original_id);
    assert_eq!(received.schema, original_schema);
    assert_eq!(received.priority, 200);
    assert_eq!(received.logical_time, 42);
    assert_eq!(received.namespace, NamespaceId("meta-test".into()));
    assert_eq!(received.pattern, DeliveryPattern::RequestResponse);
}

// ---------------------------------------------------------------------------
// Error handling: duplicate registration
// ---------------------------------------------------------------------------

#[tokio::test]
async fn duplicate_mailbox_registration_is_rejected() {
    let svc = InProcessMailboxService::new();
    let mb1: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());
    let mb2: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());

    svc.register(Target::from("dup"), mb1).await.unwrap();
    let err = svc.register(Target::from("dup"), mb2).await;
    assert!(err.is_err(), "duplicate registration must fail");
    assert!(err.unwrap_err().to_string().contains("already registered"));
}
