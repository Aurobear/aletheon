//! Cross-platform abstraction layer.
//!
//! PlatformAdapter provides a unified interface for platform-specific
//! operations (IPC, process management, filesystem, permissions).

use anyhow::Result;
use async_trait::async_trait;
use serde::{Deserialize, Serialize};

/// Platform-specific capabilities.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PlatformCapabilities {
    pub has_dbus: bool,
    pub has_systemd: bool,
    pub has_polkit: bool,
    pub has_binder: bool,
    pub platform_name: String,
}

/// Service information discovered via platform mechanisms.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServiceInfo {
    pub name: String,
    pub status: ServiceStatus,
    pub description: String,
    pub pid: Option<u32>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum ServiceStatus {
    Running,
    Stopped,
    Failed,
    Unknown,
}

/// Cross-platform adapter trait.
#[async_trait]
pub trait PlatformAdapter: Send + Sync {
    /// Platform name (e.g., "linux", "android")
    fn name(&self) -> &str;

    /// Whether this adapter is available on the current system
    fn is_available(&self) -> bool;

    /// Platform capabilities
    fn capabilities(&self) -> PlatformCapabilities;

    // === Service Management ===

    /// List system services
    async fn list_services(&self) -> Result<Vec<ServiceInfo>>;

    /// Get service status
    async fn service_status(&self, name: &str) -> Result<ServiceInfo>;

    /// Start a service
    async fn service_start(&self, name: &str) -> Result<()>;

    /// Stop a service
    async fn service_stop(&self, name: &str) -> Result<()>;

    /// Restart a service
    async fn service_restart(&self, name: &str) -> Result<()>;

    // === System Information ===

    /// Get system hostname
    async fn hostname(&self) -> Result<String>;

    /// Get kernel version
    async fn kernel_version(&self) -> Result<String>;

    /// Get uptime in seconds
    async fn uptime(&self) -> Result<u64>;

    // === Permission Management ===

    /// Check if running as root/admin
    fn is_root(&self) -> bool;

    /// Request privilege escalation (e.g., via polkit/sudo)
    async fn elevate_privileges(&self) -> Result<()>;
}
