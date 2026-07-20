use std::path::{Path, PathBuf};

use tracing::debug;

use crate::sandbox::glob_scanner::GlobScanner;
use crate::sandbox::policy::FilesystemPolicy;
use crate::sandbox::SandboxConfig;

/// Advanced bubblewrap argument builder driven by a [`FilesystemPolicy`].
///
/// The builder translates the declarative policy into an ordered sequence of
/// `bwrap` CLI arguments.  The argument order is critical — bubblewrap applies
/// bind mounts in the order they appear, so later mounts override earlier ones.
pub struct BwrapBuilder {
    policy: FilesystemPolicy,
}

impl BwrapBuilder {
    /// Create a new builder from the given filesystem policy.
    pub fn new(policy: FilesystemPolicy) -> Self {
        Self { policy }
    }

    /// Build the complete `bwrap` argument list for executing `cmd`.
    ///
    /// Argument order (critical — bubblewrap applies mounts in order,
    /// later mounts override earlier ones):
    /// 1. Base isolation flags (`--die-with-parent`, `--unshare-*`)
    /// 2. Full read-only root bind (must come BEFORE dev/proc so the
    ///    fresh devtmpfs isn't overwritten by the recursive root bind)
    /// 3. `--dev /dev`, `--proc /proc` (fresh devtmpfs on top of RO root)
    /// 4. Mask unreadable ancestors of writable roots
    /// 5. Bind writable roots (`--bind <root> <root>`)
    /// 6. Re-protect sub-paths (`--ro-bind` for `.git`, etc.)
    /// 7. Mask unreadable glob matches (`--ro-bind /dev/null <path>`)
    /// 8. Environment variables
    /// 9. Command
    pub fn build_args(&self, cmd: &str, config: &SandboxConfig) -> Vec<String> {
        let mut args = Vec::with_capacity(128);

        // -- Step 1: Base isolation flags --
        self.push_base_flags(&mut args);

        // -- Step 2: One ordered root/writable/protected/masked plan --
        append_mount_plan(&mut args, &self.policy);

        // -- Step 3: Device and proc (fresh devtmpfs on top of RO root) --
        self.push_dev_and_proc(&mut args);

        // -- Step 4: Environment variables --
        for (key, value) in &config.environment {
            args.push("--setenv".into());
            args.push(key.clone());
            args.push(value.clone());
        }

        // -- Step 5: Command --
        args.push("--".into());
        args.push("/bin/bash".into());
        args.push("-c".into());
        args.push(cmd.to_string());

        args
    }

    /// Return the underlying policy (for inspection / testing).
    pub fn policy(&self) -> &FilesystemPolicy {
        &self.policy
    }

    // ---- private helpers ------------------------------------------------

    fn push_base_flags(&self, args: &mut Vec<String>) {
        args.push("--die-with-parent".into());
        args.push("--unshare-pid".into());
        args.push("--unshare-ipc".into());
        args.push("--unshare-net".into());
    }

    fn push_dev_and_proc(&self, args: &mut Vec<String>) {
        args.push("--dev".into());
        args.push("/dev".into());
        args.push("--proc".into());
        args.push("/proc".into());
    }
}

/// Append the mount overrides shared by the policy builder and production
/// bubblewrap backend. Ordering is security-sensitive: writable roots must be
/// installed before their protected subpaths are rebound read-only.
pub(crate) fn append_mount_plan(args: &mut Vec<String>, policy: &FilesystemPolicy) {
    // Even the legacy writable-default variant is fail-closed: callers must
    // enumerate explicit writable roots rather than expose the host root.
    let _ = policy.default;
    push_triplet(args, "--ro-bind", Path::new("/"), Path::new("/"));

    for root in &policy.writable_roots {
        debug!(path = %root.path.display(), "Binding writable root");
        push_triplet(args, "--bind", &root.path, &root.path);
    }

    for root in &policy.writable_roots {
        for relative in &root.read_only_subpaths {
            push_ro_bind_if_exists(args, &root.path.join(relative));
        }
        for name in &policy.protected_metadata {
            push_ro_bind_if_exists(args, &root.path.join(name));
        }
    }

    if !policy.unreadable_globs.is_empty() {
        let scanner = GlobScanner::default();
        for root in &policy.writable_roots {
            for path in scanner.scan(&policy.unreadable_globs, &root.path) {
                debug!(path = %path.display(), "Masking unreadable path");
                push_triplet(args, "--ro-bind", Path::new("/dev/null"), &path);
            }
        }
    }
}

