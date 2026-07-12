//! Integration tests for MailboxService V2.
//!
//! These tests exercise the full mailbox lifecycle across multiple async tasks,
//! simulating inter-process messaging patterns.

use fabric::ipc::envelope_v2::{DeliveryPattern, EnvelopeV2, SchemaId, Target};
use fabric::ipc::mailbox::{
    DeliveryReceipt, InProcessMailbox, InProcessMailboxService, Mailbox, MailboxService,
};
use fabric::types::process::NamespaceId;
use std::sync::Arc;

fn make_envelope(source: &str, target: &str, payload: &str) -> EnvelopeV2 {
    EnvelopeV2::new(
        SchemaId::from("aletheon.test/v1"),
        Target::from(source),
        Target::from(target),
        DeliveryPattern::Direct,
        NamespaceId("integration-test".into()),
        serde_json::json!({"body": payload}),
    )
}

// ---------------------------------------------------------------------------
// Send / Recv
// ---------------------------------------------------------------------------

#[tokio::test]
async fn concurrent_send_recv_multiple_senders() {
    let mb = Arc::new(InProcessMailbox::with_capacity(32));
    let mb_recv = mb.clone();

    // Spawn 3 senders that each send 10 messages.
    let mut handles = vec![];
    for sender_id in 0..3 {
        let tx = mb.clone();
        handles.push(tokio::spawn(async move {
            for i in 0..10 {
                let env = make_envelope(
                    &format!("sender-{sender_id}"),
                    "receiver",
                    &format!("msg-{sender_id}-{i}"),
                );
                tx.send(env).await;
            }
        }));
    }

    // Receiver collects all 30 messages.
    let mut received = 0;
    while received < 30 {
        if mb_recv.recv().await.is_some() {
            received += 1;
        }
    }

    for h in handles {
        h.await.unwrap();
    }
    assert_eq!(received, 30);
}

#[tokio::test]
async fn send_receives_in_fifo_order() {
    let mb = InProcessMailbox::with_capacity(10);
    for i in 0..5 {
        mb.send(make_envelope("src", "dst", &format!("m{i}"))).await;
    }

    for i in 0..5 {
        let env = mb.recv().await.expect("should receive in order");
        assert_eq!(env.payload["body"], format!("m{i}"));
    }
}

// ---------------------------------------------------------------------------
// Backpressure
// ---------------------------------------------------------------------------

#[tokio::test]
async fn backpressure_signal_on_full_buffer() {
    let mb = InProcessMailbox::with_capacity(1);
    let r1 = mb.send(make_envelope("s", "d", "first")).await;
    assert!(r1.is_ok());

    // Second send should be rejected (buffer full, no consumer).
    let r2 = mb.send(make_envelope("s", "d", "second")).await;
    assert!(matches!(r2, DeliveryReceipt::Rejected { .. }));

    // After consuming one, send should succeed again.
    let _ = mb.recv().await;
    let r3 = mb.send(make_envelope("s", "d", "third")).await;
    assert!(r3.is_ok());
}

// ---------------------------------------------------------------------------
// MailboxService routing
// ---------------------------------------------------------------------------

#[tokio::test]
async fn route_delivers_to_registered_target() {
    let svc = InProcessMailboxService::new();
    let alice: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());
    let bob: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());

    svc.register(Target::from("alice"), alice.clone())
        .await
        .unwrap();
    svc.register(Target::from("bob"), bob.clone())
        .await
        .unwrap();

    let env = make_envelope("alice", "bob", "hello");
    let receipt = svc.route(env).await;
    assert!(receipt.is_ok());

    let received = bob.recv().await.expect("bob should receive");
    assert_eq!(received.payload["body"], "hello");
}

#[tokio::test]
async fn route_to_unknown_target() {
    let svc = InProcessMailboxService::new();
    let env = make_envelope("alice", "ghost", "hi");
    let receipt = svc.route(env).await;
    assert!(matches!(receipt, DeliveryReceipt::NoSuchMailbox { .. }));
}

// ---------------------------------------------------------------------------
// Request-Response pattern
// ---------------------------------------------------------------------------

