//! Integration tests for the communication protocol module handlers.
//!
//! Tests Task 11: MemoryModule, BodyModule, SelfFieldModule, and PerceptionModule
//! through the CommunicationBus envelope pipeline.

use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{mpsc, Mutex};

use base::envelope::*;
use base::self_field::{Intent, IntentSource};
use base::envelope::Payload;
use base::CommunicationBus;

use runtime::r#impl::engine::modules::body_module::BodyModule;
use runtime::r#impl::engine::modules::memory_module::MemoryModule;
use runtime::r#impl::engine::modules::perception_module::PerceptionModule;
use runtime::r#impl::engine::modules::self_field_module::SelfFieldModule;
use runtime::r#impl::engine::modules::{
    BodyRequest, BodyResponse, MemoryRequest, MemoryResponse, PerceptionEventMsg, SelfFieldRequest,
    SelfFieldResponse,
};

use corpus::r#impl::tools::ToolRegistry;
use runtime::CoreMemory;
use dasein::core::{SelfField, SelfFieldConfig};
use dasein::r#impl::perception::event::{
    EventCategory, EventData, EventSource, PerceptionEvent, Priority as EventPriority,
};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn make_envelope_request(target_module: ModuleId, payload_json: serde_json::Value) -> Envelope {
    Envelope::request(
        Endpoint::System,
        Target::Module(target_module),
        Payload::Json(payload_json),
        Duration::from_secs(5),
    )
}

fn core_memory_with_blocks() -> CoreMemory {
    let mut core = CoreMemory::new();
    let persona = runtime::r#impl::memory::core_memory::MemoryBlock::new(
        "persona",
        "I am a test agent",
        1024,
    );
    core.set_block(persona);
    let context = runtime::r#impl::memory::core_memory::MemoryBlock::new(
        "context",
        "Running integration tests",
        1024,
    );
    core.set_block(context);
    core
}

