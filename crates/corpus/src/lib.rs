//! Core execution body — the minimal runtime for tool execution.

#[cfg(feature = "acix")]
pub mod acix;
pub mod agent_lifecycle_hooks;
pub mod catalog;
pub mod core;
pub mod drivers;
pub mod extension;
pub mod hook;
mod process_spawn;
pub mod security;
pub mod service;
pub mod skill;
pub mod tools;

// Re-export main types
pub use agent_lifecycle_hooks::CorpusAgentLifecycleHookSink;
pub use catalog::{
    discover_runtime_extensions, ActivationConstraints, ExtensionCatalog, ExtensionContractError,
    ExtensionDescriptor, ExtensionId, ExtensionKind, ExtensionOrigin, ExtensionSnapshot,
};
pub use core::AletheonBodyRuntime;
pub use service::{
    ActivatedCorpusExecutor, ActivationId, ActivationReceipt, ActivationRequest, CorpusError,
    CorpusRetryDisposition, CorpusService, DefaultCorpusService, ExtensionGrant,
    GovernedInvocation,
};

// Re-export subcrate modules for backward compatibility
pub use drivers::*;
pub use security::*;
pub use tools::*;

// Re-export skill/hook top-level types for runtime convenience
pub use hook::registry::HookRegistry;
pub use skill::loader::SkillLoader;
pub use skill::plugin::register_skill;
pub use skill::router::SkillRouter;

/// Compatibility export for the Corpus-owned ACIX tool adapter.
#[cfg(feature = "acix")]
pub use acix::tools as acix_tools;
