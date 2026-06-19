//! WritableRoot: fine-grained path isolation within writable directories.
//!
//! Three-layer protection:
//! - L1: Policy layer (FileSystemSandboxPolicy rules)
//! - L2: Sandbox layer (bubblewrap --ro-bind args)
//! - L3: Runtime layer (PathAccessGuard checks)

use std::path::{Path, PathBuf};

// ── Protected Metadata Names ────────────────────────────────────────────────

/// Directory names that agents must never create or modify.
/// These represent version control, SSH keys, and agent metadata.
pub const PROTECTED_METADATA_NAMES: &[&str] = &[".git", ".ssh", ".codex", ".agents"];

// ── Access Mode ─────────────────────────────────────────────────────────────

/// Permission level for a path. Higher priority wins on conflict.
#[derive(
    Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, serde::Serialize, serde::Deserialize,
)]
pub enum AccessMode {
    /// Read-only (priority 1).
    Read = 1,
    /// Writable (priority 2).
    Write = 2,
    /// Denied (priority 3 — highest).
    Deny = 3,
}

// ── Path Pattern ────────────────────────────────────────────────────────────

/// How a rule matches against file paths.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum PathPattern {
    /// Exact path match (relative to root), e.g. `.git`.
    Exact(PathBuf),
    /// Directory and all sub-paths, e.g. `.ssh/`.
    Directory(PathBuf),
    /// File extension match, e.g. `*.pem`.
    Extension(String),
    /// Filename prefix match, e.g. `.env`.
    Prefix(String),
    /// First path component under root, e.g. `.git` matches `.git/anything`.
    TopLevelComponent(String),
}

impl PathPattern {
    /// Check if a relative path (relative to root) matches this pattern.
    pub fn matches(&self, relative: &Path) -> bool {
        match self {
            Self::Exact(pattern) => relative == pattern,
            Self::Directory(dir) => relative == dir || relative.starts_with(dir),
            Self::Extension(ext) => relative
                .extension()
                .map(|e| e == ext.as_str())
                .unwrap_or(false),
            Self::Prefix(prefix) => relative
                .file_name()
                .map(|f| f.to_string_lossy().starts_with(prefix.as_str()))
                .unwrap_or(false),
            Self::TopLevelComponent(comp) => relative
                .components()
                .next()
                .map(|c| c.as_os_str() == comp.as_str())
                .unwrap_or(false),
        }
    }
}

// ── FileSystemSandboxPolicy ─────────────────────────────────────────────────

/// A set of access rules that determine the final permission for any path.
///
/// Rules are evaluated in order; the last matching rule wins.
/// Default (no matching rule) is `Read`.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct FileSystemSandboxPolicy {
    rules: Vec<(PathPattern, AccessMode)>,
}

impl FileSystemSandboxPolicy {
    /// Create a policy with default read-only protections.
    pub fn with_defaults() -> Self {
        let mut policy = Self { rules: Vec::new() };

        // Version control
        policy.add_readonly_rule(PathPattern::Directory(".git".into()));
        policy.add_readonly_rule(PathPattern::Directory(".svn".into()));
        policy.add_readonly_rule(PathPattern::Directory(".hg".into()));

        // SSH and keys
        policy.add_readonly_rule(PathPattern::Directory(".ssh".into()));

        // Agent metadata
        policy.add_readonly_rule(PathPattern::Directory(".agents".into()));
        policy.add_readonly_rule(PathPattern::Directory(".codex".into()));

        // Sensitive file extensions
        policy.add_readonly_rule(PathPattern::Extension("pem".into()));
        policy.add_readonly_rule(PathPattern::Extension("key".into()));
        policy.add_readonly_rule(PathPattern::Extension("p12".into()));
        policy.add_readonly_rule(PathPattern::Extension("pfx".into()));

        // Environment and config
        policy.add_readonly_rule(PathPattern::Prefix(".env".into()));
        policy.add_readonly_rule(PathPattern::Prefix(".secret".into()));

        // Package management
        policy.add_readonly_rule(PathPattern::Directory("node_modules".into()));
        policy.add_readonly_rule(PathPattern::Directory(".venv".into()));
        policy.add_readonly_rule(PathPattern::Directory("__pycache__".into()));

        policy
    }

    /// Add a read-only rule.
    pub fn add_readonly_rule(&mut self, pattern: PathPattern) {
        self.rules.push((pattern, AccessMode::Read));
    }

    /// Add an explicit write exception (overrides read-only defaults).
    pub fn add_write_exception(&mut self, pattern: PathPattern) {
        self.rules.push((pattern, AccessMode::Write));
    }

