//! Linux platform adapter with D-Bus integration.
//!
//! Uses zbus for D-Bus communication with systemd, polkit, and other system services.

#[cfg(feature = "dbus")]
use zbus::Connection;

use super::adapter::*;
use anyhow::Result;
use async_trait::async_trait;

/// Linux platform adapter with D-Bus support.
pub struct LinuxPlatformAdapter {
    #[cfg(feature = "dbus")]
    connection: Option<Connection>,
}

impl LinuxPlatformAdapter {
    pub fn new() -> Self {
        Self {
            #[cfg(feature = "dbus")]
            connection: None,
        }
    }

    pub fn is_available_static() -> bool {
        // Check if D-Bus system bus is available
        std::path::Path::new("/var/run/dbus/system_bus_socket").exists()
    }
}

#[async_trait]
impl PlatformAdapter for LinuxPlatformAdapter {
    fn name(&self) -> &str {
        "linux_dbus"
    }

    fn is_available(&self) -> bool {
        Self::is_available_static()
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            has_dbus: true,
            has_systemd: true,
            has_polkit: true,
            has_binder: false,
            platform_name: "linux".to_string(),
        }
    }

    async fn list_services(&self) -> Result<Vec<ServiceInfo>> {
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

    async fn service_status(&self, name: &str) -> Result<ServiceInfo> {
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

    async fn service_start(&self, name: &str) -> Result<()> {
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

    async fn service_stop(&self, name: &str) -> Result<()> {
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

    async fn service_restart(&self, name: &str) -> Result<()> {
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

    async fn hostname(&self) -> Result<String> {
        Ok(hostname::get()
            .map(|h| h.to_string_lossy().to_string())
            .unwrap_or_else(|_| "unknown".to_string()))
    }

    async fn kernel_version(&self) -> Result<String> {
        let version = tokio::fs::read_to_string("/proc/version").await?;
        Ok(version
            .split_whitespace()
            .nth(2)
            .unwrap_or("unknown")
            .to_string())
    }

    async fn uptime(&self) -> Result<u64> {
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

    async fn elevate_privileges(&self) -> Result<()> {
        // Use pkexec for polkit-based privilege escalation
        let output = tokio::process::Command::new("pkexec")
            .arg("--help")
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => {
                tracing::info!("Polkit available for privilege escalation");
                Ok(())
            }
            _ => {
                // Fallback to sudo
                let output = tokio::process::Command::new("sudo")
                    .arg("-n")
                    .arg("true")
                    .output()
                    .await?;

                if output.status.success() {
                    tracing::info!("Sudo available for privilege escalation");
                    Ok(())
                } else {
                    anyhow::bail!("No privilege escalation method available (polkit/sudo)")
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linux_adapter_capabilities() {
        let adapter = LinuxPlatformAdapter::new();
        let caps = adapter.capabilities();
        assert!(caps.has_systemd);
        assert_eq!(caps.platform_name, "linux");
    }

    #[tokio::test]
    async fn test_linux_adapter_hostname() {
        let adapter = LinuxPlatformAdapter::new();
        let hostname = adapter.hostname().await.unwrap();
        assert!(!hostname.is_empty());
    }

    #[tokio::test]
    async fn test_linux_adapter_kernel_version() {
        let adapter = LinuxPlatformAdapter::new();
        let version = adapter.kernel_version().await.unwrap();
        assert!(!version.is_empty());
    }

    #[tokio::test]
    async fn test_linux_adapter_uptime() {
        let adapter = LinuxPlatformAdapter::new();
        let uptime = adapter.uptime().await.unwrap();
        let _ = uptime;
    }
}
