//! Sandbox executor types — now defined in fabric.
//!
//! This module provides backward-compatible re-exports and a convenience factory
//! that constructs a [`SandboxExecutor`] with corpus default backends.

pub use fabric::sandbox::{SandboxExecutor, SandboxPreference};

/// Construct a [`SandboxExecutor`] pre-loaded with corpus default backends
/// in priority order: Bubblewrap (namespace) > Process (resource limits)
/// > Noop (no isolation).
pub fn create_default_executor(preference: SandboxPreference) -> SandboxExecutor {
    use crate::sandbox::bubblewrap::BubblewrapBackend;
    use crate::sandbox::noop::NoopBackend;
    use crate::sandbox::process::ProcessBackend;

    let mut backends: Vec<Box<dyn fabric::sandbox::SandboxBackend>> = Vec::new();

    // Priority order: Bubblewrap > Process > Noop
    if let Some(bwrap) = BubblewrapBackend::probe() {
        backends.push(Box::new(bwrap));
    }
    backends.push(Box::new(ProcessBackend));
    backends.push(Box::new(NoopBackend));

    tracing::info!(
        preference = ?preference,
        available = backends.len(),
        "SandboxExecutor initialized"
    );

    SandboxExecutor::new(backends, preference)
}
