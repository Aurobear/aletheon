//! Platform-agnostic host capability types, traits, and error/receipt models.
//! No OS-specific code lives here — backends implement these contracts.

pub mod desktop;
pub mod error;
pub mod filesystem;
pub mod manifest;
pub mod path;
pub mod process;
pub mod pty;
pub mod receipt;
pub mod sandbox;
pub mod structured_patch;
pub mod service;

pub use desktop::DesktopHost;
pub use error::{HostError, HostErrorKind};
pub use filesystem::{AtomicWrite, EntryMetadata, FsEvent, FsEventStream, FilesystemHost, WriteReceipt};
pub use manifest::{FeatureState, HostCapabilityManifest, HostFeature};
pub use path::HostPath;
pub use process::{ProcessHost, ProcessId, ProcessSignal, ProcessSnapshot, SpawnSpec};
pub use pty::{PtyChannel, PtyHost, PtySize};
pub use receipt::HostReceipt;
pub use sandbox::{SandboxHost, SandboxProfile, SandboxStrength};
pub use service::{ServiceHost, ServiceState};

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
        assert_eq!(
            m.state_of(&HostFeature::Pty),
            FeatureState::Unsupported
        );
    }
}
