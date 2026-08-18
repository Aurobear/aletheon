//! Fail-closed filesystem adapter for Goal artifacts.

use std::fs::{File, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};

use anyhow::{bail, Context, Result};
use application::goal_artifact::GoalArtifactStore;

pub struct FilesystemGoalArtifactStore {
    root: PathBuf,
}

impl FilesystemGoalArtifactStore {
    pub fn open(root: impl AsRef<Path>) -> Result<Self> {
        std::fs::create_dir_all(root.as_ref()).context("creating Goal artifact directory")?;
        let root = root
            .as_ref()
            .canonicalize()
            .context("canonicalizing Goal artifact directory")?;
        Ok(Self { root })
    }

    fn validate(relative: &Path) -> Result<()> {
        if relative.as_os_str().is_empty()
            || relative.is_absolute()
            || relative.components().any(|component| {
                matches!(
                    component,
                    Component::ParentDir | Component::RootDir | Component::Prefix(_)
                )
            })
        {
            bail!("artifact reference must be a safe relative path");
        }
        Ok(())
    }
}

impl GoalArtifactStore for FilesystemGoalArtifactStore {
    fn root(&self) -> &Path {
        &self.root
    }

    fn resolve(&self, relative: &Path) -> Result<PathBuf> {
        Self::validate(relative)?;
        let path = self.root.join(relative);
        let parent = path.parent().context("artifact path has no parent")?;
        std::fs::create_dir_all(parent)?;
        let canonical_parent = parent.canonicalize()?;
        if !canonical_parent.starts_with(&self.root) {
            bail!("artifact path escapes Goal artifact directory");
        }
        Ok(path)
    }

    fn write_atomic(&self, relative: &Path, bytes: &[u8]) -> Result<()> {
        let path = self.resolve(relative)?;
        if path.exists() {
            bail!("coding diff artifact already exists");
        }
        let temp = path.with_extension(format!("tmp-{}", uuid::Uuid::new_v4()));
        let result = (|| -> Result<()> {
            let mut file = OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&temp)?;
            file.write_all(bytes)?;
            file.sync_all()?;
            std::fs::rename(&temp, &path)?;
            if let Some(parent) = path.parent() {
                File::open(parent)?.sync_all()?;
            }
            Ok(())
        })();
        if result.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
        result
    }

    fn read_bounded(&self, relative: &Path, max_bytes: usize) -> Result<Vec<u8>> {
        let path = self.resolve(relative)?;
        let metadata = std::fs::metadata(&path)?;
        if metadata.len() > max_bytes as u64 {
            bail!("persisted coding diff exceeds bounded artifact limit");
        }
        let bytes = std::fs::read(path)?;
        if bytes.len() > max_bytes {
            bail!("persisted coding diff exceeds bounded artifact limit");
        }
        Ok(bytes)
    }

    fn remove(&self, relative: &Path) -> Result<()> {
        let path = self.resolve(relative)?;
        match std::fs::remove_file(path) {
            Ok(()) => Ok(()),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(error) => Err(error.into()),
        }
    }
}