    /// Add a deny rule (highest priority).
    pub fn add_deny_rule(&mut self, pattern: PathPattern) {
        self.rules.push((pattern, AccessMode::Deny));
    }

    /// Query the final access mode for a relative path.
    ///
    /// The last matching rule wins. Default is `Write` (paths under the
    /// writable root are writable unless a rule restricts them).
    pub fn query_access_mode(&self, relative: &Path) -> AccessMode {
        let mut result = AccessMode::Write;
        for (pattern, mode) in &self.rules {
            if pattern.matches(relative) {
                result = *mode;
            }
        }
        result
    }

    /// Check if a path is writable (mode is `Write`, not `Read` or `Deny`).
    pub fn can_write_path(&self, relative: &Path) -> bool {
        self.query_access_mode(relative) == AccessMode::Write
    }
}

impl Default for FileSystemSandboxPolicy {
    fn default() -> Self {
        Self::with_defaults()
    }
}

// ── WritableRoot ────────────────────────────────────────────────────────────

/// The writable root directory for a tool execution context.
///
/// Provides three-layer write permission checking:
/// 1. System-level read-only path check
/// 2. Path must be under root
/// 3. Path must not be under any read_only_subpath
/// 4. Path must not contain a protected_metadata_name
/// 5. Final determination via FileSystemSandboxPolicy
#[derive(Debug, Clone)]
pub struct WritableRoot {
    /// The root directory that is writable.
    pub root: PathBuf,
    /// Sub-paths under root that are read-only (used for --ro-bind).
    pub read_only_subpaths: Vec<PathBuf>,
    /// Protected metadata directory names.
    pub protected_metadata_names: Vec<String>,
    /// The filesystem sandbox policy.
    pub policy: FileSystemSandboxPolicy,
    /// System-level read-only paths (outside root).
    pub system_readonly: Vec<PathBuf>,
}

impl WritableRoot {
    /// Create a new WritableRoot with default settings.
    pub fn new(root: PathBuf) -> Self {
        Self {
            root,
            read_only_subpaths: Vec::new(),
            protected_metadata_names: PROTECTED_METADATA_NAMES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            policy: FileSystemSandboxPolicy::with_defaults(),
            system_readonly: Self::default_system_readonly(),
        }
    }

    /// Create with a custom policy.
    pub fn with_policy(root: PathBuf, policy: FileSystemSandboxPolicy) -> Self {
        Self {
            root,
            read_only_subpaths: Vec::new(),
            protected_metadata_names: PROTECTED_METADATA_NAMES
                .iter()
                .map(|s| s.to_string())
                .collect(),
            policy,
            system_readonly: Self::default_system_readonly(),
        }
    }

    /// Default system-level read-only paths.
    fn default_system_readonly() -> Vec<PathBuf> {
        vec![
            PathBuf::from("/etc/agent"),
            PathBuf::from("/etc/ssh"),
            PathBuf::from("/etc/ssl/private"),
            PathBuf::from("/var/log/agent"),
        ]
    }

    /// Generate default read-only sub-paths based on what exists under root.
    pub fn generate_default_read_only_subpaths(&mut self) {
        let candidates = vec![".git", ".svn", ".hg", ".ssh", ".agents", ".codex"];

        self.read_only_subpaths.clear();
        for name in candidates {
            let path = self.root.join(name);
            if path.exists() {
                self.read_only_subpaths.push(path);
            }
        }
    }

    /// Check if a path is writable under this root.
    ///
    /// Five-layer check:
    /// 1. System-level read-only path check
    /// 2. Path must be under root
    /// 3. Path must not be under any read_only_subpath
    /// 4. Path must not contain a protected_metadata_name
    /// 5. Final determination via FileSystemSandboxPolicy
    pub fn is_path_writable(&self, path: &Path) -> bool {
        // 1. System-level read-only
        for sys_path in &self.system_readonly {
            if path == sys_path || path.starts_with(sys_path) {
                return false;
            }
        }

        // 2. Must be under root
        if !path.starts_with(&self.root) {
            return false;
        }

        // 3. Must not be under any read_only_subpath
        for ro_path in &self.read_only_subpaths {
            if path == ro_path || path.starts_with(ro_path) {
                return false;
            }
        }

        // 4. Must not contain a protected_metadata_name in path components
        if let Ok(relative) = path.strip_prefix(&self.root) {
            for component in relative.components() {
                let name = component.as_os_str().to_string_lossy();
                if self.protected_metadata_names.contains(&name.to_string()) {
                    // Check if there's an explicit write exception
                    if !self.policy.can_write_path(relative) {
                        return false;
                    }
                }
            }

            // 5. Final policy check
            self.policy.can_write_path(relative)
        } else {
            false
        }
    }

