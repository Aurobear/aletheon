//! # Aletheon Base
//!
//! Core trait definitions for the Aletheon persistent self-evolving runtime.
//! This crate contains **zero implementations** — only interfaces.
//!
//! Like Linux kernel header files define the contract between subsystems
//! (`file_operations`, `net_proto_ops`), this crate defines the contracts
//! between Aletheon subsystems.
//!
//! ## Module Layout (Linux kernel style)
//!
//! - `include/` — Subsystem trait contracts (like kernel `include/`)
//! - `types/` — Shared data types
//! - `events/` — Event system (types + infrastructure)
//! - `ipc/` — Inter-process communication (like kernel `net/`)
//! - `kernel/` — Core infrastructure (observability, registry, debug, errors)
//! - `policy/` — Execution policy engine
//! - `dasein/` — Phenomenological module

// === Module declarations ===

pub mod include;
pub mod types;
pub mod events;
pub mod ipc;
pub mod kernel;
pub mod policy;
pub mod dasein;

// === Backward-compatible module re-exports ===
// These allow `base::genome::*`, `base::self_field::*`, etc. to continue working.
// Downstream crates can also use the new paths: `base::include::genome::*`, `base::types::tool::*`, etc.

// Subsystem trait modules (from include/)
pub use include::body;
pub use include::brain;
pub use include::event_bus;
pub use include::memory;
pub use include::meta;
pub use include::runtime;
pub use include::self_field;
pub use include::subsystem;

// Shared type modules (from types/)
pub use types::agent;
pub use types::capability;
pub use types::context;
pub use types::genome;
pub use types::hook;
pub use types::hook_ext;
pub use types::llm_types;
pub use types::message;
pub use types::paths;
pub use types::permission;
pub use types::resource;
pub use types::sandbox;
pub use types::tool;

// Event modules (from events/)
pub use events::event;
pub use events::evolution;
pub use events::ui_event;

// IPC modules (from ipc/)
// Note: `base::ipc` is already the directory module (pub mod ipc above).
// Old `base::ipc::IpcMessage` etc. are now at `base::ipc::ipc_msg::IpcMessage`.
// Re-export ipc_msg types at ipc level for backward compatibility.
pub use ipc::envelope;
pub use ipc::ipc_types;
pub use ipc::protocol;
pub use ipc::transport;

// Kernel modules (from kernel/)
pub use kernel::debug;
pub use kernel::observable;
pub use kernel::registry;

// Policy modules (from policy/)
pub use policy::execpolicy;
pub use policy::verifier;

// === Re-exports for backward compatibility ===
// These preserve the flat API surface so external consumers don't need to change.

// Subsystem traits (from include/)
pub use include::body::{Action, ActionResult, BodyRuntime};
pub use include::brain::{
    BehaviorAdjustment, BrainCoreOps, CostEstimate, Critique, EvolutionLogEntry, ExecutionResult,
    Experience, LearnedRule, Observation, Plan, PlanStep, Reflection, ReflectionEntry,
    ReflectionOutcome, ReflectionTrigger,
};
pub use include::event_bus::EventBus;
pub use include::memory::{
    CompactResult, CompactStrategy, EmbeddingProvider, MemoryBackend, MemoryEntry, MemoryFilter,
    MemoryHandle, MemoryQuery, MemoryStats, MemoryType,
};
pub use include::meta::{Evaluation, MetaRuntimeOps, MigrationResult, RuntimeCandidate, TestResult};
pub use include::runtime::{AgentInfo, AgentStatus, RuntimeOps, ScheduleKind, ScheduledTask, StepResult};
pub use include::self_field::{
    AwarenessCore, AwarenessExtension, AwarenessExtensionCounts, AwarenessGrowthSuggestion, Care,
    Conflict, Identity, Intent, IntentSource, MutationIntent, Resolution, RiskLevel,
    SelfAwareness, SelfFieldOps, SelfState, Verdict, VerdictAction, VerdictHandler,
};
pub use include::subsystem::{InitPhase, Subsystem, SubsystemContext, SubsystemHealth, Version};

// Shared types (from types/)
pub use types::agent::Pid;
pub use types::capability::{Capability, CapabilitySet, PermissionLevel};
pub use types::context::{Context, TraceState};
pub use types::genome::Genome;
pub use types::hook::{HookContext, HookPoint, HookResult, HookToolResult};
pub use types::hook_ext::{CommandHookResult, HookConfig, HookType};
pub use types::llm_types::ToolDefinition;
pub use types::message::{ContentBlock, ImageSource, Message, Priority as MessagePriority, Role};
pub use types::permission::{ModeConfig, PermissionBehavior, PermissionContext, PermissionMode, PermissionRule};
pub use types::resource::{ManagedResource, ResourceState};
pub use types::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxConfig, SandboxResult,
};
pub use types::tool::{
    PermissionLevel as ToolPermissionLevel, Tool, ToolContext, ToolResult, ToolResultMeta,
};

// Event types (from events/)
pub use events::event::{AsyncEventHandler, ConcreteEvent, Event, EventHandler, EventType, Priority, SubscriptionId};
pub use events::event_bridge::EventBridge;
pub use events::event_log::{EventLog, LogEntry};
pub use events::ui_event::{
    AwarenessLevel, CollaborationMode, EvolutionStage, InterruptReason,
    PlanUpdate, SubAgentHandle, SubAgentStatus, UiEvent,
};

// IPC types (from ipc/)
pub use ipc::envelope::{Endpoint, Envelope, EnvelopeId, ModuleId, Pattern, Payload, Target};
pub use ipc::ipc_msg::{ForkDirective, ForkResult, GroupId, IpcMessage, MessageKind, Signal};
pub use ipc::ipc_types::{
    AgentId, AgentMessage, IpcBackend, IpcPreference, IpcPriority, IpcProbeError, MessageType,
};
pub use ipc::protocol::Protocol;
pub use ipc::transport::{HealthStatus, Transport as EnvelopeTransport, TransportHealth, TransportKind};
pub use ipc::bus::communication_bus::{BusConfig, CommunicationBus};
pub use ipc::bus::kernel_bus::KernelEventBus;

// Kernel foundations (from kernel/)
pub use kernel::debug::{DebugEvent, DebugLevel, DebugSink, Tracepoint};
pub use kernel::debug_bus::{DebugBusHook, EventFilter, PerfCounter};
pub use kernel::error::{
    handle_tool_error, llm_backoff, llm_degradation_chain, tool_backoff, tool_degradation_chain,
    AgentError, BackoffStrategy, DegradationChain, DegradationStrategy, ErrorCategory,
    ErrorSeverity, RegistryErrorKind, ToolErrorAction,
};
pub use kernel::observable::{Observable, SubsystemStatus};
pub use kernel::registry::{RegistrationId, Registry};

// Policy (from policy/)
pub use policy::execpolicy::{
    Decision, NetworkProtocol, NetworkRule, PatternToken, Policy, PrefixRule,
    default_heuristics, load_policy_from_str, load_policy_layered,
};
// Note: policy::execpolicy::Evaluation is not re-exported at crate root
// to avoid conflict with include::meta::Evaluation.
// Access via base::policy::execpolicy::Evaluation or base::execpolicy::Evaluation.
