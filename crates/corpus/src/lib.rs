//! Core execution body — the minimal runtime for tool execution.

pub mod bridge;
pub mod core;
pub mod drivers;
pub mod security;
pub mod testing;
pub mod tools;

// Re-export main types
pub use core::AletheonBodyRuntime;

// Re-export subcrate modules for backward compatibility
pub use drivers::*;
pub use security::*;
pub use tools::*;