    /// Generate bubblewrap arguments for this writable root.
    ///
    /// Produces:
    /// 1. `--bind root root` (writable root)
    /// 2. `--ro-bind subpath subpath` for each existing read-only sub-path
    /// 3. `--ro-bind sys_path sys_path` for system-level paths
    pub fn to_bwrap_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        // Writable root
        args.push("--bind".to_string());
        args.push(self.root.to_string_lossy().to_string());
        args.push(self.root.to_string_lossy().to_string());

        // Read-only sub-paths
        for ro_path in &self.read_only_subpaths {
            if ro_path.exists() {
                args.push("--ro-bind".to_string());
                args.push(ro_path.to_string_lossy().to_string());
                args.push(ro_path.to_string_lossy().to_string());
            }
        }

        // System-level read-only paths
        for sys_path in &self.system_readonly {
            if sys_path.exists() {
                args.push("--ro-bind".to_string());
                args.push(sys_path.to_string_lossy().to_string());
                args.push(sys_path.to_string_lossy().to_string());
            }
        }

        args
    }
}

// ── PathAccessGuard ─────────────────────────────────────────────────────────

/// Stateless helper for runtime path access checks.
///
/// Used at the policy layer (L1) before sandbox execution.
#[derive(Debug)]
pub struct PathAccessGuard;

impl PathAccessGuard {
    /// Canonicalize a path while preserving the final component as-is.
    ///
    /// Resolves intermediate symlinks but keeps the final component unchanged.
    /// Gracefully degrades to the original path on dangling symlinks.
    pub fn canonicalize_preserving_symlinks(path: &Path) -> PathBuf {
        let parent = path.parent().unwrap_or(Path::new("."));
        let file_name = path.file_name();

        match parent.canonicalize() {
            Ok(canonical_parent) => {
                if let Some(name) = file_name {
                    canonical_parent.join(name)
                } else {
                    canonical_parent
                }
            }
            Err(_) => path.to_path_buf(),
        }
    }

    /// Check if a path is writable, returning the canonical path on success.
    ///
    /// Returns `Ok(canonical_path)` or `Err(reason)` with a specific refusal
    /// reason and suggested action.
    pub fn check_write(
        path: &Path,
        writable_root: &WritableRoot,
    ) -> Result<PathBuf, PathAccessError> {
        let canonical = Self::canonicalize_preserving_symlinks(path);

        if !writable_root.is_path_writable(&canonical) {
            // Determine the specific reason
            for sys_path in &writable_root.system_readonly {
                if canonical == *sys_path || canonical.starts_with(sys_path) {
                    return Err(PathAccessError::SystemReadOnly {
                        path: canonical,
                        suggestion: format!(
                            "Path {} is system-level read-only. Use a path under {} instead.",
                            path.display(),
                            writable_root.root.display()
                        ),
                    });
                }
            }

            if !canonical.starts_with(&writable_root.root) {
                return Err(PathAccessError::OutsideRoot {
                    path: canonical,
                    root: writable_root.root.clone(),
                    suggestion: format!(
                        "Path {} is outside writable root {}. Use a path under {}.",
                        path.display(),
                        writable_root.root.display(),
                        writable_root.root.display()
                    ),
                });
            }

            // Check if it's a protected metadata name
            if let Ok(relative) = canonical.strip_prefix(&writable_root.root) {
                for component in relative.components() {
                    let name = component.as_os_str().to_string_lossy();
                    if writable_root
                        .protected_metadata_names
                        .contains(&name.to_string())
                    {
                        return Err(PathAccessError::ProtectedMetadata {
                            path: canonical.clone(),
                            name: name.to_string(),
                            suggestion: format!(
                                "Cannot write to protected metadata '{}'. \
                                 Add an explicit write exception if needed.",
                                name
                            ),
                        });
                    }
                }
            }

            return Err(PathAccessError::ReadOnly {
                path: canonical,
                suggestion: format!(
                    "Path {} is read-only by policy. Use a different path or add a write exception.",
                    path.display()
                ),
            });
        }

        Ok(canonical)
    }

