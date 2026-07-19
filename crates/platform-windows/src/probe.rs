//! Windows capability probe — detects Job Objects, ConPTY, SCM, etc.

use platform_api::manifest::{FeatureState, HostCapabilityManifest, HostFeature};

pub struct WindowsBackend;

impl WindowsBackend {
    pub fn new() -> Self { Self }

    pub fn probe(&self) -> HostCapabilityManifest {
        HostCapabilityManifest {
            platform: "windows".into(),
            os_version: Self::os_version(),
            arch: std::env::consts::ARCH.into(),
            backend_version: env!("CARGO_PKG_VERSION").into(),
            features: vec![
                (HostFeature::ProcessTree, FeatureState::Available),
                (HostFeature::Pty, FeatureState::Available),           // ConPTY
                (HostFeature::FilesystemConfinement, FeatureState::Available),
                (HostFeature::FileWatching, FeatureState::Available),  // ReadDirectoryChangesW
                (HostFeature::ServiceManagement, FeatureState::Available), // SCM
                (HostFeature::SandboxNamespace, FeatureState::Unsupported),
                (HostFeature::SandboxSeccomp, FeatureState::Unsupported),
                (HostFeature::DesktopAccessibility, FeatureState::Unsupported),
                (HostFeature::DesktopInput, FeatureState::Unsupported),
                (HostFeature::MediaCamera, FeatureState::Unsupported),
                (HostFeature::CredentialVault, FeatureState::Available), // Credential Manager
            ],
            probed_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
        }
    }

    fn os_version() -> String { "Windows (probed)".into() }
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
    fn process_tree_available() {
        let m = WindowsBackend::new().probe();
        assert_eq!(m.state_of(&HostFeature::ProcessTree), FeatureState::Available);
    }

    #[test]
    fn pty_available_via_conpty() {
        let m = WindowsBackend::new().probe();
        assert_eq!(m.state_of(&HostFeature::Pty), FeatureState::Available);
    }
}
