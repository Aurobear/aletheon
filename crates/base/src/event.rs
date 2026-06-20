//! Event types — like Linux kernel's sk_buff.
//!
//! Events are the primary communication mechanism between Aletheon subsystems.
//! All cross-subsystem messages flow through the EventBus as typed events.

use async_trait::async_trait;
use serde::{Deserialize, Serialize};
use std::any::Any;
use std::future::Future;
use std::pin::Pin;
use std::time::Duration;

/// Event type identifier — like IRQ numbers.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum EventType {
    // User-space
    UserIntent,
    UserFeedback,

    // Environment
    EnvironmentChange,
    PerceptionUpdate,

    // BodyRuntime
    ToolObservation,
    ToolError,
    ActionCompleted,

    // Memory
    MemoryStored,
    MemoryRecalled,
    MemoryCompacted,

    // SelfField
    IdentityQuery,
    BoundaryCheck,
    ConflictDetected,
    RejectionIssued,

    // BrainCore
    PlanGenerated,
    ReflectionComplete,
    CriticismRaised,

    // MetaRuntime
    MutationIntent,
    RuntimeCandidate,
    MigrationStarted,
    MigrationComplete,

    // Lifecycle
    SubsystemStarted,
    SubsystemFailed,
    HealthCheck,

    // Runtime
    AgentStarted,
    AgentStopped,
    AgentFailed,
    ScheduledTaskFired,
    BootPhaseChanged,
    ReActIterationStart,
    ReActIterationEnd,
    AgentForkCompleted,

    // Self-evolution
    RuleExtracted,
    EvolutionTriggered,
    EvolutionResult,

    // Energy / agent lifecycle
    CognitivePulse,
    AgentSpawned,
}

/// Event priority — like IRQ priority levels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Default, Serialize, Deserialize)]
pub enum Priority {
    Critical = 0, // Emergency stop, security events
    High = 1,     // User intent, conflict detection
    #[default]
    Normal = 2, // Regular tasks
    Low = 3,      // Background learning, health checks
    Background = 4, // Maintenance tasks
}

/// Unique identifier for an event subscription.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SubscriptionId(pub u64);

/// Event trait — the fundamental unit of communication.
///
/// Like Linux kernel's `sk_buff`, events carry typed payloads through
/// the system. All cross-subsystem communication happens via events.
#[async_trait]
pub trait Event: Send + Sync + 'static {
    /// The event type (used for routing and subscription filtering).
    fn event_type(&self) -> EventType;

    /// Priority level (determines processing order).
    fn priority(&self) -> Priority;

    /// Name of the subsystem that produced this event.
    fn source(&self) -> &str;

    /// Type-erased payload. Downcast to concrete type at the receiver.
    fn payload(&self) -> &dyn Any;

    /// Human-readable summary for logging/debugging.
    fn summary(&self) -> String {
        format!("{:?} from {}", self.event_type(), self.source())
    }

    /// Serialize the event to JSON for async handler delivery.
    ///
    /// Default returns `Null` — concrete event types should override
    /// to provide structured data for `AsyncEventHandler`.
    fn to_json(&self) -> serde_json::Value {
        serde_json::Value::Null
    }
}

/// Handler function for event subscription.
///
/// Receives a reference to the event and returns whether to continue
/// processing (true) or stop propagation (false).
pub type EventHandler = Box<dyn Fn(&dyn Event) -> bool + Send + Sync>;

/// Async handler for event subscription.
///
/// Receives the event as a serialized JSON value and returns whether to
/// continue processing (true) or stop propagation (false). Use this when
/// the handler needs to perform async I/O (e.g., memory writes, LLM calls).
pub type AsyncEventHandler =
    Box<dyn Fn(serde_json::Value) -> Pin<Box<dyn Future<Output = bool> + Send>> + Send + Sync>;

/// Maximum time to wait for a request-response cycle.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);

/// A concrete event implementation for direct use.
pub struct ConcreteEvent {
    event_type: EventType,
    priority: Priority,
    source: String,
    payload: Box<dyn Any + Send + Sync>,
}

impl ConcreteEvent {
    pub fn new(
        event_type: EventType,
        priority: Priority,
        source: String,
        payload: Box<dyn Any + Send + Sync>,
    ) -> Self {
        Self {
            event_type,
            priority,
            source,
            payload,
        }
    }
}

#[async_trait]
impl Event for ConcreteEvent {
    fn event_type(&self) -> EventType {
        self.event_type.clone()
    }
    fn priority(&self) -> Priority {
        self.priority
    }
    fn source(&self) -> &str {
        &self.source
    }
    fn payload(&self) -> &dyn Any {
        &*self.payload
    }

    fn to_json(&self) -> serde_json::Value {
        self.payload
            .downcast_ref::<serde_json::Value>()
            .cloned()
            .unwrap_or(serde_json::Value::Null)
    }
}
