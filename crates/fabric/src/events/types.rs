//! Event types — like Linux kernel's sk_buff.
//!
//! Events are the primary communication mechanism between Aletheon subsystems.
//! All cross-subsystem messages flow through the EventBus as typed events.

use serde::{Deserialize, Serialize};
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

    // CognitCore
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

/// Maximum time to wait for a request-response cycle.
pub const DEFAULT_REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