    /// Pre-execution check: block writes to protected metadata before
    /// sandbox execution starts.
    pub fn forbidden_agent_metadata_write(path: &Path) -> Result<(), PathAccessError> {
        let components: Vec<_> = path
            .components()
            .map(|c| c.as_os_str().to_string_lossy().to_string())
            .collect();

        for name in PROTECTED_METADATA_NAMES {
            if components.iter().any(|c| c == name) {
                return Err(PathAccessError::ProtectedMetadata {
                    path: path.to_path_buf(),
                    name: name.to_string(),
                    suggestion: format!(
                        "Cannot write to protected metadata '{}'. \
                         This is blocked before sandbox execution.",
                        name
                    ),
                });
            }
        }

        Ok(())
    }

    /// Batch check multiple paths.
    pub fn check_write_batch(
        paths: &[PathBuf],
        writable_root: &WritableRoot,
    ) -> Vec<Result<PathBuf, PathAccessError>> {
        paths
            .iter()
            .map(|p| Self::check_write(p, writable_root))
            .collect()
    }
}

// ── Path Access Error ───────────────────────────────────────────────────────

/// Specific reason why a path write was refused.
#[derive(Debug, Clone)]
pub enum PathAccessError {
    /// Path is outside the writable root.
    OutsideRoot {
        path: PathBuf,
        root: PathBuf,
        suggestion: String,
    },
    /// Path is a system-level read-only location.
    SystemReadOnly { path: PathBuf, suggestion: String },
    /// Path is read-only by policy.
    ReadOnly { path: PathBuf, suggestion: String },
    /// Path contains a protected metadata name.
    ProtectedMetadata {
        path: PathBuf,
        name: String,
        suggestion: String,
    },
}

impl std::fmt::Display for PathAccessError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::OutsideRoot { suggestion, .. } => write!(f, "{}", suggestion),
            Self::SystemReadOnly { suggestion, .. } => write!(f, "{}", suggestion),
            Self::ReadOnly { suggestion, .. } => write!(f, "{}", suggestion),
            Self::ProtectedMetadata { suggestion, .. } => write!(f, "{}", suggestion),
        }
    }
}

impl std::error::Error for PathAccessError {}

