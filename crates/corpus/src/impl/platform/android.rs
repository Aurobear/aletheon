//! Android platform adapter with Binder IPC.
//!
//! This is a stub implementation. Full Android support requires:
//! - Android NDK for native compilation
//! - Binder IPC library (libbinder_ndk)
//! - AOSP APIs for service management
//!
//! To build for Android:
//! 1. Install Android NDK
//! 2. Set target: aarch64-linux-android / x86_64-linux-android
//! 3. Enable feature: --features android

use super::adapter::*;
use anyhow::Result;
use async_trait::async_trait;

/// Android platform adapter.
///
/// Currently a stub. Full implementation requires Android NDK compilation.
pub struct AndroidPlatformAdapter;

impl AndroidPlatformAdapter {
    pub fn new() -> Self {
        Self
    }

    /// Check if running on Android.
    pub fn is_android() -> bool {
        // Check for Android-specific files
        std::path::Path::new("/system/build.prop").exists()
            || std::path::Path::new("/vendor/build.prop").exists()
    }
}

#[async_trait]
impl PlatformAdapter for AndroidPlatformAdapter {
    fn name(&self) -> &str {
        "android"
    }

    fn is_available(&self) -> bool {
        Self::is_android()
    }

    fn capabilities(&self) -> PlatformCapabilities {
        PlatformCapabilities {
            has_dbus: false,
            has_systemd: false,
            has_polkit: false,
            has_binder: true,
            platform_name: "android".to_string(),
        }
    }

    async fn list_services(&self) -> Result<Vec<ServiceInfo>> {
        // Android uses 'getprop' and 'dumpsys' for service discovery
        let output = tokio::process::Command::new("dumpsys")
            .arg("activity")
            .arg("services")
            .output()
            .await?;

        let stdout = String::from_utf8_lossy(&output.stdout);
        let mut services = Vec::new();

        for line in stdout.lines() {
            if line.contains("ServiceRecord") {
                // Parse service record
                if let Some(name) = line.split('{').next() {
                    let name = name.trim().to_string();
                    if !name.is_empty() {
                        services.push(ServiceInfo {
                            name,
                            status: ServiceStatus::Running,
                            description: String::new(),
                            pid: None,
                        });
                    }
                }
            }
        }

        Ok(services)
    }

    async fn service_status(&self, name: &str) -> Result<ServiceInfo> {
        // Use 'getprop' to check service status
        let output = tokio::process::Command::new("getprop")
            .arg(format!("init.svc.{}", name))
            .output()
            .await?;

        let status = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(ServiceInfo {
            name: name.to_string(),
            status: match status.as_str() {
                "running" => ServiceStatus::Running,
                "stopped" => ServiceStatus::Stopped,
                "restarting" => ServiceStatus::Running,
                _ => ServiceStatus::Unknown,
            },
            description: String::new(),
            pid: None,
        })
    }

    async fn service_start(&self, name: &str) -> Result<()> {
        // Android uses 'setprop' to control services
        let output = tokio::process::Command::new("setprop")
            .arg(format!("ctl.start {}", name))
            .arg(name)
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!("Failed to start service: {}", name);
        }
        Ok(())
    }

    async fn service_stop(&self, name: &str) -> Result<()> {
        let output = tokio::process::Command::new("setprop")
            .arg(format!("ctl.stop {}", name))
            .arg(name)
            .output()
            .await?;

        if !output.status.success() {
            anyhow::bail!("Failed to stop service: {}", name);
        }
        Ok(())
    }

    async fn service_restart(&self, name: &str) -> Result<()> {
        self.service_stop(name).await?;
        tokio::time::sleep(std::time::Duration::from_millis(100)).await;
        self.service_start(name).await
    }

    async fn hostname(&self) -> Result<String> {
        let output = tokio::process::Command::new("getprop")
            .arg("net.hostname")
            .output()
            .await?;

        Ok(String::from_utf8_lossy(&output.stdout).trim().to_string())
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
        // On Android, root access is typically via 'su'
        let output = tokio::process::Command::new("su")
            .arg("-c")
            .arg("id")
            .output()
            .await;

        match output {
            Ok(o) if o.status.success() => {
                tracing::info!("Root access available via su");
                Ok(())
            }
            _ => {
                // Try ADB root
                let output = tokio::process::Command::new("adb")
                    .arg("root")
                    .output()
                    .await;

                match output {
                    Ok(o) if o.status.success() => {
                        tracing::info!("ADB root available");
                        Ok(())
                    }
                    _ => anyhow::bail!("No root access available (su/adb root)"),
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_android_detection() {
        // This test only makes sense on Android
        let is_android = AndroidPlatformAdapter::is_android();
        let _ = is_android; // Just verify it doesn't panic
    }

    #[test]
    fn test_android_adapter_capabilities() {
        let adapter = AndroidPlatformAdapter::new();
        let caps = adapter.capabilities();
        assert!(caps.has_binder);
        assert_eq!(caps.platform_name, "android");
    }
}
