/// Tests socket peer credential enforcement.
/// Requires: the installed per-user daemon and its official XDG runtime socket.
#[cfg(test)]
#[allow(clippy::module_inception)]
mod socket_auth {
    use std::os::unix::fs::{FileTypeExt, MetadataExt, PermissionsExt};
    use std::os::unix::net::UnixStream;

    /// Verify the user-private socket exists, is mode 0600, and is owned by this user.
    #[test]
    #[cfg_attr(not(feature = "integration-tests"), ignore)]
    fn socket_exists_and_restricted() {
        let _runtime_guard = crate::installed_runtime_lock();
        let path = crate::official_user_socket();
        let meta = std::fs::metadata(&path)
            .unwrap_or_else(|error| panic!("socket {} should exist: {error}", path.display()));
        assert!(meta.file_type().is_socket(), "Should be a Unix socket");

        let mode = meta.permissions().mode();
        assert_eq!(
            mode & 0o777,
            0o600,
            "user socket should be 0600, got {:o}",
            mode & 0o777
        );
        assert_eq!(
            meta.uid(),
            unsafe { libc::geteuid() },
            "user socket should be owned by the invoking user"
        );
    }

    /// Verify the invoking user can connect to its private socket.
    #[test]
    #[cfg_attr(not(feature = "integration-tests"), ignore)]
    fn can_connect_to_socket() {
        let _runtime_guard = crate::installed_runtime_lock();
        let path = crate::official_user_socket();
        let _stream = UnixStream::connect(&path).unwrap_or_else(|error| {
            panic!("should connect to user socket {}: {error}", path.display())
        });
    }
}
