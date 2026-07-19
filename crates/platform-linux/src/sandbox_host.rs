//! Linux SandboxHost — namespace/seccomp/cgroup fail-closed (H1-06).

use platform_api::error::HostError;
use platform_api::receipt::HostReceipt;
use platform_api::sandbox::{SandboxHost, SandboxProfile, SandboxStrength};
use async_trait::async_trait;

pub struct LinuxSandboxHost;

impl LinuxSandboxHost {
    pub fn new() -> Self { Self }
}

#[async_trait]
impl SandboxHost for LinuxSandboxHost {
    async fn probe(&self) -> Vec<SandboxStrength> {
        vec![SandboxStrength::Namespace, SandboxStrength::Seccomp]
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
        assert!(strengths.contains(&SandboxStrength::Namespace));
    }

    #[tokio::test]
    async fn sandbox_host_apply_unimplemented() {
        let host = LinuxSandboxHost::new();
        let result = host.apply(&SandboxProfile {
            strengths: vec![],
            readonly_root: true,
            network_disabled: true,
            writable_paths: vec![],
        }).await;
        assert!(result.is_err());
    }
}
