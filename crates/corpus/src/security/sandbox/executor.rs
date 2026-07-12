use anyhow::Result;
use std::time::Duration;
use tracing::{info, warn};

use crate::sandbox::bubblewrap::BubblewrapBackend;
use crate::sandbox::noop::NoopBackend;
use crate::sandbox::process::ProcessBackend;
use crate::sandbox::{IsolationLevel, SandboxBackend, SandboxConfig, SandboxResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SandboxPreference {
    /// Select the best available backend automatically.
    Auto,
    /// Require namespace-level isolation; fail if unavailable.
    Require,
    /// Disable sandbox entirely (debug mode).
    Forbid,
    /// Use best available, but warn on degraded isolation.
    BestEffort,
}

impl SandboxPreference {
    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Self {
        match s.to_lowercase().as_str() {
            "auto" => Self::Auto,
            "require" => Self::Require,
            "forbid" => Self::Forbid,
            "best_effort" | "besteffort" => Self::BestEffort,
            _ => Self::Auto,
        }
    }
}

/// Selects the best available sandbox backend and dispatches execution.
///
/// Backends are probed and registered in priority order:
/// Bubblewrap (namespace) > Process (resource limits) > Noop (no isolation).
pub struct SandboxExecutor {
    backends: Vec<Box<dyn SandboxBackend>>,
    preference: SandboxPreference,
}

impl SandboxExecutor {
    pub fn new(preference: SandboxPreference) -> Self {
        let mut backends: Vec<Box<dyn SandboxBackend>> = Vec::new();

        // Priority order: Bubblewrap > Process > Noop
        if let Some(bwrap) = BubblewrapBackend::probe() {
            backends.push(Box::new(bwrap));
        }
        backends.push(Box::new(ProcessBackend));
        backends.push(Box::new(NoopBackend));

        info!(
            preference = ?preference,
            available = backends.len(),
            "SandboxExecutor initialized"
        );

        Self {
            backends,
            preference,
        }
    }

    /// Select the most appropriate backend based on the configured preference.
    pub fn select_backend(&self) -> Option<&dyn SandboxBackend> {
        match self.preference {
            SandboxPreference::Auto | SandboxPreference::BestEffort => {
                // Return the first available backend (highest priority).
                self.backends
                    .iter()
                    .find(|b| b.is_available())
                    .map(|b| b.as_ref())
            }
            SandboxPreference::Require => {
                // Must have namespace-level or better isolation.
                self.backends
                    .iter()
                    .find(|b| {
                        b.is_available()
                            && matches!(
                                b.isolation_level(),
                                IsolationLevel::Namespace | IsolationLevel::Container
                            )
                    })
                    .map(|b| b.as_ref())
            }
            SandboxPreference::Forbid => {
                // Return NoopBackend explicitly.
                self.backends
                    .iter()
                    .find(|b| b.name() == "noop")
                    .map(|b| b.as_ref())
            }
        }
    }

    /// Execute a command using the selected sandbox backend.
    pub async fn run(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        timeout: Duration,
    ) -> Result<SandboxResult> {
        let backend = self
            .select_backend()
            .ok_or_else(|| anyhow::anyhow!("No suitable sandbox backend available"))?;

        // Defense-in-depth: if Require preference somehow selects NoopBackend
        // (e.g. a misconfigured backend claiming namespace isolation), fail
        // explicitly rather than executing without real isolation.
        if self.preference == SandboxPreference::Require && backend.name() == "noop" {
            return Err(anyhow::anyhow!(
                "Sandbox required but NoopBackend was selected (fail-closed)"
            ));
        }

        if self.preference == SandboxPreference::BestEffort
            && backend.isolation_level() == IsolationLevel::None
        {
            warn!("Sandbox degraded to no isolation (BestEffort mode)");
        }

        backend.execute(cmd, config, timeout).await
    }

    /// List all registered backends with their availability status.
    pub fn list_backends(&self) -> Vec<(&str, bool)> {
        self.backends
            .iter()
            .map(|b| (b.name(), b.is_available()))
            .collect()
    }
}
