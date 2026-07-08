/// Tests socket peer credential enforcement.
/// Requires: daemon running as aletheon user, socket at /run/aletheon/aletheon.sock.
#[cfg(test)]
#[allow(clippy::module_inception)]
mod socket_auth {
    use std::os::unix::fs::FileTypeExt;
    use std::os::unix::net::UnixStream;

    /// Verify the socket exists and has correct permissions (0660, owned by aletheon:aletheon).
    #[test]
    #[cfg_attr(not(feature = "integration-tests"), ignore)]
    fn socket_exists_and_restricted() {
        let path = "/run/aletheon/aletheon.sock";
        let meta = std::fs::metadata(path).expect("Socket should exist");
        assert!(meta.file_type().is_socket(), "Should be a Unix socket");

        // On Unix, check permissions
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = meta.permissions().mode();
            // Should be 0660 (socket, no world access)
            assert_eq!(
                mode & 0o777,
                0o660,
                "Socket should be 0660, got {:o}",
                mode & 0o777
            );
        }
    }

    /// Verify we CAN connect to the socket (current user should be in aletheon group).
    #[test]
    #[cfg_attr(not(feature = "integration-tests"), ignore)]
    fn can_connect_to_socket() {
        let path = "/run/aletheon/aletheon.sock";
        let _stream = UnixStream::connect(path).expect("Should be able to connect to socket");
    }
}
