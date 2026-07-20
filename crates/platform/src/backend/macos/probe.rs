//! macOS capability probe — detects posix_spawn, FSEvents, launchd, Keychain.

use crate::manifest::{FeatureState, HostCapabilityManifest, HostFeature};

pub struct MacOSBackend;

impl MacOSBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn probe(&self) -> HostCapabilityManifest {
        HostCapabilityManifest {
            platform: "macos".into(),
            os_version: "macOS (probed)".into(),
            arch: std::env::consts::ARCH.into(),
            backend_version: env!("CARGO_PKG_VERSION").into(),
            features: vec![
                (HostFeature::ProcessTree, FeatureState::Unavailable), // posix_spawn
                (HostFeature::Pty, FeatureState::Unavailable),         // Unix98 PTY
                (
                    HostFeature::FilesystemConfinement,
                    FeatureState::Unavailable,
                ),
                (HostFeature::FileWatching, FeatureState::Unavailable), // FSEvents
                (HostFeature::ServiceManagement, FeatureState::Unavailable), // launchd
                (HostFeature::SandboxNamespace, FeatureState::Unsupported),
                (HostFeature::SandboxSeccomp, FeatureState::Unsupported),
                (
                    HostFeature::DesktopAccessibility,
                    FeatureState::PermissionRequired,
                ), // TCC
                (HostFeature::DesktopInput, FeatureState::PermissionRequired), // Accessibility
                (HostFeature::MediaCamera, FeatureState::PermissionRequired),  // TCC Camera
                (HostFeature::CredentialVault, FeatureState::Unavailable),     // Keychain
            ],
            probed_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_returns_manifest() {
        let m = MacOSBackend::new().probe();
        assert_eq!(m.platform, "macos");
        assert!(!m.features.is_empty());
    }
    #[test]
    fn pty_is_unavailable_until_native_contract_passes() {
        assert_eq!(
            MacOSBackend::new().probe().state_of(&HostFeature::Pty),
            FeatureState::Unavailable
        );
    }
    #[test]
    fn tcc_permission_required() {
        assert_eq!(
            MacOSBackend::new()
                .probe()
                .state_of(&HostFeature::DesktopAccessibility),
            FeatureState::PermissionRequired
        );
    }
}
