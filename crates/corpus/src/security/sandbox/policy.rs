use std::path::PathBuf;

/// Default filesystem access mode for the sandbox root.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FsDefault {
    /// The entire filesystem is read-only; individual roots are made writable.
    ReadOnly,
    /// The entire filesystem is writable; individual roots are made read-only.
    Writable,
}

impl Default for FsDefault {
    fn default() -> Self {
        Self::ReadOnly
    }
}

/// A directory tree that should be writable inside the sandbox,
/// with an explicit list of sub-paths that must remain read-only.
#[derive(Debug, Clone, Default)]
pub struct WritableRoot {
    /// Absolute path to the writable root.
    pub path: PathBuf,
    /// Sub-paths (relative to `path`) that must be re-protected as read-only
    /// even though the parent root is writable.
    pub read_only_subpaths: Vec<PathBuf>,
}

impl WritableRoot {
    /// Create a new writable root with no read-only sub-paths.
    pub fn new(path: impl Into<PathBuf>) -> Self {
        Self {
            path: path.into(),
            read_only_subpaths: Vec::new(),
        }
    }

    /// Builder-style method to add a read-only sub-path.
    pub fn with_read_only(mut self, subpath: impl Into<PathBuf>) -> Self {
        self.read_only_subpaths.push(subpath.into());
        self
    }
}

/// Filesystem policy that describes the desired isolation layout.
///
/// The policy is consumed by [`BwrapBuilder`](super::bwrap_builder::BwrapBuilder)
/// to produce bubblewrap CLI arguments.
#[derive(Debug, Clone, Default)]
pub struct FilesystemPolicy {
    /// Default access mode for the filesystem root.
    pub default: FsDefault,
    /// Directory trees that should be writable (with re-protected sub-paths).
    pub writable_roots: Vec<WritableRoot>,
    /// Basenames of directories that contain sensitive metadata and must be
    /// re-protected as read-only even inside writable roots (e.g. `.git`,
    /// `.agents`).
    pub protected_metadata: Vec<String>,
    /// Glob patterns whose matching files should be masked (bound to
    /// `/dev/null`) so they are unreadable inside the sandbox.
    pub unreadable_globs: Vec<String>,
}

impl FilesystemPolicy {
    /// Create a policy with the given default mode and no roots or masks.
    pub fn new(default: FsDefault) -> Self {
        Self {
            default,
            ..Default::default()
        }
    }

    /// Convenience: a strict read-only policy suitable for code-generation
    /// tasks that only need write access to a single working directory.
    pub fn strict_read_only(work_dir: impl Into<PathBuf>) -> Self {
        Self {
            default: FsDefault::ReadOnly,
            writable_roots: vec![WritableRoot::new(work_dir)],
            protected_metadata: vec![".git".into(), ".agents".into()],
            unreadable_globs: vec!["**/*.env".into(), "**/*.key".into(), "**/*.pem".into()],
        }
    }

    /// Returns `true` if the default mode is read-only.
    pub fn is_read_only_default(&self) -> bool {
        self.default == FsDefault::ReadOnly
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_fs_default_is_read_only() {
        assert_eq!(FsDefault::default(), FsDefault::ReadOnly);
    }

    #[test]
    fn test_writable_root_builder() {
        let root = WritableRoot::new("/home/user/project")
            .with_read_only(".git")
            .with_read_only(".env");
        assert_eq!(root.path, PathBuf::from("/home/user/project"));
        assert_eq!(root.read_only_subpaths.len(), 2);
        assert_eq!(root.read_only_subpaths[0], PathBuf::from(".git"));
    }

    #[test]
    fn test_policy_creation_read_only() {
        let policy = FilesystemPolicy::new(FsDefault::ReadOnly);
        assert!(policy.is_read_only_default());
        assert!(policy.writable_roots.is_empty());
        assert!(policy.protected_metadata.is_empty());
        assert!(policy.unreadable_globs.is_empty());
    }

    #[test]
    fn test_policy_strict_read_only() {
        let policy = FilesystemPolicy::strict_read_only("/work");
        assert_eq!(policy.default, FsDefault::ReadOnly);
        assert_eq!(policy.writable_roots.len(), 1);
        assert_eq!(policy.writable_roots[0].path, PathBuf::from("/work"));
        assert!(policy.protected_metadata.contains(&".git".to_string()));
        assert!(!policy.unreadable_globs.is_empty());
    }

    #[test]
    fn test_policy_default_is_empty() {
        let policy = FilesystemPolicy::default();
        assert_eq!(policy.default, FsDefault::ReadOnly);
        assert!(policy.writable_roots.is_empty());
    }
}
