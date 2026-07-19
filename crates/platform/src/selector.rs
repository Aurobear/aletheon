//! OS detection and backend selection.

use crate::manifest::HostCapabilityManifest;
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
use crate::manifest::{FeatureState, HostFeature};

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
    native_backend()
}

#[cfg(target_os = "linux")]
fn native_backend() -> Box<dyn Backend> {
    Box::new(crate::backend::linux::LinuxBackend::new())
}

#[cfg(target_os = "windows")]
fn native_backend() -> Box<dyn Backend> {
    Box::new(crate::backend::windows::WindowsBackend::new())
}

#[cfg(target_os = "macos")]
fn native_backend() -> Box<dyn Backend> {
    Box::new(crate::backend::macos::MacOSBackend::new())
}

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
fn native_backend() -> Box<dyn Backend> {
    Box::new(UnsupportedBackend)
}

/// A backend provides a capability manifest. Production backends also
/// implement the platform host traits (ProcessHost, FilesystemHost, …).
pub trait Backend: Send + Sync {
    fn probe(&self) -> HostCapabilityManifest;
}

#[cfg(target_os = "linux")]
impl Backend for crate::backend::linux::LinuxBackend {
    fn probe(&self) -> HostCapabilityManifest {
        crate::backend::linux::LinuxBackend::probe(self)
    }
}

#[cfg(target_os = "windows")]
impl Backend for crate::backend::windows::WindowsBackend {
    fn probe(&self) -> HostCapabilityManifest {
        crate::backend::windows::WindowsBackend::probe(self)
    }
}

#[cfg(target_os = "macos")]
impl Backend for crate::backend::macos::MacOSBackend {
    fn probe(&self) -> HostCapabilityManifest {
        crate::backend::macos::MacOSBackend::probe(self)
    }
}

/// Single fail-closed backend for unsupported compilation targets.
#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
struct UnsupportedBackend;

#[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
impl Backend for UnsupportedBackend {
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
            platform: "unknown".into(),
            os_version: "unsupported".into(),
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
    use crate::manifest::FeatureState;
    #[cfg(not(target_os = "linux"))]
    use crate::manifest::HostFeature;

    #[test]
    fn detect_platform_returns_known_variant() {
        let p = detect_platform();
        assert!(matches!(
            p,
            HostPlatform::Linux
                | HostPlatform::Windows
                | HostPlatform::MacOS
                | HostPlatform::Unknown
        ));
    }

    #[test]
    fn select_backend_returns_probe() {
        let backend = select_backend();
        let manifest = backend.probe();
        assert!(!manifest.platform.is_empty());

        #[cfg(target_os = "linux")]
        {
            assert_eq!(manifest.platform, "linux");
            assert_ne!(manifest.os_version, "stub");
            assert!(manifest.probed_at_unix_ms > 0);
            assert!(manifest
                .features
                .iter()
                .any(|(_, state)| *state != FeatureState::Unsupported));
        }

        #[cfg(not(target_os = "linux"))]
        assert_eq!(
            manifest.state_of(&HostFeature::ProcessTree),
            FeatureState::Unsupported
        );
    }
}
