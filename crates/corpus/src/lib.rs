//! Core execution body — the minimal runtime for tool execution.

pub mod core;
pub mod bridge;
pub mod testing;
pub mod drivers;
pub mod tools;
pub mod security;

// Re-export main types
pub use core::AletheonBodyRuntime;

// Re-export subcrate modules for backward compatibility
pub use drivers::*;
pub use tools::*;
pub use security::*;
