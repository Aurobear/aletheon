//! # Aletheon ABI
//!
//! Core trait definitions for the Aletheon persistent self-evolving runtime.
//! This crate contains **zero implementations** — only interfaces.
//!
//! Like Linux kernel header files define the contract between subsystems
//! (`file_operations`, `net_proto_ops`), this crate defines the contracts
//! between Aletheon subsystems.

pub mod subsystem;
pub mod event;
pub mod event_bus;
pub mod context;
pub mod capability;
pub mod body;
pub mod memory;
pub mod self_field;
pub mod brain;
pub mod meta;
pub mod runtime;
pub mod genome;

// Merged from argos-types
pub mod message;
pub mod tool;
pub mod sandbox;
pub mod ipc_types;
pub mod llm_types;

// Shared error types
pub mod error;

// Re-export key types at crate root for convenience
pub use subsystem::{Subsystem, SubsystemHealth, SubsystemContext, Version};
pub use event::{Event, EventType, Priority, SubscriptionId, EventHandler};
pub use event_bus::EventBus;
pub use context::{Context, TraceState};
pub use capability::{Capability, CapabilitySet, PermissionLevel};
pub use body::{Action, ActionResult, BodyRuntime};
pub use memory::{MemoryBackend, MemoryEntry, MemoryHandle, MemoryQuery, MemoryType, MemoryFilter, CompactStrategy, CompactResult, MemoryStats};
pub use self_field::{SelfFieldOps, Verdict, Intent, IntentSource, Identity, Care, Conflict, Resolution, MutationIntent};
pub use brain::{BrainCoreOps, Plan, PlanStep, CostEstimate, ExecutionResult, Reflection, Critique, LearnedRule, Experience, Observation};
pub use meta::{MetaRuntimeOps, RuntimeCandidate, TestResult, Evaluation, MigrationResult};
pub use genome::Genome;
pub use runtime::{RuntimeOps, AgentInfo, AgentStatus, ScheduledTask, ScheduleKind, StepResult};

// Re-export key types from argos-types migration
// Note: tool::PermissionLevel (L0-L3) is aliased as ToolPermissionLevel
// to avoid conflict with capability::PermissionLevel (ReadOnly/SandboxWrite/...).
pub use message::{Message, ContentBlock, Role, ImageSource, Priority as MessagePriority};
pub use tool::{Tool, ToolResult, ToolResultMeta, ToolContext, PermissionLevel as ToolPermissionLevel};
pub use llm_types::ToolDefinition;
pub use sandbox::{SandboxBackend, SandboxConfig, SandboxResult, SandboxCapabilities, IsolationLevel};
pub use ipc_types::{IpcBackend, IpcPreference, IpcProbeError, AgentMessage, AgentId, MessageType, IpcPriority};

// Re-export key error types
pub use error::{AgentError, ErrorSeverity, ErrorCategory, BackoffStrategy, DegradationStrategy, ToolErrorAction,
    DegradationChain, handle_tool_error, tool_backoff, llm_backoff, llm_degradation_chain, tool_degradation_chain};
