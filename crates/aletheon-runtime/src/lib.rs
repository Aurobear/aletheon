pub mod bridge;
pub mod core;
pub mod r#impl;

// Re-export from core for backward compatibility
pub use core::behavior_paths::{BehaviorPath, BehaviorPathRouter};
pub use core::config::{
    AgentConfig, AppConfig, DaemonConfig, HooksConfig, McpServerConfig, MemoryConfig,
    PluginsConfig, ProviderConfig, RuntimeConfig, SandboxConfig, Transport,
};
pub use core::orchestrator::AletheonRuntime;
pub use core::react_loop::{ReActLoop, TurnMetrics};
pub use core::verdict_handler::DefaultVerdictHandler;

// Re-export from impl for backward compatibility
pub use r#impl::agent::AgentRuntime;
pub use r#impl::automation;
pub use r#impl::orchestration;
pub use r#impl::plugin;
pub use r#impl::session;

// Re-export provider registry from brain-core
pub use aletheon_brain::r#impl::provider_registry::ProviderRegistry;

// Re-export memory types
pub use r#impl::memory::core_memory::CoreMemory;
pub use r#impl::memory::recall_memory::RecallMemory;
pub use r#impl::memory::tools as memory_tools;
