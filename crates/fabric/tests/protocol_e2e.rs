// crates/aletheon-comm/tests/protocol_e2e.rs

//! End-to-end tests for the communication protocol stack.

use std::sync::Arc;
use std::time::Duration;

use fabric::envelope::*;
use fabric::events::types::Priority;

use fabric::CommunicationBus;

#[tokio::test]
async fn test_point_to_point_request_response() {
    let bus = Arc::new(CommunicationBus::new());

    // Register a "SelfField" module that responds to requests
    let mut rx = bus.register_module(ModuleId::Dasein, Some(16));

    // Spawn a responder task: receives request from mailbox, sends response via bus
    let bus_clone = bus.clone();
    let responder = tokio::spawn(async move {
        if let Some(envelope) = rx.recv().await {
            let response = Envelope::response(
                &envelope,
                Payload::Json(serde_json::json!({
                    "verdict": "Allow"
                })),
            );
            bus_clone.send(response).await.unwrap();
        }
    });

    // Send a request via the protocol layer (blocks until response arrives)
    let request = Envelope::request(
        Endpoint::Module(ModuleId::Cognit),
        Target::Module(ModuleId::Dasein),
        Payload::Json(serde_json::json!({
            "intent": "execute_tool",
            "tool": "bash"
        })),
        Duration::from_secs(5),
    );

    let response = bus
        .request(request)
        .await
        .expect("request should get a response");

    // Verify the response payload
    if let Payload::Json(json) = &response.payload {
        assert_eq!(json["verdict"], "Allow");
    } else {
        panic!("expected JSON payload in response");
    }

    // Wait for responder to finish
    responder.await.unwrap();
}

#[tokio::test]
async fn test_topic_publish_subscribe() {
    let bus = CommunicationBus::new();

    // Subscribe to a topic
    let mut sub1 = bus.subscribe_topic("tool.observation", Some(16));
    let mut sub2 = bus.subscribe_topic("tool.observation", Some(16));

    // Publish to the topic
    let envelope = Envelope::publish(
        Endpoint::Module(ModuleId::Executive),
        "tool.observation",
        Payload::Json(serde_json::json!({
            "tool": "bash",
            "exit_code": 0
        })),
    );
    bus.publish(envelope).await.unwrap();

    // Both subscribers should receive
    let msg1 = tokio::time::timeout(Duration::from_millis(100), sub1.recv()).await;
    let msg2 = tokio::time::timeout(Duration::from_millis(100), sub2.recv()).await;

    assert!(msg1.is_ok(), "subscriber 1 should receive");
    assert!(msg2.is_ok(), "subscriber 2 should receive");

    let msg1 = msg1.unwrap().unwrap();
    let msg2 = msg2.unwrap().unwrap();

    assert_eq!(msg1.id, msg2.id, "both should receive the same envelope");
}

#[tokio::test]
async fn test_request_timeout() {
    let bus = CommunicationBus::new();

    // Send a request to a module that has no handler
    let request = Envelope::request(
        Endpoint::Module(ModuleId::Cognit),
        Target::Module(ModuleId::Dasein), // No one is listening
        Payload::Json(serde_json::json!({"test": true})),
        Duration::from_millis(100), // Short timeout
    );

    let result = bus.request(request).await;
    assert!(result.is_err(), "request should timeout");

    let err = result.unwrap_err().to_string();
    assert!(
        err.contains("timed out") || err.contains("closed"),
        "error should indicate timeout: {err}"
    );
}

#[tokio::test]
async fn test_concurrent_request_response() {
    let bus = Arc::new(CommunicationBus::new());

    // Register a responder module
    let mut rx = bus.register_module(ModuleId::Dasein, Some(64));

    // Spawn a responder that handles exactly 10 requests then exits
    let bus_clone = bus.clone();
    let responder = tokio::spawn(async move {
        for _ in 0..10 {
            if let Some(envelope) = rx.recv().await {
                let bus = bus_clone.clone();
                tokio::spawn(async move {
                    let response = Envelope::response(
                        &envelope,
                        Payload::Json(serde_json::json!({
                            "id": envelope.id
                        })),
                    );
                    bus.send(response).await.unwrap();
                });
            }
        }
    });

    // Fire 10 concurrent requests
    let mut handles = Vec::new();
    for _ in 0..10 {
        let bus = bus.clone();
        handles.push(tokio::spawn(async move {
            let request = Envelope::request(
                Endpoint::Module(ModuleId::Cognit),
                Target::Module(ModuleId::Dasein),
                Payload::Json(serde_json::json!({"ping": true})),
                Duration::from_secs(5),
            );
            let request_id = request.id;
            let response = bus.request(request).await.expect("request should succeed");
            if let Payload::Json(json) = &response.payload {
                assert_eq!(
                    json["id"], request_id,
                    "response correlation_id should match request id"
                );
            }
        }));
    }

    for handle in handles {
        handle.await.unwrap();
    }

    responder.await.unwrap();
}

#[tokio::test]
async fn test_fire_and_forget() {
    let bus = CommunicationBus::new();

    let mut rx = bus.register_module(ModuleId::Mnemosyne, Some(16));

    let envelope = Envelope::fire_and_forget(
        Endpoint::Module(ModuleId::Cognit),
        Target::Module(ModuleId::Mnemosyne),
        Payload::Json(serde_json::json!({"store": "episodic"})),
    );

    bus.send(envelope).await.unwrap();

    let received = tokio::time::timeout(Duration::from_millis(100), rx.recv()).await;
    assert!(received.is_ok(), "should receive fire-and-forget");
}

#[tokio::test]
async fn test_envelope_ttl_expiry() {
    let mut envelope = Envelope::new(
        Endpoint::System,
        Target::Broadcast,
        Pattern::FireAndForget,
        Payload::Empty,
    );
    // Set TTL to 0ms (already expired)
    envelope.ttl_ms = Some(0);
    envelope.timestamp_ms = 0; // Force epoch

    assert!(
        envelope.is_expired(),
        "envelope with 0ms TTL at epoch should be expired"
    );

    // Fresh envelope with no TTL should not be expired
    let fresh = Envelope::new(
        Endpoint::System,
        Target::Broadcast,
        Pattern::FireAndForget,
        Payload::Empty,
    );
    assert!(
        !fresh.is_expired(),
        "fresh envelope with no TTL should not be expired"
    );
}

#[tokio::test]
async fn test_priority_ordering() {
    assert!(Priority::Critical < Priority::High);
    assert!(Priority::High < Priority::Normal);
    assert!(Priority::Normal < Priority::Low);
    assert!(Priority::Low < Priority::Background);
}
