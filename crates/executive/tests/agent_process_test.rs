#![allow(deprecated)]

//! Integration tests for AgentProcess lifecycle, pulse-driven execution,
//! and child spawning.

use std::sync::Arc;

use executive::r#impl::agent::{AgentProcess, AgentProcessConfig, AgentState};
use fabric::evolution::{CognitivePulseEvent, ProviderHealth};
use fabric::CommunicationBus;
use fabric::EventBus;
use fabric::EventType;
use fabric::KernelEventBus;

use std::sync::atomic::{AtomicBool, Ordering};
use uuid::Uuid;

/// Helper: create a CommunicationBus wrapping a KernelEventBus.
fn make_bus() -> Arc<CommunicationBus> {
    let kernel = Arc::new(KernelEventBus::new(256));
    Arc::new(CommunicationBus::from_event_bus(kernel))
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

// ---------------------------------------------------------------------------
// Test 1: AgentProcess lifecycle — Idle -> start -> terminate
// ---------------------------------------------------------------------------

#[tokio::test]
async fn test_agent_process_lifecycle() {
    let bus = make_bus();
    let config = AgentProcessConfig::default();
    let agent = AgentProcess::new(None, "test-task".to_string(), bus, config);

    // Initial state is Idle
    assert_eq!(agent.state(), AgentState::Idle);
    assert!(agent.parent().is_none());
    assert_eq!(agent.task(), "test-task");

    // Subscribe to AgentStarted to verify the event fires
    let started = Arc::new(AtomicBool::new(false));
    let started_clone = started.clone();
    let sub_bus = make_bus();
    sub_bus
        .event_bus()
        .subscribe(
            EventType::AgentStarted,
            Box::new(move |_| {
                started_clone.store(true, Ordering::SeqCst);
                true
            }),
        )
        .await
        .unwrap();

    // Re-create agent with the subscribed bus so events reach the handler
    let mut agent = AgentProcess::new(
        None,
        "test-task".to_string(),
        sub_bus.clone(),
        AgentProcessConfig::default(),
    );

    // start() — state stays Idle (as per implementation) and publishes event
    agent.start().await.unwrap();
    assert_eq!(agent.state(), AgentState::Idle);
    assert!(
        started.load(Ordering::SeqCst),
        "AgentStarted event should have fired"
    );

    // terminate() — state becomes Terminated
    let stopped = Arc::new(AtomicBool::new(false));
    let stopped_clone = stopped.clone();
    sub_bus
        .event_bus()
        .subscribe(
            EventType::AgentStopped,
            Box::new(move |_| {
                stopped_clone.store(true, Ordering::SeqCst);
                true
            }),
        )
        .await
        .unwrap();

    agent.terminate().await.unwrap();
    assert_eq!(agent.state(), AgentState::Terminated);
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
    let mut agent = AgentProcess::new(None, "pulse-test".to_string(), bus, config);

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

    let agent = AgentProcess::new(None, "parent-task".to_string(), bus.clone(), config);
    let parent_pid = agent.pid();

    // Subscribe to AgentSpawned to verify the event
    let spawned = Arc::new(AtomicBool::new(false));
    let spawned_clone = spawned.clone();
    bus.event_bus()
        .subscribe(
            EventType::AgentSpawned,
            Box::new(move |_| {
                spawned_clone.store(true, Ordering::SeqCst);
                true
            }),
        )
        .await
        .unwrap();

    // Spawn a child
    let child_pid = agent.spawn_child("child-task".to_string()).await.unwrap();

    // Child PID is valid and different from parent
    assert_ne!(
        child_pid, parent_pid,
        "Child PID must differ from parent PID"
    );

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
    let agent = AgentProcess::new(None, "no-spawn-task".to_string(), bus, config);

    let result = agent.spawn_child("child-task".to_string()).await;
    assert!(
        result.is_err(),
        "spawn_child should fail when can_spawn is false"
    );
}
