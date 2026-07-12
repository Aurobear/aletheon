//! # Aletheon ABI
//!
//! Shared traits, types, and error handling for the Aletheon macro-kernel.
//! This crate is the foundation — all other Aletheon crates depend on it.
//!
//! ## Module Layout
//!
//! - `include/` — Subsystem trait contracts
//! - `types/` — Shared data types
//! - `event.rs` — Event trait and handler types
//! - `compaction.rs` — Context compaction interface
//! - `error.rs` — AgentError and error handling types
//! - `bus_handle.rs` — BusHandle trait for subsystem communication

#![allow(deprecated)]

// === Module declarations ===

pub mod bus_handle;
pub mod compaction;
pub mod error;
pub mod event;
pub mod include;
pub mod types;

// === Module-level re-exports (backward compat) ===
// These let `crate::agora::*`, `crate::context::*`, etc. resolve.

// Subsystem trait modules (from include/)
pub use include::agora;
pub use include::body;
pub use include::cognit;
pub use include::event_bus;
pub use include::memory;
pub use include::meta;
pub use include::plugin;
pub use include::runtime;
pub use include::self_field;
pub use include::space;
pub use include::subsystem;

// Shared type modules (from types/)
pub use types::admission;
pub use types::agent;
pub use types::capability;
pub use types::context;
pub use types::genome;
pub use types::grounding;
pub use types::hook;
pub use types::hook_ext;
pub use types::llm_types;
pub use types::message;
pub use types::objective;
pub use types::paths;
pub use types::permission;
pub use types::resource;
pub use types::sandbox;
pub use types::tool;
pub use types::vision;

// === Type-level re-exports ===
// These let `crate::Context`, `crate::Subsystem`, etc. resolve.

pub use compaction::{prune_tool_outputs, CompactorTrait};

pub use include::admission::AdmissionController;
pub use include::agora::{
    AgoraCommit, AgoraOperation, AgoraOps, AgoraProposal, RejectReason, VersionConflict,
};
pub use include::body::{Action, ActionResult, BodyRuntime};
pub use include::capability_invoker::CapabilityInvoker;
pub use include::chronos::Clock;
pub use include::cognit::{
    BehaviorAdjustment, CognitOps, CostEstimate, Critique, EvolutionLogEntry, ExecutionResult,
    Experience, LearnedRule, Observation, Plan, PlanStep, Reflection, ReflectionEntry,
    ReflectionOutcome, ReflectionTrigger,
};
pub use include::event_bus::EventBus;
pub use include::memory::{
    CompactResult, CompactStrategy, EmbeddingProvider, MemoryBackend, MemoryEntry, MemoryFilter,
    MemoryHandle, MemoryQuery, MemoryStats, MemoryType,
};
pub use include::meta::{
    Evaluation, MetaRuntimeOps, MigrationResult, RuntimeCandidate, TestResult,
};
pub use include::plugin::{Plugin, PluginContext};
pub use include::process::{OperationHandle, OperationManager, ProcessHandle, ProcessManager};
pub use include::runtime::{
    AgentInfo, AgentStatus, RuntimeOps, ScheduleKind, ScheduledTask, StepResult,
};
pub use include::self_field::{
    AwarenessCore, AwarenessExtension, AwarenessExtensionCounts, AwarenessGrowthSuggestion, Care,
    Conflict, Identity, Intent, IntentSource, MutationIntent, Resolution, RiskLevel, SelfAwareness,
    SelfFieldOps, SelfState, Verdict, VerdictAction, VerdictHandler,
};
pub use include::space::SpaceManager;
pub use include::subsystem::{InitPhase, Subsystem, SubsystemContext, SubsystemHealth, Version};
pub use include::turn::{
    AgoraView, CapabilityRequest, CapabilityResult, DaseinView, NoopTurnEventSink, RecallRequest,
    RecallSet, StubTurnServices, TurnEventSink, TurnServices,
};

// Shared types (from types/)
pub use types::admission::{
    AdmissionError, AdmissionRequest, AuditEventId, BudgetRequest, BudgetReservationId,
    CapabilityId, CapabilityScope, ExecutionPermit, LeaseRequest, PermitId, PrincipalId,
    ResourceLeaseId, RevokeReason, SandboxDecision, SandboxRequirement, UsageReport,
};
pub use types::agent::Pid;
pub use types::capability::{Capability, CapabilitySet, PermissionLevel};
pub use types::context::{Context, TraceState};
pub use types::evidence::Evidence;
pub use types::genome::Genome;
pub use types::hook::{HookContext, HookPoint, HookResult, HookToolResult};
pub use types::hook_ext::{CommandHookResult, HookConfig, HookType};
pub use types::llm_types::{
    LlmProvider, LlmResponse, LlmStream, ModelInfo, StopReason, StreamChunk, ToolDefinition, Usage,
};
pub use types::message::{ContentBlock, ImageSource, Message, Priority as MessagePriority, Role};
pub use types::objective::{Objective, ObjectiveStatus, ObjectiveSummary};
pub use types::operation::{
    CancelReason, MonoDeadlineMillis, OperationExitReason, OperationId, OperationKind,
    OperationRecord, OperationRequest, OperationResult, OperationState, ProcessId,
};
pub use types::permission::{
    ModeConfig, PermissionBehavior, PermissionContext, PermissionMode, PermissionRule,
};
pub use types::process::{
    AgentId, AgentProfileId, ExitReason, ExitStatus, MailboxId, NamespaceId, ProcessRecord,
    ProcessSignal, ProcessSnapshot, ProcessState, SpaceId, SpawnSpec,
};
pub use types::resource::{ManagedResource, ResourceState};
pub use types::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxConfig, SandboxResult,
};
pub use types::space::{AccessMode, AgoraSpaceId, AgoraVersion, ContextBinding};
pub use types::time::{MonoDeadline, MonoTime, WallTime};
pub use types::tool::{
    PermissionLevel as ToolPermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta,
};
pub use types::turn::{TurnEvent, TurnMetrics, TurnRequest, TurnResult, TurnStop};

// Event types
pub use event::{
    AsyncEventHandler, ConcreteEvent, Event, EventHandler, EventType, Priority as EventPriority,
    SubscriptionId,
};

// Error types
pub use error::{
    handle_tool_error, llm_backoff, llm_degradation_chain, tool_backoff, tool_degradation_chain,
    AgentError, BackoffStrategy, DegradationChain, DegradationStrategy, ErrorCategory,
    ErrorSeverity, LlmErrorKind, RegistryErrorKind, SandboxErrorKind, ToolErrorAction,
    ToolErrorKind,
};
