//! # Aletheon ABI
//!
//! Core trait definitions for the Aletheon persistent self-evolving runtime.
//! This crate contains **zero implementations** — only interfaces.
//!
//! Like Linux kernel header files define the contract between subsystems
//! (`file_operations`, `net_proto_ops`), this crate defines the contracts
//! between Aletheon subsystems.

pub mod agent;
pub mod body;
pub mod brain;
pub mod capability;
pub mod context;
pub mod event;
pub mod event_bus;
pub mod evolution;
pub mod genome;
pub mod memory;
pub mod meta;
pub mod paths;
pub mod runtime;
pub mod self_field;
pub mod subsystem;

// Shared types (message, tool, sandbox, IPC, LLM)
pub mod hook;
pub mod ipc;
pub mod ipc_types;
pub mod llm_types;
pub mod message;
pub mod sandbox;
pub mod tool;

// Shared error types
pub mod error;
pub mod permission;

// Independent execution policy engine
pub mod execpolicy;

// Kernel-style foundations
pub mod observable;
pub mod registry;
pub mod resource;

// Debug infrastructure
pub mod debug;

// Communication protocol types
pub mod envelope;
pub mod protocol;
pub mod transport;

// Re-export key types at crate root for convenience
pub use body::{Action, ActionResult, BodyRuntime};
pub use brain::{
    BehaviorAdjustment, BrainCoreOps, CostEstimate, Critique, EvolutionLogEntry, ExecutionResult,
    Experience, LearnedRule, Observation, Plan, PlanStep, Reflection, ReflectionEntry,
    ReflectionOutcome, ReflectionTrigger,
};
pub use capability::{Capability, CapabilitySet, PermissionLevel};
pub use context::{Context, TraceState};
pub use event::{AsyncEventHandler, Event, EventHandler, EventType, Priority, SubscriptionId};
pub use event_bus::EventBus;
pub use genome::Genome;
pub use memory::{
    CompactResult, CompactStrategy, MemoryBackend, MemoryEntry, MemoryFilter, MemoryHandle,
    MemoryQuery, MemoryStats, MemoryType,
};
pub use meta::{Evaluation, MetaRuntimeOps, MigrationResult, RuntimeCandidate, TestResult};
pub use runtime::{AgentInfo, AgentStatus, RuntimeOps, ScheduleKind, ScheduledTask, StepResult};
pub use self_field::{
    AwarenessCore, AwarenessExtension, AwarenessExtensionCounts, AwarenessGrowthSuggestion, Care,
    Conflict, Identity, Intent, IntentSource, MutationIntent, Resolution, RiskLevel,
    SelfAwareness, SelfFieldOps, SelfState, Verdict, VerdictAction, VerdictHandler,
};
pub use subsystem::{InitPhase, Subsystem, SubsystemContext, SubsystemHealth, Version};

// Re-export shared types
// Note: tool::PermissionLevel (L0-L3) is aliased as ToolPermissionLevel
// to avoid conflict with capability::PermissionLevel (ReadOnly/SandboxWrite/...).
pub use hook::{HookContext, HookPoint, HookResult, HookToolResult};
pub use ipc::{ForkDirective, ForkResult, GroupId, IpcMessage, MessageKind, Signal};
pub use ipc_types::{
    AgentId, AgentMessage, IpcBackend, IpcPreference, IpcPriority, IpcProbeError, MessageType,
};
pub use llm_types::ToolDefinition;
pub use message::{ContentBlock, ImageSource, Message, Priority as MessagePriority, Role};
pub use sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxConfig, SandboxResult,
};
pub use tool::{
    PermissionLevel as ToolPermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta,
};

// Re-export key error types
pub use error::{
    handle_tool_error, llm_backoff, llm_degradation_chain, tool_backoff, tool_degradation_chain,
    AgentError, BackoffStrategy, DegradationChain, DegradationStrategy, ErrorCategory,
    ErrorSeverity, RegistryErrorKind, ToolErrorAction,
};

// Re-export kernel-style foundations
pub use observable::{Observable, SubsystemStatus};
pub use registry::{RegistrationId, Registry};
pub use resource::{ManagedResource, ResourceState};

// Re-export communication protocol types
pub use envelope::{Endpoint, Envelope, EnvelopeId, ModuleId, Pattern, Payload, Target};
pub use protocol::Protocol;
pub use transport::{HealthStatus, Transport as EnvelopeTransport, TransportHealth, TransportKind};

// Re-export permission types
pub use permission::{PermissionBehavior, PermissionContext, PermissionMode, PermissionRule};
