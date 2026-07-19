//! Linux capability probe — detects cgroup v2, pidfd, inotify, PTY, systemd.

use crate::manifest::{FeatureState, HostCapabilityManifest, HostFeature};

pub struct LinuxBackend;

impl Default for LinuxBackend {
    fn default() -> Self {
        Self
    }
}

impl LinuxBackend {
    pub fn new() -> Self {
        Self
    }

    pub fn probe(&self) -> HostCapabilityManifest {
        let features = vec![
            // ProcessTree: pidfd support (Linux 5.3+)
            (HostFeature::ProcessTree, Self::check_pidfd()),
            // FilesystemConfinement: only claim support when Landlock is exposed.
            (
                HostFeature::FilesystemConfinement,
                Self::check_path("/sys/kernel/security/landlock"),
            ),
            // FileWatching: inotify
            (HostFeature::FileWatching, Self::check_inotify()),
            (HostFeature::Pty, Self::check_path("/dev/ptmx")),
            // ServiceManagement: systemd via D-Bus
            (HostFeature::ServiceManagement, Self::check_systemd()),
            // Sandbox features
            (
                HostFeature::SandboxNamespace,
                Self::check_path("/proc/self/ns/user"),
            ),
            (
                HostFeature::SandboxSeccomp,
                Self::check_path("/proc/sys/kernel/seccomp/actions_avail"),
            ),
            // Desktop: not probed yet (H4)
            (HostFeature::DesktopAccessibility, FeatureState::Unsupported),
            (HostFeature::DesktopInput, FeatureState::Unsupported),
            (HostFeature::MediaCamera, FeatureState::Unsupported),
            (HostFeature::CredentialVault, FeatureState::Unsupported),
        ];

        HostCapabilityManifest {
            platform: "linux".into(),
            os_version: Self::os_release(),
            arch: std::env::consts::ARCH.into(),
            backend_version: env!("CARGO_PKG_VERSION").into(),
            features,
            probed_at_unix_ms: std::time::SystemTime::now()
                .duration_since(std::time::UNIX_EPOCH)
                .unwrap_or_default()
                .as_millis() as u64,
        }
    }

    fn check_pidfd() -> FeatureState {
        // Probe the syscall rather than inferring support from /proc.
        let fd = unsafe { libc::syscall(libc::SYS_pidfd_open, std::process::id(), 0) };
        if fd >= 0 {
            unsafe {
                libc::close(fd as i32);
            }
            return FeatureState::Available;
        }

        match std::io::Error::last_os_error().raw_os_error() {
            Some(libc::ENOSYS) => FeatureState::Unsupported,
            Some(libc::EPERM) | Some(libc::EACCES) => FeatureState::PermissionRequired,
            _ => FeatureState::Unavailable,
        }
    }

    fn check_inotify() -> FeatureState {
        if std::path::Path::new("/proc/sys/fs/inotify/max_user_watches").exists() {
            FeatureState::Available
        } else {
            FeatureState::Unavailable
        }
    }

    fn check_systemd() -> FeatureState {
        if std::path::Path::new("/run/systemd/system").exists() {
            FeatureState::Available
        } else {
            FeatureState::Unavailable
        }
    }

    fn check_path(path: &str) -> FeatureState {
        if std::path::Path::new(path).exists() {
            FeatureState::Available
        } else {
            FeatureState::Unavailable
        }
    }

    fn os_release() -> String {
        std::fs::read_to_string("/etc/os-release")
            .ok()
            .and_then(|s| {
                s.lines().find(|l| l.starts_with("PRETTY_NAME=")).map(|l| {
                    l.trim_start_matches("PRETTY_NAME=")
                        .trim_matches('"')
                        .to_string()
                })
            })
            .unwrap_or_else(|| "Linux (unknown)".into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn probe_returns_manifest() {
        let backend = LinuxBackend::new();
        let manifest = backend.probe();
        assert_eq!(manifest.platform, "linux");
        assert!(!manifest.arch.is_empty());
        assert!(!manifest.features.is_empty());
    }

    #[test]
    fn process_tree_feature_is_probed() {
        let backend = LinuxBackend::new();
        let m = backend.probe();
        let state = m.state_of(&HostFeature::ProcessTree);
        assert!(state == FeatureState::Available || state == FeatureState::Unsupported);
    }

    #[test]
    fn desktop_features_are_unsupported_yet() {
        let backend = LinuxBackend::new();
        let m = backend.probe();
        assert_eq!(
            m.state_of(&HostFeature::DesktopAccessibility),
            FeatureState::Unsupported
        );
    }
}
