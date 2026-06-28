//! Lifecycle hooks system — external commands that run at key lifecycle events.
//!
//! Hooks are registered via config files or the API, then executed by the runner
//! at matching lifecycle events (session start, tool use, compaction, etc.).

pub mod registry;
pub mod runner;
pub mod types;

pub use registry::HookRegistry;
pub use runner::HookRunner;
pub use types::{Hook, HookEvent, HookPayload, HookResponse};
