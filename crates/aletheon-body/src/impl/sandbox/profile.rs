//! Sandbox profile — declarative descriptor of what a sandbox allows.

use std::path::{Path, PathBuf};

/// A declarative profile that describes the access boundaries of a sandbox.
///
/// This is additive metadata; it does **not** change [`SandboxExecutor`]'s
/// existing behaviour.  Consumers can inspect a profile to decide whether
/// an action is permitted before handing it to the executor.
#[derive(Debug, Clone, Default)]
pub struct SandboxProfile {
    /// Directories that may be read inside the sandbox.
    pub read_roots: Vec<PathBuf>,
    /// Directories that may be written inside the sandbox.
    pub write_roots: Vec<PathBuf>,
    /// Paths that are explicitly denied (highest priority).
    pub deny_paths: Vec<PathBuf>,
    /// Whether outbound network access is allowed.
    pub network_enabled: bool,
    /// Extra environment variables to inject into the sandbox.
    pub env_vars: Vec<(String, String)>,
}

impl SandboxProfile {
    /// Create an empty profile with nothing allowed.
    pub fn new() -> Self {
        Self::default()
    }

    /// Returns `true` if `path` falls under any write root and is not denied.
    pub fn allows_write(&self, path: &Path) -> bool {
        if self.is_denied(path) {
            return false;
        }
        self.write_roots.iter().any(|root| path.starts_with(root))
    }

    /// Returns `true` if `path` falls under any read root (or write root, since
    /// write implies read) and is not denied.
    pub fn allows_read(&self, path: &Path) -> bool {
        if self.is_denied(path) {
            return false;
        }
        self.read_roots.iter().any(|root| path.starts_with(root))
            || self.write_roots.iter().any(|root| path.starts_with(root))
    }

    /// Returns `true` if outbound network access is permitted.
    pub fn allows_network(&self) -> bool {
        self.network_enabled
    }

    /// Check whether `path` is in the deny list.
    fn is_denied(&self, path: &Path) -> bool {
        self.deny_paths.iter().any(|denied| path.starts_with(denied))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;

    #[test]
    fn write_root_allows_write() {
        let profile = SandboxProfile {
            write_roots: vec![PathBuf::from("/workspace")],
            ..Default::default()
        };
        assert!(profile.allows_write(Path::new("/workspace/src/main.rs")));
        assert!(!profile.allows_write(Path::new("/etc/passwd")));
    }

    #[test]
    fn deny_overrides_write() {
        let profile = SandboxProfile {
            write_roots: vec![PathBuf::from("/workspace")],
            deny_paths: vec![PathBuf::from("/workspace/secret")],
            ..Default::default()
        };
        assert!(profile.allows_write(Path::new("/workspace/src/main.rs")));
        assert!(!profile.allows_write(Path::new("/workspace/secret/key.pem")));
    }

    #[test]
    fn read_includes_write_roots() {
        let profile = SandboxProfile {
            read_roots: vec![PathBuf::from("/usr")],
            write_roots: vec![PathBuf::from("/workspace")],
            ..Default::default()
        };
        assert!(profile.allows_read(Path::new("/usr/lib/libc.so")));
        assert!(profile.allows_read(Path::new("/workspace/file.txt")));
        assert!(!profile.allows_read(Path::new("/etc/shadow")));
    }

    #[test]
    fn network_disabled() {
        let profile = SandboxProfile {
            network_enabled: false,
            ..Default::default()
        };
        assert!(!profile.allows_network());
    }

    #[test]
    fn network_enabled() {
        let profile = SandboxProfile {
            network_enabled: true,
            ..Default::default()
        };
        assert!(profile.allows_network());
    }
}
