//! Platform-agnostic host capability types, traits, and error/receipt models.
//! No OS-specific code lives here — backends implement these contracts.

pub mod error;
pub mod path;
pub mod receipt;

pub use error::{HostError, HostErrorKind};
pub use path::HostPath;
pub use receipt::HostReceipt;

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn host_path_round_trip() {
        let hp = HostPath::new(PathBuf::from("/tmp/test"));
        assert_eq!(hp.logical(), "/tmp/test");
        assert_eq!(hp.native().to_string_lossy(), "/tmp/test");
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
        assert_eq!(r.operation, "test_op");
    }

    #[test]
    fn receipt_err() {
        let r = HostReceipt::err("test_op", 42, "denied");
        assert!(!r.success);
        assert_eq!(r.detail.unwrap(), "denied");
    }

    #[test]
    fn error_unsupported() {
        let e = HostError::unsupported("pty");
        assert_eq!(e.kind, HostErrorKind::Unsupported("pty".into()));
    }
}
