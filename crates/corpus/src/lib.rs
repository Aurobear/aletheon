//! Core execution body — the minimal runtime for tool execution.

pub mod core;
pub mod bridge;
pub mod testing;

// Re-export main types
pub use core::AletheonBodyRuntime;
