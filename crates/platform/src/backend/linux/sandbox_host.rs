//! Linux SandboxHost — namespace/seccomp/cgroup fail-closed (H1-06).

use crate::error::HostError;
use crate::receipt::HostReceipt;
use crate::sandbox::{SandboxHost, SandboxProfile, SandboxStrength};
use async_trait::async_trait;

pub struct LinuxSandboxHost;

impl LinuxSandboxHost {
    pub fn new() -> Self {
        Self
    }
}

#[async_trait]
impl SandboxHost for LinuxSandboxHost {
    async fn probe(&self) -> Vec<SandboxStrength> {
        let mut strengths = Vec::new();
        if std::path::Path::new("/proc/self/ns/user").exists() {
            strengths.push(SandboxStrength::Namespace);
        }
        if std::path::Path::new("/proc/sys/kernel/seccomp/actions_avail").exists() {
            strengths.push(SandboxStrength::Seccomp);
        }
        if std::path::Path::new("/sys/kernel/security/landlock").exists() {
            strengths.push(SandboxStrength::Landlock);
        }
        if strengths.is_empty() {
            strengths.push(SandboxStrength::None);
        }
        strengths
    }
    async fn apply(&self, _profile: &SandboxProfile) -> Result<HostReceipt, HostError> {
        Err(HostError::unsupported("sandbox apply"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn sandbox_host_probe_returns_strengths() {
        let host = LinuxSandboxHost::new();
        let strengths = host.probe().await;
        assert!(!strengths.is_empty());
        assert_eq!(
            strengths.contains(&SandboxStrength::Seccomp),
            std::path::Path::new("/proc/sys/kernel/seccomp/actions_avail").exists()
        );
    }

    #[tokio::test]
    async fn sandbox_host_apply_unimplemented() {
        let host = LinuxSandboxHost::new();
        let result = host
            .apply(&SandboxProfile {
                strengths: vec![],
                readonly_root: true,
                network_disabled: true,
                writable_paths: vec![],
            })
            .await;
        assert!(result.is_err());
    }
}
