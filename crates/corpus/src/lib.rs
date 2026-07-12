//! Core execution body — the minimal runtime for tool execution.

pub mod core;
pub mod drivers;
pub mod hook;
pub mod security;
pub mod skill;
pub mod tools;

// Re-export main types
pub use core::AletheonBodyRuntime;

// Re-export subcrate modules for backward compatibility
pub use drivers::*;
pub use security::*;
pub use tools::*;

// Re-export skill/hook top-level types for runtime convenience
pub use hook::registry::HookRegistry;
pub use skill::loader::SkillLoader;
pub use skill::plugin::register_skill;
pub use skill::router::SkillRouter;
