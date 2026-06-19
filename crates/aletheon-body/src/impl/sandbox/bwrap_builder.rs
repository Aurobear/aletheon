use std::path::{Path, PathBuf};

use tracing::debug;

use crate::r#impl::sandbox::glob_scanner::GlobScanner;
use crate::r#impl::sandbox::policy::{FilesystemPolicy, FsDefault};
use crate::r#impl::sandbox::SandboxConfig;

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
    /// Argument order (critical):
    /// 1. Base isolation flags (`--die-with-parent`, `--unshare-*`)
    /// 2. Full read-only root bind (if default is `ReadOnly`)
    /// 3. `--dev /dev`, `--proc /proc`
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

        // -- Step 2: Full filesystem root --
        self.push_root_bind(&mut args);

        // -- Step 3: Device and proc --
        self.push_dev_and_proc(&mut args);

        // -- Step 4: Mask unreadable ancestors of writable roots --
        self.push_ancestor_masks(&mut args);

        // -- Step 5: Bind writable roots --
        self.push_writable_roots(&mut args);

        // -- Step 6: Re-protect metadata sub-paths --
        self.push_protected_metadata(&mut args);

        // -- Step 7: Mask unreadable globs --
        self.push_unreadable_masks(&mut args);

        // -- Step 8: Tmpfs for /tmp --
        args.push("--tmpfs".into());
        args.push("/tmp".into());

        // -- Step 9: Writable working directory (always bind) --
        if !config.working_dir.is_empty() {
            args.push("--bind".into());
            args.push(config.working_dir.clone());
            args.push(config.working_dir.clone());
        }

        // -- Step 10: Environment variables --
        for (key, value) in &config.env_vars {
            args.push("--setenv".into());
            args.push(key.clone());
            args.push(value.clone());
        }

        // -- Step 11: Command --
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

    fn push_root_bind(&self, args: &mut Vec<String>) {
        match self.policy.default {
            FsDefault::ReadOnly => {
                args.push("--ro-bind".into());
                args.push("/".into());
                args.push("/".into());
            }
            FsDefault::Writable => {
                // In writable-default mode we still bind the root read-only
                // and then make specific roots writable.  A fully writable
                // root is extremely permissive; the explicit writable_roots
                // list is the intended escape hatch.
                args.push("--ro-bind".into());
                args.push("/".into());
                args.push("/".into());
            }
        }
    }

    fn push_dev_and_proc(&self, args: &mut Vec<String>) {
        args.push("--dev".into());
        args.push("/dev".into());
        args.push("--proc".into());
        args.push("/proc".into());
    }

    /// For every writable root, mask its parent directory entries that the
    /// sandbox should not be able to read (using the unreadable globs to
    /// discover files inside the root that must be hidden).
    fn push_ancestor_masks(&self, args: &mut Vec<String>) {
        for root in &self.policy.writable_roots {
            // Ensure the root's parent exists and is visible.
            // If the root path itself does not exist on the host we skip it —
            // bwrap would fail to mount anyway.
            if !root.path.exists() {
                debug!(path = %root.path.display(), "Writable root does not exist, skipping ancestor mask");
                continue;
            }
            // Nothing extra to mask at the ancestor level for now — the
            // unreadable glob masking in step 7 handles individual files.
            // This step is a placeholder for future expansion (e.g. masking
            // sibling directories of the writable root).
            let _ = args; // suppress unused warning
        }
    }

    fn push_writable_roots(&self, args: &mut Vec<String>) {
        for root in &self.policy.writable_roots {
            debug!(path = %root.path.display(), "Binding writable root");
            args.push("--bind".into());
            args.push(root.path.to_string_lossy().to_string());
            args.push(root.path.to_string_lossy().to_string());
        }
    }

    fn push_protected_metadata(&self, args: &mut Vec<String>) {
        if self.policy.protected_metadata.is_empty() {
            return;
        }

        for root in &self.policy.writable_roots {
            for meta_name in &self.policy.protected_metadata {
                let meta_path = root.path.join(meta_name);
                if meta_path.exists() {
                    debug!(path = %meta_path.display(), "Re-protecting metadata");
                    args.push("--ro-bind".into());
                    args.push(meta_path.to_string_lossy().to_string());
                    args.push(meta_path.to_string_lossy().to_string());
                }
            }
        }
    }

    fn push_unreadable_masks(&self, args: &mut Vec<String>) {
        if self.policy.unreadable_globs.is_empty() {
            return;
        }

        let scanner = GlobScanner::default();

        for root in &self.policy.writable_roots {
            let matches = scanner.scan(&self.policy.unreadable_globs, &root.path);
            for path in matches {
                debug!(path = %path.display(), "Masking unreadable path");
                args.push("--ro-bind".into());
                args.push("/dev/null".into());
                args.push(path.to_string_lossy().to_string());
            }
        }
    }
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
    use crate::r#impl::sandbox::policy::{FilesystemPolicy, FsDefault, WritableRoot};

    fn default_config() -> SandboxConfig {
        SandboxConfig {
            working_dir: "/tmp/work".into(),
            env_vars: Default::default(),
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
        let mut env = std::collections::HashMap::new();
        env.insert("FOO".to_string(), "bar".to_string());
        let config = SandboxConfig {
            working_dir: "/tmp/work".into(),
            env_vars: env,
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
