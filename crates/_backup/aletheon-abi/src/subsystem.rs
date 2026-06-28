//! Subsystem lifecycle — like Linux kernel's module_init / module_exit.
//!
//! Every Aletheon subsystem (SelfField, BrainCore, BodyRuntime, Memory, etc.)
//! implements this trait. The runtime uses it for init, health checks, and
//! graceful shutdown.

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Semantic version for ABI compatibility checks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Version {
    pub major: u32,
    pub minor: u32,
    pub patch: u32,
}

impl Version {
    pub const fn new(major: u32, minor: u32, patch: u32) -> Self {
        Self { major, minor, patch }
    }

    /// Check if two versions are ABI-compatible (same major).
    pub fn is_compatible_with(&self, other: &Version) -> bool {
        self.major == other.major
    }
}

impl std::fmt::Display for Version {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

/// Initialization phase — like Linux kernel's `early_initcall` / `subsys_initcall`.
#[derive(Debug, Default, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum InitPhase {
    Core = 0,
    #[default]
    Subsystem = 1,
    Service = 2,
    Late = 3,
}

/// Subsystem health status.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SubsystemHealth {
    Healthy,
    Degraded { reason: String },
    Failed { reason: String },
}

/// Context passed to subsystem during init.
///
/// Contains references to shared infrastructure (EventBus, Memory, etc.)
/// that the subsystem needs to register with.
pub struct SubsystemContext {
    pub name: String,
    pub working_dir: std::path::PathBuf,
    pub config: serde_json::Value,
}

/// Unified subsystem lifecycle — every Aletheon subsystem implements this.
///
/// Like Linux kernel's `module_init` / `module_exit` / `module_param`.
#[async_trait]
pub trait Subsystem: Send + Sync {
    /// Subsystem name (e.g., "self_field", "brain_core", "body_runtime").
    fn name(&self) -> &str;

    /// Initialize the subsystem. Called once at startup.
    ///
    /// The subsystem should register its EventBus subscriptions,
    /// load configuration, and prepare resources here.
    async fn init(&mut self, ctx: &SubsystemContext) -> Result<()>;

    /// Health check. Called periodically by the runtime.
    async fn health(&self) -> SubsystemHealth;

    /// Graceful shutdown. Called once at exit.
    ///
    /// The subsystem should flush state, close connections, and
    /// deregister EventBus subscriptions here.
    async fn shutdown(&mut self) -> Result<()>;

    /// Subsystem version. Used for ABI compatibility checks
    /// before hot-upgrade.
    fn version(&self) -> Version;

    /// Initialization phase. Subsystems with lower phases init first.
    fn init_phase(&self) -> InitPhase {
        InitPhase::Subsystem
    }
}
