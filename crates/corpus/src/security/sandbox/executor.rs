//! Sandbox executor types — now defined in fabric.
//!
//! This module provides backward-compatible re-exports and a convenience factory
//! that constructs a [`SandboxExecutor`] with corpus default backends.

use fabric::Clock;
use std::sync::Arc;

pub use fabric::sandbox::{SandboxExecutor, SandboxPreference};

/// Construct a [`SandboxExecutor`] pre-loaded with corpus default backends
/// in priority order: Bubblewrap (namespace) > Process (resource limits)
/// > Noop (no isolation).
pub fn create_default_executor(
    preference: SandboxPreference,
    clock: Arc<dyn Clock>,
) -> SandboxExecutor {
    create_executor_with_front_backend(preference, clock, None)
}

pub fn create_executor_with_front_backend(
    preference: SandboxPreference,
    clock: Arc<dyn Clock>,
    front: Option<Box<dyn fabric::sandbox::SandboxBackend>>,
) -> SandboxExecutor {
    use crate::sandbox::bubblewrap::BubblewrapBackend;
    use crate::sandbox::noop::NoopBackend;
    use crate::sandbox::process::ProcessBackend;

    let mut backends: Vec<Box<dyn fabric::sandbox::SandboxBackend>> = Vec::new();
    // An explicitly supplied external owner is the requested execution route,
    // not merely another best-effort candidate. Put it first so enabling the
    // execd gate cannot silently continue through bubblewrap and bypass
    // process/read streaming. With no front backend, Auto keeps the original
    // Bubblewrap > Process > Noop policy unchanged.
    if let Some(front) = front {
        backends.push(front);
    }
    // Default priority order: Bubblewrap > Process > Noop.
    if let Some(bwrap) = BubblewrapBackend::probe(clock.clone()) {
        backends.push(Box::new(bwrap));
    }
    backends.push(Box::new(ProcessBackend {
        clock: clock.clone(),
    }));
    backends.push(Box::new(NoopBackend { clock }));

    tracing::info!(
        preference = ?preference,
        available = backends.len(),
        "SandboxExecutor initialized"
    );

    SandboxExecutor::new(backends, preference)
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use fabric::{
        IsolationLevel, MonoTime, SandboxBackend, SandboxCapabilities, SandboxConfig,
        SandboxResult, WallTime,
    };
    use std::time::Duration;

    struct FixedClock;
    impl Clock for FixedClock {
        fn wall_now(&self) -> WallTime {
            WallTime(0)
        }
        fn mono_now(&self) -> MonoTime {
            MonoTime(0)
        }
    }

    struct Front;
    #[async_trait]
    impl SandboxBackend for Front {
        fn name(&self) -> &str {
            "execd"
        }
        fn isolation_level(&self) -> IsolationLevel {
            IsolationLevel::Process
        }
        fn is_available(&self) -> bool {
            true
        }
        fn capabilities(&self) -> SandboxCapabilities {
            SandboxCapabilities {
                filesystem_isolation: false,
                network_isolation: false,
                resource_limits: false,
                seccomp_filter: false,
                limitations: Vec::new(),
            }
        }
        async fn execute(
            &self,
            _cmd: &str,
            _config: &SandboxConfig,
            _timeout: Duration,
        ) -> anyhow::Result<SandboxResult> {
            unreachable!("selection test does not execute")
        }
    }

    #[test]
    fn explicit_front_backend_is_selected_by_auto() {
        let executor = create_executor_with_front_backend(
            SandboxPreference::Auto,
            Arc::new(FixedClock),
            Some(Box::new(Front)),
        );
        assert_eq!(executor.select_backend().unwrap().name(), "execd");
    }
}
