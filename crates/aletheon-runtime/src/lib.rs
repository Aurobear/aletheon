pub mod core;
pub mod bridge;
pub mod r#impl;

// Re-export from core for backward compatibility
pub use core::config::{RuntimeConfig, AppConfig, AgentConfig, ProviderConfig, Transport};
pub use core::orchestrator::AletheonRuntime;
pub use core::behavior_paths::{BehaviorPath, BehaviorPathRouter};
pub use core::react_loop::ReActLoop;

// Re-export from impl for backward compatibility
pub use r#impl::agent::AgentRuntime;
pub use r#impl::orchestration as orchestration;
pub use r#impl::automation as automation;
pub use r#impl::session as session;
pub use r#impl::plugin as plugin;

// Re-export provider registry from brain-core (migrated from argos-core)
pub use aletheon_brain_core::r#impl::provider_registry::ProviderRegistry;

// Re-export memory types (migrated from argos-core into aletheon-runtime)
pub use r#impl::memory::core_memory::CoreMemory;
pub use r#impl::memory::recall_memory::RecallMemory;
pub use r#impl::memory::tools as memory_tools;
