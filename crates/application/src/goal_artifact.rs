//! Filesystem port for bounded Goal artifacts.

use std::path::{Path, PathBuf};

pub trait GoalArtifactStore: Send + Sync {
    fn root(&self) -> &Path;
    fn write_atomic(&self, relative: &Path, bytes: &[u8]) -> anyhow::Result<()>;
    fn read_bounded(&self, relative: &Path, max_bytes: usize) -> anyhow::Result<Vec<u8>>;
    fn remove(&self, relative: &Path) -> anyhow::Result<()>;
    fn resolve(&self, relative: &Path) -> anyhow::Result<PathBuf>;
}
