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

pub mod bridge;
pub mod core;
pub mod r#impl;
pub mod tools;

pub mod host;

// Re-export from core for backward compatibility
pub use cognit::harness::linear::{ReActLoop, TurnMetrics};
pub use core::behavior_paths::{BehaviorPath, BehaviorPathRouter};
pub use core::config::{
    AgentConfig, AppConfig, DaemonConfig, HooksConfig, McpServerConfig, MemoryConfig,
    PluginsConfig, ProviderConfig, RuntimeConfig, SandboxConfig, Transport,
};
pub use core::orchestrator::AletheonRuntime;
pub use core::verdict_handler::DefaultVerdictHandler;

// Re-export from impl for backward compatibility
pub use r#impl::agent::AgentRuntime;
pub use r#impl::automation;
pub use r#impl::orchestration;
pub use r#impl::plugin;
pub use r#impl::session;

// Re-export provider registry from brain-core
pub use cognit::r#impl::provider_registry::ProviderRegistry;

// Re-export memory types (now in the memory crate, Group B Phase 2)
pub use memory::memory_tools;
pub use memory::CoreMemory;
pub use memory::RecallMemory;
