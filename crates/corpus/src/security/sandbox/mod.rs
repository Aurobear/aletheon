//! Sandbox execution layer.

pub mod backend;
pub mod bubblewrap;
pub mod bwrap_builder;
pub mod env;
pub mod executor;
pub mod glob_scanner;
pub mod noop;
pub mod policy;
pub mod process;
pub mod profile;
pub(crate) mod streaming;

// Re-export key types for convenience (inlined from backend.rs)
pub use bubblewrap::BubblewrapBackend;
pub use bwrap_builder::BwrapBuilder;
pub use env::SandboxEnvironment;
pub use executor::{SandboxExecutor, SandboxPreference};
pub use fabric::sandbox::{
    IsolationLevel, SandboxBackend, SandboxCapabilities, SandboxCommand, SandboxConfig,
    SandboxResult,
};
pub use glob_scanner::GlobScanner;
pub use noop::NoopBackend;
pub use policy::{FilesystemPolicy, FsDefault, WritableRoot};
pub use process::ProcessBackend;
pub use profile::SandboxProfile;