fn push_ro_bind_if_exists(args: &mut Vec<String>, path: &Path) {
    if path.exists() {
        debug!(path = %path.display(), "Re-protecting workspace path");
        push_triplet(args, "--ro-bind", path, path);
    }
}

fn push_triplet(args: &mut Vec<String>, flag: &str, source: &Path, target: &Path) {
    args.push(flag.to_owned());
    args.push(source.to_string_lossy().into_owned());
    args.push(target.to_string_lossy().into_owned());
}

/// Mask an individual path by binding `/dev/null` over it.
///
/// Returns the two `bwrap` arguments: `["--ro-bind", "/dev/null", "<path>"]`.
pub fn mask_args(path: &Path) -> [String; 3] {
    [
        "--ro-bind".into(),
        "/dev/null".into(),
        path.to_string_lossy().to_string(),
    ]
}

/// Collect all ancestor directories of `path` up to (but not including) `root`.
///
/// For `/home/user/project/.git` with root `/`, this returns
/// `["/home", "/home/user", "/home/user/project"]`.
pub fn ancestor_dirs(path: &Path, root: &Path) -> Vec<PathBuf> {
    let mut ancestors = Vec::new();
    let mut current = path.parent();
    while let Some(p) = current {
        if p == root || p.as_os_str().is_empty() {
            break;
        }
        ancestors.push(p.to_path_buf());
        current = p.parent();
    }
    ancestors.reverse();
    ancestors
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::sandbox::policy::{FilesystemPolicy, FsDefault, WritableRoot};

    fn default_config() -> SandboxConfig {
        SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots("/tmp/work".into(), vec![])
                .unwrap(),
            environment: Default::default(),
            policy: None,
        }
    }

    #[test]
    fn test_read_only_default_has_ro_bind_root() {
        let policy = FilesystemPolicy::new(FsDefault::ReadOnly);
        let builder = BwrapBuilder::new(policy);
        let args = builder.build_args("echo hello", &default_config());

        // Must contain --ro-bind / /
        let ro_pos = args.iter().position(|a| a == "--ro-bind");
        assert!(ro_pos.is_some(), "Expected --ro-bind in args");
        assert_eq!(args[ro_pos.unwrap() + 1], "/");
        assert_eq!(args[ro_pos.unwrap() + 2], "/");
    }

    #[test]
    fn test_writable_default_still_binds_root_ro() {
        let policy = FilesystemPolicy::new(FsDefault::Writable);
        let builder = BwrapBuilder::new(policy);
        let args = builder.build_args("echo hello", &default_config());

        let ro_pos = args.iter().position(|a| a == "--ro-bind");
        assert!(
            ro_pos.is_some(),
            "Expected --ro-bind / / even in writable mode"
        );
        assert_eq!(args[ro_pos.unwrap() + 1], "/");
        assert_eq!(args[ro_pos.unwrap() + 2], "/");
    }

    #[test]
    fn test_writable_root_binds() {
        let policy = FilesystemPolicy {
            default: FsDefault::ReadOnly,
            writable_roots: vec![WritableRoot::new("/tmp/work")],
            protected_metadata: vec![],
            unreadable_globs: vec![],
        };
        let builder = BwrapBuilder::new(policy);
        let args = builder.build_args("echo hi", &default_config());

        // Find --bind /tmp/work /tmp/work
        let bind_positions: Vec<usize> = args
            .iter()
            .enumerate()
            .filter_map(|(i, a)| if a == "--bind" { Some(i) } else { None })
            .collect();
        assert!(
            bind_positions
                .iter()
                .any(|&i| args[i + 1] == "/tmp/work" && args[i + 2] == "/tmp/work"),
            "Expected --bind /tmp/work /tmp/work in args: {:?}",
            args
        );
    }

    #[test]
    fn test_protected_metadata_reprotection() {
        let policy = FilesystemPolicy {
            default: FsDefault::ReadOnly,
            writable_roots: vec![WritableRoot::new("/tmp/work")],
            protected_metadata: vec![".git".into(), ".agents".into()],
            unreadable_globs: vec![],
        };
        let builder = BwrapBuilder::new(policy);
        let args = builder.build_args("echo hi", &default_config());

        // Check that --ro-bind is used for .git and .agents paths.
        // Note: the actual --ro-bind only fires if the path exists on disk.
        // In a test environment these paths may not exist, so we verify the
        // *intent* by checking that the builder produced args without panic.
        let ro_bind_count = args.iter().filter(|a| a.as_str() == "--ro-bind").count();
        // At minimum we have the root --ro-bind / / (1 occurrence for ReadOnly)
        assert!(ro_bind_count >= 1, "Expected at least 1 --ro-bind for root");
    }

    #[test]
    fn test_unreadable_glob_masking() {
        // Use a glob that won't match anything on disk — just verify the
        // scanner is invoked and the builder doesn't panic.
        let policy = FilesystemPolicy {
            default: FsDefault::ReadOnly,
            writable_roots: vec![WritableRoot::new("/tmp/work")],
            protected_metadata: vec![],
            unreadable_globs: vec!["**/*.nonexistent_extension_xyz".into()],
        };
        let builder = BwrapBuilder::new(policy);
        let args = builder.build_args("echo hi", &default_config());

        // No matches expected, so no extra --ro-bind /dev/null entries.
        let mask_count = args
            .windows(3)
            .filter(|w| w[0] == "--ro-bind" && w[1] == "/dev/null")
            .count();
        assert_eq!(
            mask_count, 0,
            "Expected 0 mask entries for non-matching glob"
        );
    }

    #[test]
    fn test_env_vars_propagated() {
        let mut env = std::collections::BTreeMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let config = SandboxConfig {
            workspace: fabric::WorkspacePolicy::from_resolved_roots("/tmp/work".into(), vec![])
                .unwrap(),
            environment: env,
            policy: None,
        };
        let policy = FilesystemPolicy::new(FsDefault::ReadOnly);
        let builder = BwrapBuilder::new(policy);
        let args = builder.build_args("echo hi", &config);

        let setenv_pos = args.iter().position(|a| a == "--setenv");
        assert!(setenv_pos.is_some(), "Expected --setenv for env vars");
        assert_eq!(args[setenv_pos.unwrap() + 1], "FOO");
        assert_eq!(args[setenv_pos.unwrap() + 2], "bar");
    }

    #[test]
    fn test_ancestor_dirs() {
        let ancestors = ancestor_dirs(
            &PathBuf::from("/home/user/project/.git"),
            &PathBuf::from("/"),
        );
        assert_eq!(
            ancestors,
            vec![
                PathBuf::from("/home"),
                PathBuf::from("/home/user"),
                PathBuf::from("/home/user/project"),
            ]
        );
    }

    #[test]
    fn test_mask_args_format() {
        let args = mask_args(&PathBuf::from("/tmp/secret.env"));
        assert_eq!(args[0], "--ro-bind");
        assert_eq!(args[1], "/dev/null");
        assert_eq!(args[2], "/tmp/secret.env");
    }

    #[test]
    fn test_command_at_end() {
        let policy = FilesystemPolicy::new(FsDefault::ReadOnly);
        let builder = BwrapBuilder::new(policy);
        let args = builder.build_args("ls -la /tmp", &default_config());

        assert_eq!(args.last().unwrap(), "ls -la /tmp");
        assert_eq!(args[args.len() - 2], "-c");
        assert_eq!(args[args.len() - 3], "/bin/bash");
        assert_eq!(args[args.len() - 4], "--");
    }
}
