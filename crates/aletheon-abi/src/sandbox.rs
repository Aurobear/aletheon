//! Sandbox trait and types.
//!
//! Merged from argos-types sandbox module into aletheon-abi.

use async_trait::async_trait;
use anyhow::Result;
use serde::Deserialize;
use std::collections::HashMap;
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
#[derive(Debug, Clone, Default, Deserialize)]
pub struct SandboxConfig {
    /// Working directory for the spawned process.
    pub working_dir: String,
    /// Extra environment variables to set.
    #[serde(default)]
    pub env_vars: HashMap<String, String>,
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

    /// Execute a shell command under this sandbox with the given config and timeout.
    async fn execute(&self, cmd: &str, config: &SandboxConfig, timeout: Duration) -> Result<SandboxResult>;
}
