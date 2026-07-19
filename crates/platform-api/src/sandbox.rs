//! SandboxHost — process/filesystem/network isolation.

use crate::error::HostError;
use crate::receipt::HostReceipt;
use async_trait::async_trait;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SandboxStrength {
    None,
    ProcessJob,
    Namespace,
    Seccomp,
    Landlock,
    AppContainer,
}

#[derive(Clone, Debug)]
pub struct SandboxProfile {
    pub strengths: Vec<SandboxStrength>,
    pub readonly_root: bool,
    pub network_disabled: bool,
    pub writable_paths: Vec<crate::path::HostPath>,
}

#[async_trait]
pub trait SandboxHost: Send + Sync {
    async fn probe(&self) -> Vec<SandboxStrength>;
    async fn apply(&self, profile: &SandboxProfile) -> Result<HostReceipt, HostError>;
}
