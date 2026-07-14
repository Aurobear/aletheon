//! Cross-platform abstraction layer.

pub mod adapter;
pub mod android;
pub mod awareness;
pub mod boot;

#[cfg(feature = "dbus")]
pub mod linux;

pub use adapter::{PlatformAdapter, PlatformCapabilities, ServiceInfo, ServiceStatus};

use fabric::Clock;
use std::sync::Arc;

/// Create the best available platform adapter.
pub fn create_platform_adapter(clock: Arc<dyn Clock>) -> Box<dyn PlatformAdapter> {
    // Check Android first
    if android::AndroidPlatformAdapter::is_android() {
        return Box::new(android::AndroidPlatformAdapter::new(clock));
    }

    #[cfg(feature = "dbus")]
    {
        if linux::LinuxPlatformAdapter::is_available_static() {
            return Box::new(linux::LinuxPlatformAdapter::new());
        }
    }

    // Fallback: basic /proc-based adapter
    Box::new(BasicLinuxAdapter)
}

/// Basic Linux adapter using /proc and systemd CLI (always available).
pub struct BasicLinuxAdapter;

#[async_trait::async_trait]
impl PlatformAdapter for BasicLinuxAdapter {
    fn name(&self) -> &str {
        "basic_linux"
    }
    fn is_available(&self) -> bool {
        true
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            has_dbus: false,
            has_systemd: true,
            has_polkit: false,
            has_binder: false,
            platform_name: "linux".to_string(),
        }
    }

    async fn list_services(&self) -> anyhow::Result<Vec<ServiceInfo>> {
        let output = tokio::process::Command::new("systemctl")
            .args(["list-units", "--type=service", "--no-pager", "--plain"])
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        Ok(stdout
            .lines()
            .skip(1)
            .filter_map(|line| {
                let parts: Vec<&str> = line.split_whitespace().collect();
                if parts.len() >= 4 {
                    Some(ServiceInfo {
                        name: parts[0].to_string(),
                        status: match parts[2] {
                            "active" => ServiceStatus::Running,
                            "inactive" => ServiceStatus::Stopped,
                            "failed" => ServiceStatus::Failed,
                            _ => ServiceStatus::Unknown,
                        },
                        description: parts[3..].join(" "),
                        pid: None,
                    })
                } else {
                    None
                }
            })
            .collect())
    }

    async fn service_status(&self, name: &str) -> anyhow::Result<ServiceInfo> {
        let output = tokio::process::Command::new("systemctl")
            .args(["is-active", name])
            .output()
            .await?;

        let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(ServiceInfo {
            name: name.to_string(),
            status: match status.as_str() {
                "active" => ServiceStatus::Running,
                "inactive" => ServiceStatus::Stopped,
                "failed" => ServiceStatus::Failed,
                _ => ServiceStatus::Unknown,
            },
            description: String::new(),
            pid: None,
        })
    }

    async fn service_start(&self, name: &str) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("systemctl")
            .args(["start", name])
            .output()
            .await?;
        if !output.status.success() {
            anyhow::bail!(
                "Failed to start {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    async fn service_stop(&self, name: &str) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("systemctl")
            .args(["stop", name])
            .output()
            .await?;
        if !output.status.success() {
            anyhow::bail!(
                "Failed to stop {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    async fn service_restart(&self, name: &str) -> anyhow::Result<()> {
        let output = tokio::process::Command::new("systemctl")
            .args(["restart", name])
            .output()
            .await?;
        if !output.status.success() {
            anyhow::bail!(
                "Failed to restart {}: {}",
                name,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        Ok(())
    }

    async fn hostname(&self) -> anyhow::Result<String> {
        Ok(hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string()))
    }

    async fn kernel_version(&self) -> anyhow::Result<String> {
        let version = tokio::fs::read_to_string("/proc/version").await?;
        Ok(version
            .split_whitespace()
            .nth(2)
            .unwrap_or("unknown")
            .to_string())
    }

    async fn uptime(&self) -> anyhow::Result<u64> {
        let uptime = tokio::fs::read_to_string("/proc/uptime").await?;
        let seconds: f64 = uptime
            .split_whitespace()
            .next()
            .and_then(|s| s.parse().ok())
            .unwrap_or(0.0);
        Ok(seconds as u64)
    }

    fn is_root(&self) -> bool {
        unsafe { libc::geteuid() == 0 }
    }

    async fn elevate_privileges(&self) -> anyhow::Result<()> {
        anyhow::bail!("Privilege escalation not implemented for basic_linux adapter. Use D-Bus adapter for polkit support.")
    }
}
