//! Core execution body — the minimal runtime for tool execution.

pub mod core;
pub mod drivers;
pub mod hook;
pub mod security;
pub mod service;
pub mod skill;
pub mod tools;

// Re-export main types
pub use core::AletheonBodyRuntime;
pub use service::{
    ActivationId, ActivationReceipt, ActivationRequest, CorpusError, CorpusRetryDisposition,
    CorpusService, DefaultCorpusService, ExtensionCatalog, ExtensionDescriptor, ExtensionGrant,
    ExtensionId, ExtensionKind, ExtensionSnapshot, GovernedInvocation,
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