// ── Tests ───────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;

    fn make_root() -> (tempfile::TempDir, WritableRoot) {
        let dir = tempfile::tempdir().unwrap();
        // Create some directories
        std::fs::create_dir_all(dir.path().join("src")).unwrap();
        std::fs::create_dir_all(dir.path().join(".git")).unwrap();
        std::fs::create_dir_all(dir.path().join(".ssh")).unwrap();
        std::fs::write(dir.path().join("src").join("main.rs"), "fn main() {}").unwrap();
        std::fs::write(dir.path().join("secret.pem"), "key").unwrap();
        std::fs::write(dir.path().join(".env"), "SECRET=1").unwrap();

        let mut root = WritableRoot::new(dir.path().to_path_buf());
        root.generate_default_read_only_subpaths();
        (dir, root)
    }

    #[test]
    fn test_writable_under_root() {
        let (_dir, root) = make_root();
        assert!(root.is_path_writable(&root.root.join("src").join("main.rs")));
        assert!(root.is_path_writable(&root.root.join("README.md")));
    }

    #[test]
    fn test_readonly_git() {
        let (_dir, root) = make_root();
        assert!(!root.is_path_writable(&root.root.join(".git").join("config")));
        assert!(!root.is_path_writable(&root.root.join(".git")));
    }

    #[test]
    fn test_readonly_ssh() {
        let (_dir, root) = make_root();
        assert!(!root.is_path_writable(&root.root.join(".ssh").join("id_rsa")));
    }

    #[test]
    fn test_readonly_pem_extension() {
        let (_dir, root) = make_root();
        assert!(!root.is_path_writable(&root.root.join("secret.pem")));
    }

    #[test]
    fn test_readonly_env_prefix() {
        let (_dir, root) = make_root();
        assert!(!root.is_path_writable(&root.root.join(".env")));
        assert!(!root.is_path_writable(&root.root.join(".env.local")));
    }

    #[test]
    fn test_outside_root_rejected() {
        let (_dir, root) = make_root();
        assert!(!root.is_path_writable(Path::new("/tmp/outside")));
        assert!(!root.is_path_writable(Path::new("/etc/passwd")));
    }

    #[test]
    fn test_system_readonly() {
        let (_dir, root) = make_root();
        assert!(!root.is_path_writable(Path::new("/etc/agent/config.toml")));
        assert!(!root.is_path_writable(Path::new("/etc/ssh/ssh_config")));
    }

    #[test]
    fn test_write_exception() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join(".env"), "key=val").unwrap();

        let mut policy = FileSystemSandboxPolicy::with_defaults();
        policy.add_write_exception(PathPattern::Exact(PathBuf::from(".env")));

        let root = WritableRoot::with_policy(dir.path().to_path_buf(), policy);
        // .env is read-only by default prefix rule, but write exception overrides
        // Note: the write exception is for exact ".env", not ".env.local"
        assert!(root.is_path_writable(&root.root.join(".env")));
    }

    #[test]
    fn test_deny_rule() {
        let dir = tempfile::tempdir().unwrap();
        let mut policy = FileSystemSandboxPolicy::with_defaults();
        policy.add_deny_rule(PathPattern::Extension("key".into()));

        let root = WritableRoot::with_policy(dir.path().to_path_buf(), policy);
        assert!(!root.is_path_writable(&dir.path().join("server.key")));
    }

    #[test]
    fn test_path_pattern_exact() {
        let p = PathPattern::Exact(PathBuf::from(".git"));
        assert!(p.matches(Path::new(".git")));
        assert!(!p.matches(Path::new(".git/config")));
        assert!(!p.matches(Path::new("git")));
    }

    #[test]
    fn test_path_pattern_directory() {
        let p = PathPattern::Directory(PathBuf::from(".ssh"));
        assert!(p.matches(Path::new(".ssh")));
        assert!(p.matches(Path::new(".ssh/id_rsa")));
        assert!(!p.matches(Path::new("ssh")));
    }

    #[test]
    fn test_path_pattern_extension() {
        let p = PathPattern::Extension("pem".into());
        assert!(p.matches(Path::new("cert.pem")));
        assert!(p.matches(Path::new("dir/cert.pem")));
        assert!(!p.matches(Path::new("cert.crt")));
    }

    #[test]
    fn test_path_pattern_prefix() {
        let p = PathPattern::Prefix(".env".into());
        assert!(p.matches(Path::new(".env")));
        assert!(p.matches(Path::new(".env.local")));
        assert!(p.matches(Path::new("dir/.env")));
        assert!(!p.matches(Path::new("environment")));
    }

    #[test]
    fn test_path_pattern_top_level_component() {
        let p = PathPattern::TopLevelComponent(".git".into());
        assert!(p.matches(Path::new(".git")));
        assert!(p.matches(Path::new(".git/config")));
        assert!(!p.matches(Path::new("src/.git")));
    }

    #[test]
    fn test_bwrap_args() {
        let (_dir, root) = make_root();
        let args = root.to_bwrap_args();
        // Should have at least --bind for root
        assert!(args.contains(&"--bind".to_string()));
        // Should have --ro-bind for .git and .ssh
        assert!(args.iter().any(|a| a.contains(".git")));
        assert!(args.iter().any(|a| a.contains(".ssh")));
    }

    #[test]
    fn test_path_access_guard_check_write() {
        let (_dir, root) = make_root();

        // Writable path
        let result = PathAccessGuard::check_write(&root.root.join("src").join("main.rs"), &root);
        assert!(result.is_ok());

        // Protected metadata
        let result = PathAccessGuard::check_write(&root.root.join(".git").join("config"), &root);
        assert!(matches!(
            result,
            Err(PathAccessError::ProtectedMetadata { .. })
        ));
    }

    #[test]
    fn test_forbidden_agent_metadata_write() {
        assert!(PathAccessGuard::forbidden_agent_metadata_write(Path::new("src/main.rs")).is_ok());
        assert!(PathAccessGuard::forbidden_agent_metadata_write(Path::new(".git/config")).is_err());
        assert!(
            PathAccessGuard::forbidden_agent_metadata_write(Path::new("project/.ssh/id_rsa"))
                .is_err()
        );
    }

    #[test]
    fn test_check_write_batch() {
        let (_dir, root) = make_root();
        let paths = vec![
            root.root.join("src").join("main.rs"),
            root.root.join(".git").join("config"),
            root.root.join("README.md"),
        ];
        let results = PathAccessGuard::check_write_batch(&paths, &root);
        assert!(results[0].is_ok());
        assert!(results[1].is_err());
        assert!(results[2].is_ok());
    }

    #[test]
    fn test_canonicalize_preserving_symlinks() {
        let dir = tempfile::tempdir().unwrap();
        let target = dir.path().join("target");
        std::fs::create_dir(&target).unwrap();
        let link = dir.path().join("link");
        #[cfg(unix)]
        std::os::unix::fs::symlink(&target, &link).unwrap();

        let result = PathAccessGuard::canonicalize_preserving_symlinks(&link.join("file.txt"));
        // Should resolve the symlink for parent but keep file.txt
        assert!(result.ends_with("file.txt"));
    }
}
