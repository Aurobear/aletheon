//! Engine modules providing self-field, memory, perception, and body
//! capabilities. These modules are used by the daemon handler and tests.

pub mod config;
pub mod modules;

// Re-export key types
pub use config::EngineConfig;