// ---------------------------------------------------------------------------
// Test: MemoryModule — FormatForContext request/response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_memory_module_format_for_context() {
    let bus = Arc::new(CommunicationBus::new());

    // Spawn the module in the background.
    let core = Arc::new(Mutex::new(core_memory_with_blocks()));
    let module = MemoryModule::new(core, None);
    let bus_clone = bus.clone();
    tokio::spawn(async move {
        module.run(bus_clone).await;
    });

    // Give the module a moment to register.
    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send a FormatForContext request.
    let request = serde_json::to_value(MemoryRequest::FormatForContext).unwrap();
    let envelope = make_envelope_request(ModuleId::Memory, request);

    let resp_envelope = bus.request(envelope).await.expect("request should succeed");

    let response: MemoryResponse = match &resp_envelope.payload {
        Payload::Json(val) => serde_json::from_value(val.clone()).expect("should deserialize"),
        _ => panic!("expected JSON payload"),
    };

    match response {
        MemoryResponse::ContextFormatted { text } => {
            assert!(
                text.contains("[persona]"),
                "context should contain persona section header"
            );
            assert!(
                text.contains("test agent"),
                "context should contain persona value"
            );
            assert!(
                text.contains("[context]"),
                "context should contain the extra context block"
            );
            assert!(
                text.contains("Running integration tests"),
                "context should contain context value"
            );
        }
        other => panic!("expected ContextFormatted, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Test: BodyModule — Definitions request/response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_body_module_definitions() {
    let bus = Arc::new(CommunicationBus::new());

    // Use the default ToolRegistry which has built-in tools.
    let registry = Arc::new(Mutex::new(ToolRegistry::default()));
    let module = BodyModule::new(registry);
    let bus_clone = bus.clone();
    tokio::spawn(async move {
        module.run(bus_clone).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let request = serde_json::to_value(BodyRequest::Definitions).unwrap();
    let envelope = make_envelope_request(ModuleId::Body, request);

    let resp_envelope = bus.request(envelope).await.expect("request should succeed");

    let response: BodyResponse = match &resp_envelope.payload {
        Payload::Json(val) => serde_json::from_value(val.clone()).expect("should deserialize"),
        _ => panic!("expected JSON payload"),
    };

    match response {
        BodyResponse::Definitions { tools } => {
            assert!(
                !tools.is_empty(),
                "should have at least one tool definition"
            );
            // Verify the first tool has a name and description.
            let first = &tools[0];
            assert!(!first.name.is_empty(), "tool name should not be empty");
            assert!(
                !first.description.is_empty(),
                "tool description should not be empty"
            );
        }
        other => panic!("expected Definitions, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Test: BodyModule — ListTools request/response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_body_module_list_tools() {
    let bus = Arc::new(CommunicationBus::new());

    let registry = Arc::new(Mutex::new(ToolRegistry::default()));
    let module = BodyModule::new(registry);
    let bus_clone = bus.clone();
    tokio::spawn(async move {
        module.run(bus_clone).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let request = serde_json::to_value(BodyRequest::ListTools).unwrap();
    let envelope = make_envelope_request(ModuleId::Body, request);

    let resp_envelope = bus.request(envelope).await.expect("request should succeed");

    let response: BodyResponse = match &resp_envelope.payload {
        Payload::Json(val) => serde_json::from_value(val.clone()).expect("should deserialize"),
        _ => panic!("expected JSON payload"),
    };

    match response {
        BodyResponse::ToolList { names } => {
            assert!(!names.is_empty(), "should have at least one tool name");
        }
        other => panic!("expected ToolList, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Test: SelfFieldModule — Review request/response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_self_field_module_review() {
    let bus = Arc::new(CommunicationBus::new());

    let config = SelfFieldConfig {
        agent_name: "test-agent".to_string(),
        ..Default::default()
    };
    let self_field = Arc::new(Mutex::new(SelfField::new(config)));
    let module = SelfFieldModule::new(self_field);
    let bus_clone = bus.clone();
    tokio::spawn(async move {
        module.run(bus_clone).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let intent = Intent {
        action: "read_file".to_string(),
        parameters: serde_json::json!({ "path": "/tmp/test.txt" }),
        source: IntentSource::User,
        description: "Read a test file".to_string(),
    };
    let ctx = serde_json::json!({
        "session_id": "test-session",
        "working_dir": "/tmp"
    });
    let request = serde_json::to_value(SelfFieldRequest::Review { intent, ctx }).unwrap();
    let envelope = make_envelope_request(ModuleId::SelfField, request);

    let resp_envelope = bus.request(envelope).await.expect("request should succeed");

    let response: SelfFieldResponse = match &resp_envelope.payload {
        Payload::Json(val) => serde_json::from_value(val.clone()).expect("should deserialize"),
        _ => panic!("expected JSON payload"),
    };

    match response {
        SelfFieldResponse::Verdict { verdict } => {
            // A simple read_file intent should get Allow.
            match verdict {
                base::self_field::Verdict::Allow => { /* expected */ }
                other => panic!("expected Allow verdict for read_file, got: {:?}", other),
            }
        }
        other => panic!("expected Verdict, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Test: SelfFieldModule — GetIdentity request/response
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_self_field_module_get_identity() {
    let bus = Arc::new(CommunicationBus::new());

    let config = SelfFieldConfig {
        agent_name: "test-agent".to_string(),
        agent_description: "A test agent".to_string(),
        agent_version: "0.1.0".to_string(),
        ..Default::default()
    };
    let self_field = Arc::new(Mutex::new(SelfField::new(config)));
    let module = SelfFieldModule::new(self_field);
    let bus_clone = bus.clone();
    tokio::spawn(async move {
        module.run(bus_clone).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    let request = serde_json::to_value(SelfFieldRequest::GetIdentity).unwrap();
    let envelope = make_envelope_request(ModuleId::SelfField, request);

    let resp_envelope = bus.request(envelope).await.expect("request should succeed");

    let response: SelfFieldResponse = match &resp_envelope.payload {
        Payload::Json(val) => serde_json::from_value(val.clone()).expect("should deserialize"),
        _ => panic!("expected JSON payload"),
    };

    match response {
        SelfFieldResponse::Identity { identity } => {
            assert_eq!(identity.name, "test-agent");
            assert_eq!(identity.version, "0.1.0");
        }
        other => panic!("expected Identity, got: {:?}", other),
    }
}

// ---------------------------------------------------------------------------
// Test: PerceptionModule — publish events to topic
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_perception_module_publishes_to_topic() {
    let bus = Arc::new(CommunicationBus::new());

    // Subscribe to the perception topic before sending events.
    let mut topic_rx = bus.subscribe_topic("perception.events", Some(16));

    // Create the mpsc channel for perception events.
    let (event_tx, event_rx) = mpsc::channel::<PerceptionEvent>(16);

    let module = PerceptionModule::new(event_rx)
        .with_buffer_max(1) // flush immediately so we don't wait
        .with_flush_interval(Duration::from_millis(100));

    let bus_clone = bus.clone();
    tokio::spawn(async move {
        module.run(bus_clone).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send a Critical event (published immediately, not buffered).
    let event = PerceptionEvent {
        id: 1,
        timestamp: chrono::Utc::now(),
        source: EventSource::Proc,
        category: EventCategory::Process,
        priority: EventPriority::Critical,
        data: EventData::HighCpu {
            pid: 1234,
            comm: "test-proc".to_string(),
            cpu_percent: 99.5,
        },
    };
    event_tx.send(event).await.expect("send should succeed");

    // Wait for the topic message.
    let received = tokio::time::timeout(Duration::from_secs(2), topic_rx.recv()).await;
    assert!(
        received.is_ok(),
        "should receive a topic message within timeout"
    );

    let envelope = received.unwrap().expect("should get an envelope");

    // Verify the payload is a PerceptionEventMsg.
    let msg: PerceptionEventMsg = match &envelope.payload {
        Payload::Json(val) => serde_json::from_value(val.clone()).expect("should deserialize"),
        _ => panic!("expected JSON payload"),
    };

    assert!(msg.source.contains("Proc"), "source should contain Proc");
    assert!(
        msg.priority.contains("Critical"),
        "priority should contain Critical"
    );
    assert!(
        msg.summary.contains("High CPU"),
        "summary should mention High CPU"
    );
    assert!(
        msg.summary.contains("test-proc"),
        "summary should mention test-proc"
    );
}

// ---------------------------------------------------------------------------
// Test: PerceptionModule — buffered normal events flushed when buffer reaches max
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_perception_module_buffered_events_flushed() {
    let bus = Arc::new(CommunicationBus::new());

    let mut topic_rx = bus.subscribe_topic("perception.events", Some(16));

    let (event_tx, event_rx) = mpsc::channel::<PerceptionEvent>(16);

    // Use buffer_max=1 so the first Normal event is buffered and the second
    // triggers a flush (buffer.len() >= buffer_max).
    let module = PerceptionModule::new(event_rx)
        .with_buffer_max(1)
        .with_flush_interval(Duration::from_secs(600));

    let bus_clone = bus.clone();
    tokio::spawn(async move {
        module.run(bus_clone).await;
    });

    tokio::time::sleep(Duration::from_millis(50)).await;

    // Send two Normal events. The second one triggers a flush of the buffer.
    for i in 0..2 {
        let event = PerceptionEvent {
            id: i + 1,
            timestamp: chrono::Utc::now(),
            source: EventSource::Inotify,
            category: EventCategory::File,
            priority: EventPriority::Normal,
            data: EventData::FileModified {
                path: format!("/tmp/file_{}", i),
            },
        };
        event_tx.send(event).await.expect("send should succeed");
    }

    // Both events should have been published (first buffered, second triggers flush).
    let mut received_count = 0;
    for _ in 0..2 {
        let received = tokio::time::timeout(Duration::from_secs(2), topic_rx.recv()).await;
        if received.is_ok() && received.unwrap().is_ok() {
            received_count += 1;
        }
    }
    assert_eq!(
        received_count, 2,
        "should receive both events via buffer flush"
    );
}
