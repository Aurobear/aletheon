/// Tests daemon lifecycle: restart, session persistence.
/// Requires: systemctl, aletheon.service installed.
#[cfg(test)]
mod daemon_lifecycle {
    use std::process::Command;

    /// Verify daemon restarts cleanly.
    #[test]
    #[cfg_attr(not(feature = "integration-tests"), ignore)]
    fn daemon_restarts_cleanly() {
        // Restart the service
        let restart = Command::new("sudo")
            .args(["systemctl", "restart", "aletheon.service"])
            .output()
            .expect("Failed to restart daemon");
        assert!(
            restart.status.success(),
            "Daemon restart failed: {:?}",
            String::from_utf8_lossy(&restart.stderr)
        );

        // Wait for it to come up
        std::thread::sleep(std::time::Duration::from_secs(3));

        // Check status
        let status = Command::new("systemctl")
            .args(["is-active", "aletheon.service"])
            .output()
            .expect("Failed to check daemon status");
        assert_eq!(
            String::from_utf8_lossy(&status.stdout).trim(),
            "active",
            "Daemon should be active after restart"
        );
    }

    /// Verify the daemon binary exists and is executable.
    #[test]
    #[cfg_attr(not(feature = "integration-tests"), ignore)]
    fn binary_is_installed() {
        let output = Command::new("which")
            .arg("aletheon")
            .output()
            .expect("which command failed");
        assert!(output.status.success(), "aletheon binary should be on PATH");
    }
}
