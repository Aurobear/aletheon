/// Tests daemon lifecycle: restart, session persistence.
/// Requires: systemctl and the installed per-user aletheon.service.
#[cfg(test)]
#[allow(clippy::module_inception)]
mod daemon_lifecycle {
    use std::os::unix::fs::{FileTypeExt, PermissionsExt};
    use std::os::unix::net::UnixStream;
    use std::process::Command;
    use std::time::{Duration, Instant};

    /// Verify the installed per-user daemon restarts and recreates a usable socket.
    #[test]
    #[cfg_attr(not(feature = "integration-tests"), ignore)]
    fn daemon_restarts_cleanly() {
        let _runtime_guard = crate::installed_runtime_lock();
        let socket = crate::official_user_socket();
        let restart = Command::new("systemctl")
            .args(["--user", "restart", "aletheon.service"])
            .output()
            .expect("Failed to restart daemon");
        assert!(
            restart.status.success(),
            "Daemon restart failed: {:?}",
            String::from_utf8_lossy(&restart.stderr)
        );

        let deadline = Instant::now() + Duration::from_secs(30);
        loop {
            let active = Command::new("systemctl")
                .args(["--user", "is-active", "--quiet", "aletheon.service"])
                .status()
                .is_ok_and(|status| status.success());
            let socket_ready = std::fs::metadata(&socket).is_ok_and(|metadata| {
                metadata.file_type().is_socket()
                    && metadata.permissions().mode() & 0o777 == 0o600
                    && UnixStream::connect(&socket).is_ok()
            });
            if active && socket_ready {
                break;
            }
            if Instant::now() >= deadline {
                let status = Command::new("systemctl")
                    .args(["--user", "status", "aletheon.service", "--no-pager"])
                    .output()
                    .expect("Failed to inspect daemon status");
                panic!(
                    "user daemon did not become ready at {}:\n{}\n{}",
                    socket.display(),
                    String::from_utf8_lossy(&status.stdout),
                    String::from_utf8_lossy(&status.stderr)
                );
            }
            std::thread::sleep(Duration::from_millis(100));
        }
    }

    /// Verify the daemon binary exists and is executable.
    #[test]
    #[cfg_attr(not(feature = "integration-tests"), ignore)]
    fn binary_is_installed() {
        let _runtime_guard = crate::installed_runtime_lock();
        let path = std::path::Path::new("/usr/bin/aletheon");
        let metadata = std::fs::metadata(path).expect("/usr/bin/aletheon should be installed");
        assert!(metadata.is_file(), "/usr/bin/aletheon should be a file");
        assert_ne!(
            metadata.permissions().mode() & 0o111,
            0,
            "/usr/bin/aletheon should be executable"
        );
    }
}
