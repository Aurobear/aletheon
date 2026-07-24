//! Core execution body — the minimal runtime for tool execution.

#[cfg(feature = "acix")]
pub mod acix;
pub mod catalog;
pub mod core;
pub mod drivers;
pub mod extension;
pub mod hook;
pub mod security;
pub mod service;
pub mod skill;
pub mod tools;

// Re-export main types
pub use catalog::{discover_runtime_extensions, ExtensionCatalog};
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

pub use fabric::{
    ExtensionDescriptor, ExtensionId, ExtensionKind, ExtensionOrigin, ExtensionSnapshot,
};

/// Compatibility export for the Corpus-owned ACIX tool adapter.
#[cfg(feature = "acix")]
pub use acix::tools as acix_tools;
