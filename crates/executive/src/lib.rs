#![allow(
    deprecated,
    clippy::too_many_arguments,
    clippy::module_inception,
    clippy::new_without_default,
    clippy::single_match,
    clippy::unnecessary_unwrap,
    clippy::ptr_arg,
    clippy::needless_update,
    clippy::manual_strip
)]

pub mod core;
pub mod r#impl;
pub mod service;
pub mod tools;
pub mod user_runtime;

pub mod host;
pub use aletheon_kernel as kernel;

// Re-export from core for backward compatibility
pub use core::config::{
    AgentConfig, AppConfig, DaemonConfig, ExecutiveConfig, HooksConfig, McpServerConfig,
    MemoryConfig, PluginsConfig, ProviderConfig, SandboxConfig, Transport,
};
pub use core::orchestrator::AletheonExecutive;
pub use core::verdict_handler::DefaultVerdictHandler;
pub use kernel::admission::ProductionAdmissionController;

// Re-export from impl for backward compatibility
pub use r#impl::agent::AgentRuntime;
pub use r#impl::automation;
pub use r#impl::orchestration;
pub use r#impl::plugin;
pub use r#impl::session;

// ── Re-exports for CLI exec path (bin crate uses these via executive) ───
pub use crate::service::exec_session::ExecSessionBuilder;
pub use fabric::types::admission::RiskLevel;
pub use fabric::{
    AdmissionController, AdmissionRequest, CapabilityId, CapabilityRequest, CapabilityResult,
    CapabilityScope, ContentBlock, LlmProvider, LlmResponse, LlmStream, LocalOsPrincipal, Message,
    NoopTurnEventSink, OperationId, PrincipalId, ProcessId, RecallSet, SandboxRequirement,
    StopReason, StreamChunk, ToolDefinition, TurnRequest, TurnServices, Usage, UsageReport,
};
