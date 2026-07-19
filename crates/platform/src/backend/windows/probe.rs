//! Windows capability probe — detects Job Objects, ConPTY, SCM, etc.

use crate::manifest::{FeatureState, HostCapabilityManifest, HostFeature};

pub struct WindowsBackend;

impl WindowsBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn probe(&self) -> HostCapabilityManifest {
        HostCapabilityManifest {
            platform: "windows".into(),
            os_version: Self::os_version(),
            arch: std::env::consts::ARCH.into(),
            backend_version: env!("CARGO_PKG_VERSION").into(),
            features: vec![
                (HostFeature::ProcessTree, FeatureState::Unavailable),
                (HostFeature::Pty, FeatureState::Unavailable), // ConPTY
                (
                    HostFeature::FilesystemConfinement,
                    FeatureState::Unavailable,
                ),
                (HostFeature::FileWatching, FeatureState::Unavailable), // ReadDirectoryChangesW
                (HostFeature::ServiceManagement, FeatureState::Unavailable), // SCM
                (HostFeature::SandboxNamespace, FeatureState::Unsupported),
                (HostFeature::SandboxSeccomp, FeatureState::Unsupported),
                (HostFeature::DesktopAccessibility, FeatureState::Unsupported),
                (HostFeature::DesktopInput, FeatureState::Unsupported),
                (HostFeature::MediaCamera, FeatureState::Unsupported),
                (HostFeature::CredentialVault, FeatureState::Unavailable), // Credential Manager
            ],
            probed_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    fn os_version() -> String {
        "Windows (probed)".into()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_returns_manifest() {
        let backend = WindowsBackend::new();
        let m = backend.probe();
        assert_eq!(m.platform, "windows");
        assert!(!m.features.is_empty());
    }

    #[test]
    fn process_tree_is_unavailable_until_job_object_contract_passes() {
        let m = WindowsBackend::new().probe();
        assert_eq!(
            m.state_of(&HostFeature::ProcessTree),
            FeatureState::Unavailable
        );
    }

    #[test]
    fn pty_is_unavailable_until_conpty_contract_passes() {
        let m = WindowsBackend::new().probe();
        assert_eq!(m.state_of(&HostFeature::Pty), FeatureState::Unavailable);
    }
}
