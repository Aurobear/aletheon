//! Host operating-system capabilities and their platform-specific implementations.
//!
//! This crate owns the stable host contracts, backend selection, and Linux,
//! Windows, and macOS implementations.

pub mod backend;
pub mod desktop;
pub mod error;
pub mod filesystem;
pub mod manifest;
pub mod path;
pub mod process;
pub mod pty;
pub mod receipt;
pub mod registry;
pub mod sandbox;
pub mod selector;
pub mod service;
pub mod structured_patch;

pub use desktop::DesktopHost;
pub use error::{HostError, HostErrorKind};
pub use filesystem::{
    AtomicWrite, EntryMetadata, FilesystemAccess, FilesystemHost, FilesystemScope, FsEvent,
    FsEventStream, RemoveFile, SymlinkPolicy, WriteReceipt,
};
pub use manifest::{FeatureState, HostCapabilityManifest, HostFeature};
pub use path::HostPath;
pub use process::{ProcessHost, ProcessId, ProcessSignal, ProcessSnapshot, SpawnSpec};
pub use pty::{PtyChannel, PtyHost, PtySize};
pub use receipt::HostReceipt;
pub use registry::BackendRegistry;
pub use sandbox::{SandboxHost, SandboxProfile, SandboxStrength};
pub use selector::{detect_platform, select_backend, Backend, HostPlatform};
pub use service::{ServiceHost, ServiceState};

/// Probe the selected host backend.
pub fn probe() -> Result<HostCapabilityManifest, anyhow::Error> {
    Ok(select_backend().probe())
}

/// Open an operation-scoped filesystem backend for the current target.
pub fn open_filesystem(scope: FilesystemScope) -> Result<Box<dyn FilesystemHost>, HostError> {
    #[cfg(target_os = "linux")]
    {
        backend::linux::LinuxFilesystemHost::scoped(scope)
            .map(|host| Box::new(host) as Box<dyn FilesystemHost>)
    }
    #[cfg(target_os = "windows")]
    {
        return backend::windows::WindowsFilesystemHost::scoped(scope)
            .map(|host| Box::new(host) as Box<dyn FilesystemHost>);
    }
    #[cfg(target_os = "macos")]
    {
        return backend::macos::MacOSFilesystemHost::scoped(scope)
            .map(|host| Box::new(host) as Box<dyn FilesystemHost>);
    }
    #[cfg(not(any(target_os = "linux", target_os = "windows", target_os = "macos")))]
    {
        let _ = scope;
        Err(HostError::unsupported("filesystem backend for this OS"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn host_path_round_trip() {
        let hp = HostPath::new(PathBuf::from("/tmp/test"));
        assert_eq!(hp.logical(), "/tmp/test");
    }

    #[test]
    fn host_path_normalises_backslashes() {
        let hp = HostPath::new(PathBuf::from(r"C:\Users\test"));
        assert!(!hp.logical().contains('\\'));
    }

    #[test]
    fn receipt_ok() {
        let r = HostReceipt::ok("test_op", 42);
        assert!(r.success);
    }

    #[test]
    fn receipt_err() {
        let r = HostReceipt::err("test_op", 42, "denied");
        assert!(!r.success);
    }

    #[test]
    fn error_unsupported() {
        let e = HostError::unsupported("pty");
        assert_eq!(e.kind, HostErrorKind::Unsupported("pty".into()));
    }

    #[test]
    fn manifest_feature_default_unsupported() {
        let m = HostCapabilityManifest {
            platform: "test".into(),
            os_version: "0".into(),
            arch: "x86_64".into(),
            backend_version: "0.1".into(),
            features: vec![],
            probed_at_unix_ms: 0,
        };
        assert_eq!(m.state_of(&HostFeature::Pty), FeatureState::Unsupported);
    }
}
