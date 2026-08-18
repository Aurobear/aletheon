//! Host path adapter for managed Goal coding worktrees.

use application::goal::CodingWorktreePort;
use std::path::{Component, Path, PathBuf};

pub struct HostCodingWorktreeResolver {
    base: PathBuf,
}

impl HostCodingWorktreeResolver {
    pub fn new(base: impl AsRef<Path>) -> Result<Self, String> {
        let base = base
            .as_ref()
            .canonicalize()
            .map_err(|error| format!("resolving managed worktree base: {error}"))?;
        if !base.is_dir() {
            return Err("coding worktree base is not a directory".into());
        }
        Ok(Self { base })
    }
}

impl CodingWorktreePort for HostCodingWorktreeResolver {
    fn resolve(&self, relative: &Path) -> Result<PathBuf, String> {
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            return Err("coding worktree reference must be a non-empty relative path".into());
        }
        let path = self
            .base
            .join(relative)
            .canonicalize()
            .map_err(|error| format!("resolving managed worktree: {error}"))?;
        if !path.starts_with(&self.base) || !path.is_dir() {
            return Err("managed worktree escapes configured base".into());
        }
        Ok(path)
    }
}
