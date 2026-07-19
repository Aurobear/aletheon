//! OS detection and backend selection.

use platform_api::manifest::{FeatureState, HostCapabilityManifest, HostFeature};

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum HostPlatform {
    Linux,
    Windows,
    MacOS,
    Unknown,
}

/// Detect the host platform at runtime.
pub fn detect_platform() -> HostPlatform {
    if cfg!(target_os = "linux") {
        HostPlatform::Linux
    } else if cfg!(target_os = "windows") {
        HostPlatform::Windows
    } else if cfg!(target_os = "macos") {
        HostPlatform::MacOS
    } else {
        HostPlatform::Unknown
    }
}

/// Select the backend matching the host OS.
pub fn select_backend() -> Box<dyn Backend> {
    match detect_platform() {
        HostPlatform::Linux => Box::new(StubBackend::linux()),
        HostPlatform::Windows => Box::new(StubBackend::windows()),
        HostPlatform::MacOS => Box::new(StubBackend::macos()),
        HostPlatform::Unknown => Box::new(StubBackend::unknown()),
    }
}

/// A backend provides a capability manifest. Production backends also
/// implement the platform-api host traits (ProcessHost, FilesystemHost, …).
pub trait Backend: Send + Sync {
    fn probe(&self) -> HostCapabilityManifest;
}

/// Stub backend returns `Unsupported` for every feature until the
/// real platform implementation is wired (H1-01+).
struct StubBackend {
    platform: String,
    os_version: String,
}

impl StubBackend {
    fn linux() -> Self {
        Self {
            platform: "linux".into(),
            os_version: "stub".into(),
        }
    }

    fn windows() -> Self {
        Self {
            platform: "windows".into(),
            os_version: "stub".into(),
        }
    }

    fn macos() -> Self {
        Self {
            platform: "macos".into(),
            os_version: "stub".into(),
        }
    }

    fn unknown() -> Self {
        Self {
            platform: "unknown".into(),
            os_version: "?".into(),
        }
    }
}

impl Backend for StubBackend {
    fn probe(&self) -> HostCapabilityManifest {
        let all = vec![
            HostFeature::ProcessTree,
            HostFeature::Pty,
            HostFeature::FilesystemConfinement,
            HostFeature::FileWatching,
            HostFeature::ServiceManagement,
            HostFeature::SandboxNamespace,
            HostFeature::SandboxSeccomp,
            HostFeature::DesktopAccessibility,
            HostFeature::DesktopInput,
            HostFeature::MediaCamera,
            HostFeature::CredentialVault,
        ];

        HostCapabilityManifest {
            platform: self.platform.clone(),
            os_version: self.os_version.clone(),
            arch: std::env::consts::ARCH.into(),
            backend_version: env!("CARGO_PKG_VERSION").into(),
            features: all
                .into_iter()
                .map(|f| (f, FeatureState::Unsupported))
                .collect(),
            probed_at_unix_ms: 0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_platform_returns_known_variant() {
        let p = detect_platform();
        assert!(matches!(
            p,
            HostPlatform::Linux | HostPlatform::Windows | HostPlatform::MacOS | HostPlatform::Unknown
        ));
    }

    #[test]
    fn select_backend_returns_probe() {
        let backend = select_backend();
        let manifest = backend.probe();
        assert!(!manifest.platform.is_empty());
        assert_eq!(
            manifest.state_of(&HostFeature::ProcessTree),
            FeatureState::Unsupported
        );
    }
}
