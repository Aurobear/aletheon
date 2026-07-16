//! Sandbox trait and types.

use anyhow::Result;
use async_trait::async_trait;
use serde::Deserialize;
use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::time::Duration;

/// Isolation level for sandbox execution.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum IsolationLevel {
    /// No isolation — bare process execution.
    None,
    /// Process-level — resource limits (RLIMIT) but no namespace isolation.
    Process,
    /// Namespace-level — full namespace isolation (pid, net, mount, etc.).
    Namespace,
    /// Container-level — OCI container runtime.
    Container,
}

/// Runtime configuration passed to a sandbox execute call.
#[derive(Debug, Clone, Deserialize)]
pub struct SandboxConfig {
    /// Canonical workspace authority materialized into the process sandbox.
    pub workspace: crate::WorkspacePolicy,
    /// Extra environment variables to set.
    #[serde(default)]
    pub environment: BTreeMap<String, String>,
}

impl SandboxConfig {
    pub fn working_dir(&self) -> &Path {
        self.workspace.cwd()
    }
}

/// Capabilities reported by a sandbox backend.
#[derive(Debug, Clone)]
pub struct SandboxCapabilities {
    /// Whether the backend provides filesystem isolation.
    pub filesystem_isolation: bool,
    /// Whether the backend provides network isolation.
    pub network_isolation: bool,
    /// Whether the backend enforces resource limits.
    pub resource_limits: bool,
    /// Whether the backend applies a seccomp filter.
    pub seccomp_filter: bool,
    /// Human-readable list of known limitations.
    pub limitations: Vec<String>,
}

/// Result of a sandboxed command execution.
#[derive(Debug, Clone)]
pub struct SandboxResult {
    /// Captured stdout.
    pub stdout: String,
    /// Captured stderr.
    pub stderr: String,
    /// Process exit code (-1 if unavailable, e.g. timeout).
    pub exit_code: i32,
    /// Name of the backend that produced this result.
    pub backend_used: String,
    /// Isolation level that was in effect.
    pub isolation_level: IsolationLevel,
    /// Wall-clock elapsed time in milliseconds.
    pub elapsed_ms: u64,
}

/// An argv-preserving sandbox launcher command suitable for bounded execution.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SandboxCommand {
    pub program: PathBuf,
    pub args: Vec<String>,
    pub environment: BTreeMap<String, String>,
}

/// Trait that every sandbox backend must implement.
#[async_trait]
pub trait SandboxBackend: Send + Sync {
    /// Human-readable backend name (e.g. "process", "namespace", "container").
    fn name(&self) -> &str;

    /// The isolation level this backend provides.
    fn isolation_level(&self) -> IsolationLevel;

    /// Check whether this backend is available in the current environment.
    fn is_available(&self) -> bool;

    /// Report the backend's capabilities and limitations.
    fn capabilities(&self) -> SandboxCapabilities;

    /// Wrap an argv command without shell interpolation. Backends that cannot
    /// preserve argv boundaries must fail closed rather than use `sh -c`.
    fn wrap_argv(
        &self,
        _program: &Path,
        _args: &[String],
        _config: &SandboxConfig,
    ) -> Result<SandboxCommand> {
        anyhow::bail!("sandbox backend does not support argv-safe execution")
    }

    /// Execute a shell command under this sandbox with the given config and timeout.
    async fn execute(
        &self,
        cmd: &str,
        config: &SandboxConfig,
        timeout: Duration,
    ) -> Result<SandboxResult>;
}

/// User-facing preference for sandbox isolation level.
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
/// Backends are probed and registered in priority order.
/// Concrete backends (e.g. Bubblewrap, Process, Noop) are provided by the
/// caller so that fabric does not depend on corpus.
pub struct SandboxExecutor {
    backends: Vec<Box<dyn SandboxBackend>>,
    preference: SandboxPreference,
}

impl SandboxExecutor {
    pub fn new(backends: Vec<Box<dyn SandboxBackend>>, preference: SandboxPreference) -> Self {
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
            tracing::warn!("Sandbox degraded to no isolation (BestEffort mode)");
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
