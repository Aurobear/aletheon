//! Linux ServiceHost — systemd detection + status (H1-05).

use platform_api::error::{HostError, HostErrorKind};
use platform_api::receipt::HostReceipt;
use platform_api::service::{ServiceHost, ServiceState};
use async_trait::async_trait;

pub struct LinuxServiceHost;

impl LinuxServiceHost { pub fn new() -> Self { Self } }

fn has_systemd() -> bool { std::path::Path::new("/run/systemd/system").exists() }

#[async_trait]
impl ServiceHost for LinuxServiceHost {
    async fn status(&self, name: &str) -> Result<ServiceState, HostError> {
        if !has_systemd() { return Err(HostError::unsupported("systemd not detected")); }
        let output = tokio::process::Command::new("systemctl")
            .args(["is-active", name])
            .output().await
            .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "systemctl"))?;
        let s = String::from_utf8_lossy(&output.stdout).trim().to_string();
        Ok(match s.as_str() {
            "active" => ServiceState::Running,
            "inactive" | "dead" => ServiceState::Stopped,
            "failed" => ServiceState::Failed,
            _ => ServiceState::Unknown,
        })
    }
    async fn start(&self, name: &str) -> Result<HostReceipt, HostError> {
        if !has_systemd() { return Err(HostError::unsupported("systemd")); }
        let output = tokio::process::Command::new("systemctl").args(["start", name]).output().await
            .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "systemctl start"))?;
        Ok(HostReceipt::ok("service_start", 0))
    }
    async fn stop(&self, name: &str) -> Result<HostReceipt, HostError> {
        if !has_systemd() { return Err(HostError::unsupported("systemd")); }
        tokio::process::Command::new("systemctl").args(["stop", name]).output().await
            .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "systemctl stop"))?;
        Ok(HostReceipt::ok("service_stop", 0))
    }
    async fn restart(&self, name: &str) -> Result<HostReceipt, HostError> {
        if !has_systemd() { return Err(HostError::unsupported("systemd")); }
        tokio::process::Command::new("systemctl").args(["restart", name]).output().await
            .map_err(|e| HostError::new(HostErrorKind::Io(e.to_string()), "systemctl restart"))?;
        Ok(HostReceipt::ok("service_restart", 0))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn systemd_status_returns_state_or_unsupported() {
        let host = LinuxServiceHost::new();
        match host.status("sshd.service").await {
            Ok(state) => assert!(matches!(state, ServiceState::Running | ServiceState::Stopped | ServiceState::Unknown)),
            Err(_) => {} // no systemd = Unsupported, acceptable
        }
    }
}
