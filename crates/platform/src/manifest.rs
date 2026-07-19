//! HostCapabilityManifest — runtime-probed capability matrix.

use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum FeatureState {
    Available,
    Unavailable,
    PermissionRequired,
    Degraded,
    Unsupported,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum HostFeature {
    ProcessTree,
    Pty,
    FilesystemConfinement,
    FileWatching,
    ServiceManagement,
    SandboxNamespace,
    SandboxSeccomp,
    DesktopAccessibility,
    DesktopInput,
    MediaCamera,
    CredentialVault,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HostCapabilityManifest {
    pub platform: String,
    pub os_version: String,
    pub arch: String,
    pub backend_version: String,
    pub features: Vec<(HostFeature, FeatureState)>,
    pub probed_at_unix_ms: u64,
}

impl HostCapabilityManifest {
    pub fn state_of(&self, feature: &HostFeature) -> FeatureState {
        self.features
            .iter()
            .find(|(f, _)| f == feature)
            .map(|(_, s)| s.clone())
            .unwrap_or(FeatureState::Unsupported)
    }
}
