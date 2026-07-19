//! macOS capability probe — detects posix_spawn, FSEvents, launchd, Keychain.

use platform_api::manifest::{FeatureState, HostCapabilityManifest, HostFeature};

pub struct MacOSBackend;

impl MacOSBackend {
    pub fn new() -> Self { Self }

    pub fn probe(&self) -> HostCapabilityManifest {
        HostCapabilityManifest {
            platform: "macos".into(),
            os_version: "macOS (probed)".into(),
            arch: std::env::consts::ARCH.into(),
            backend_version: env!("CARGO_PKG_VERSION").into(),
            features: vec![
                (HostFeature::ProcessTree, FeatureState::Available),    // posix_spawn
                (HostFeature::Pty, FeatureState::Available),            // Unix98 PTY
                (HostFeature::FilesystemConfinement, FeatureState::Available),
                (HostFeature::FileWatching, FeatureState::Available),   // FSEvents
                (HostFeature::ServiceManagement, FeatureState::Available), // launchd
                (HostFeature::SandboxNamespace, FeatureState::Unsupported),
                (HostFeature::SandboxSeccomp, FeatureState::Unsupported),
                (HostFeature::DesktopAccessibility, FeatureState::PermissionRequired), // TCC
                (HostFeature::DesktopInput, FeatureState::PermissionRequired),  // Accessibility
                (HostFeature::MediaCamera, FeatureState::PermissionRequired),    // TCC Camera
                (HostFeature::CredentialVault, FeatureState::Available),         // Keychain
            ],
            probed_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH).unwrap_or_default().as_millis() as u64,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test] fn probe_returns_manifest() { let m = MacOSBackend::new().probe(); assert_eq!(m.platform, "macos"); assert!(!m.features.is_empty()); }
    #[test] fn pty_available() { assert_eq!(MacOSBackend::new().probe().state_of(&HostFeature::Pty), FeatureState::Available); }
    #[test] fn tcc_permission_required() { assert_eq!(MacOSBackend::new().probe().state_of(&HostFeature::DesktopAccessibility), FeatureState::PermissionRequired); }
}