#[tokio::test]
async fn full_request_response_cycle() {
    let svc = Arc::new(InProcessMailboxService::new());
    let alice: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());
    let bob: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());

    svc.register(Target::from("alice"), alice.clone())
        .await
        .unwrap();
    svc.register(Target::from("bob"), bob.clone())
        .await
        .unwrap();

    let svc_clone = svc.clone();
    let bob_clone = bob.clone();

    // Bob: receive request, send response with correlation_id.
    tokio::spawn(async move {
        let req = bob_clone.recv().await.expect("bob should get request");
        let response = EnvelopeV2::new(
            SchemaId::from("aletheon.test/v1"),
            Target::from("bob"),
            req.source.clone(),
            DeliveryPattern::Direct,
            NamespaceId("integration-test".into()),
            serde_json::json!({"reply": "pong"}),
        )
        .with_correlation_id(req.id);
        svc_clone.route(response).await;
    });

    // Alice: send request, wait for response.
    let request = make_envelope("alice", "bob", "ping");
    let request_id = request.id;
    svc.route(request).await;

    // Alice polls for correlated response.
    let response = loop {
        let msg = alice.recv().await.expect("alice should get response");
        if msg.correlation_id == Some(request_id) {
            break msg;
        }
    };

    assert_eq!(response.payload["reply"], "pong");
    assert_eq!(response.correlation_id, Some(request_id));
}

// ---------------------------------------------------------------------------
// Expiry (deadline-based filtering is done by the dispatcher, not the mailbox)
// ---------------------------------------------------------------------------

#[tokio::test]
async fn expired_envelope_still_delivered_by_mailbox() {
    // The mailbox itself does not enforce deadlines — that's the dispatcher's
    // job. Mailbox delivers everything unconditionally.
    let mb = InProcessMailbox::new();
    let env = make_envelope("s", "d", "late");
    mb.send(env).await;
    let received = mb.recv().await;
    assert!(received.is_some());
}

// ---------------------------------------------------------------------------
// Scale: many mailboxes
// ---------------------------------------------------------------------------

#[tokio::test]
async fn many_mailboxes_concurrent_routing() {
    let svc = Arc::new(InProcessMailboxService::new());
    let n = 20;
    let mut mailboxes = vec![];

    for i in 0..n {
        let mb: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::with_capacity(8));
        mailboxes.push(mb.clone());
        svc.register(Target::from(format!("agent-{i}")), mb)
            .await
            .unwrap();
    }

    assert_eq!(svc.len().await, n);

    // Send one message to each.
    for i in 0..n {
        let env = make_envelope("kernel", &format!("agent-{i}"), &format!("to-{i}"));
        let receipt = svc.route(env).await;
        assert!(receipt.is_ok(), "route to agent-{i} failed: {receipt:?}");
    }

    // Each mailbox should have exactly 1 message.
    for (i, mb) in mailboxes.iter().enumerate() {
        let msg = mb
            .recv()
            .await
            .expect(&format!("agent-{i} should have a message"));
        assert_eq!(msg.payload["body"], format!("to-{i}"));
    }
}

#[tokio::test]
async fn route_at_rejects_expired_envelope() {
    let svc = InProcessMailboxService::new();
    let mb: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());
    svc.register(Target::from("dst"), mb).await.unwrap();
    let env = make_envelope("src", "dst", "late").with_deadline(fabric::MonoDeadlineMillis(10));

    let receipt = svc.route_at(env, 10).await;
    assert!(matches!(receipt, DeliveryReceipt::Expired { .. }));
}

#[tokio::test]
async fn unknown_schema_rejected_structurally() {
    let svc = InProcessMailboxService::new();
    let mb: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::new());
    svc.register(Target::from("dst"), mb).await.unwrap();
    let env = EnvelopeV2::new(
        SchemaId::from("aletheon.unknown/v9"),
        Target::from("src"),
        Target::from("dst"),
        DeliveryPattern::Direct,
        NamespaceId("integration-test".into()),
        serde_json::json!({}),
    );
    let receipt = svc.route_at(env, 0).await;
    assert!(
        matches!(receipt, DeliveryReceipt::Rejected { reason, .. } if reason.contains("unsupported schema"))
    );
}

#[tokio::test]
async fn process_signal_has_priority_over_ordinary_messages() {
    let svc = InProcessMailboxService::new();
    let mb: Arc<dyn Mailbox> = Arc::new(InProcessMailbox::with_capacity(4));
    svc.register(Target::from("agent"), mb.clone())
        .await
        .unwrap();

    svc.route(make_envelope("src", "agent", "ordinary")).await;
    let signal_receipt = svc
        .signal_process(Target::from("agent"), fabric::ProcessSignal::Terminate)
        .await;
    assert!(signal_receipt.is_ok());

    let first = mb.recv().await.unwrap();
    assert_eq!(first.schema.0, SchemaId::PROCESS_SIGNAL_V1);
    let second = mb.recv().await.unwrap();
    assert_eq!(second.payload["body"], "ordinary");
}
