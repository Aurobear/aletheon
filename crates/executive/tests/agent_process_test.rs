//! Integration tests for AgentProcess lifecycle, pulse-driven execution,
//! and child spawning.

use std::sync::Arc;

use executive::r#impl::agent::{AgentProcess, AgentProcessConfig, AgentState};
use fabric::evolution::{CognitivePulseEvent, ProviderHealth};
use fabric::ipc::envelope_v2::SchemaId;
use fabric::CommunicationBus;
use fabric::EventType;

use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

use aletheon_kernel::chronos::TestClock;
use fabric::Clock;

/// Helper: create a default TestClock for tests.
fn make_clock() -> Arc<dyn Clock> {
    Arc::new(TestClock::default())
}

/// Helper: create a CommunicationBus.
fn make_bus() -> Arc<CommunicationBus> {
    Arc::new(CommunicationBus::new())
}

/// Helper: create a default CognitivePulseEvent.
fn make_pulse(available_tokens: u32) -> CognitivePulseEvent {
    CognitivePulseEvent {
        pulse_id: Uuid::new_v4(),
        timestamp: "2026-06-14T00:00:00Z".to_string(),
        available_tokens,
        provider_health: ProviderHealth {
            name: "mock".to_string(),
            available: true,
            latency_ms: 10,
            tokens_remaining: Some(available_tokens),
        },
    }
}

/// Helper: subscribe to an event topic and spawn a background task that
/// sets `flag` to `true` when the first message is received.
fn subscribe_to_event(bus: &CommunicationBus, event_type: EventType, flag: Arc<AtomicBool>) {
    let schema = SchemaId::from_event_type(&event_type);
    let mut rx = bus.subscribe_topic(schema, Some(16));
    tokio::spawn(async move {
        if rx.recv().await.is_ok() {
            flag.store(true, Ordering::SeqCst);
        }
    });
}

// ---------------------------------------------------------------------------
// Test 1: AgentProcess lifecycle — Idle -> start -> terminate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_agent_process_lifecycle() {
    let bus = make_bus();
    let config = AgentProcessConfig::default();
    let agent = AgentProcess::new(None, "test-task".to_string(), bus, config, make_clock());

    // Initial state is Idle
    assert_eq!(agent.state(), AgentState::Idle);
    assert!(agent.parent().is_none());
    assert_eq!(agent.task(), "test-task");

    // Subscribe to AgentStarted to verify the event fires
    let started = Arc::new(AtomicBool::new(false));
    let sub_bus = make_bus();
    subscribe_to_event(&sub_bus, EventType::AgentStarted, started.clone());

    // Re-create agent with the subscribed bus so events reach the handler
    let mut agent = AgentProcess::new(
        None,
        "test-task".to_string(),
        sub_bus.clone(),
        AgentProcessConfig::default(),
        make_clock(),
    );

    // start() — state stays Idle (as per implementation) and publishes event
    agent.start().await.unwrap();
    assert_eq!(agent.state(), AgentState::Idle);

    // Allow background task to receive the event
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        started.load(Ordering::SeqCst),
        "AgentStarted event should have fired"
    );

    // terminate() — state becomes Terminated
    let stopped = Arc::new(AtomicBool::new(false));
    subscribe_to_event(&sub_bus, EventType::AgentStopped, stopped.clone());

    agent.terminate().await.unwrap();
    assert_eq!(agent.state(), AgentState::Terminated);

    // Allow background task to receive the event
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    assert!(
        stopped.load(Ordering::SeqCst),
        "AgentStopped event should have fired"
    );
}

// ---------------------------------------------------------------------------
// Test 2: CognitivePulse drives agent — on_pulse behavior with no engine
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_pulse_drives_agent() {
    let bus = make_bus();
    let config = AgentProcessConfig::default();
    let mut agent = AgentProcess::new(None, "pulse-test".to_string(), bus, config, make_clock());

    // Agent starts in Idle — on_pulse should be a no-op (returns Ok, stays Idle)
    let pulse = make_pulse(5_000);
    agent.on_pulse(&pulse).await.unwrap();
    assert_eq!(
        agent.state(),
        AgentState::Idle,
        "Idle agent should not change state on pulse"
    );

    // After terminate, on_pulse should also be a no-op
    agent.terminate().await.unwrap();
    agent.on_pulse(&pulse).await.unwrap();
    assert_eq!(
        agent.state(),
        AgentState::Terminated,
        "Terminated agent should not change state on pulse"
    );
}

// ---------------------------------------------------------------------------
// Test 3: spawn_child — parent spawns child, child PID returned
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_child_agent() {
    let bus = make_bus();
    let config = AgentProcessConfig::default();

    let agent = AgentProcess::new(
        None,
        "parent-task".to_string(),
        bus.clone(),
        config,
        make_clock(),
    );
    let parent_pid = agent.pid();

    // Subscribe to AgentSpawned to verify the event
    let spawned = Arc::new(AtomicBool::new(false));
    subscribe_to_event(&bus, EventType::AgentSpawned, spawned.clone());

    // Spawn a child
    let child_pid = agent.spawn_child("child-task".to_string()).await.unwrap();

    // Child PID is valid and different from parent
    assert_ne!(
        child_pid, parent_pid,
        "Child PID must differ from parent PID"
    );

    // Allow background task to receive the event
    tokio::time::sleep(std::time::Duration::from_millis(50)).await;

    // AgentSpawned event was published
    assert!(
        spawned.load(Ordering::SeqCst),
        "AgentSpawned event should have fired"
    );

    // Cannot exceed max_children (default 4) — spawn 4 more to hit the limit
    for _ in 0..3 {
        agent.spawn_child("extra-child".to_string()).await.unwrap();
    }
    // The 5th child (total 5) should fail since max_children is 4
    let result = agent.spawn_child("one-too-many".to_string()).await;
    assert!(result.is_err(), "Should fail when max_children is exceeded");
}

// ---------------------------------------------------------------------------
// Test: spawn_child blocked when can_spawn is false
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_spawn_child_blocked_when_disabled() {
    let bus = make_bus();
    let config = AgentProcessConfig {
        can_spawn: false,
        ..AgentProcessConfig::default()
    };
    let agent = AgentProcess::new(None, "no-spawn-task".to_string(), bus, config, make_clock());

    let result = agent.spawn_child("child-task".to_string()).await;
    assert!(
        result.is_err(),
        "spawn_child should fail when can_spawn is false"
    );
}
