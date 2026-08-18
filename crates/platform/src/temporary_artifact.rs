//! Host-managed temporary artifact with best-effort cleanup on drop.

use std::io::Write;
use std::path::{Path, PathBuf};

pub struct TemporaryArtifact {
    path: PathBuf,
}

impl TemporaryArtifact {
    pub fn write(prefix: &str, bytes: &[u8]) -> anyhow::Result<Self> {
        let path = std::env::temp_dir().join(format!("{prefix}-{}.json", uuid::Uuid::new_v4()));
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)?;
        file.write_all(bytes)?;
        file.sync_all()?;
        Ok(Self { path })
    }

    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl application::approval::TemporaryArtifactHandle for TemporaryArtifact {
    fn path(&self) -> &Path {
        self.path()
    }
}

#[derive(Debug, Clone, Default)]
pub struct HostTemporaryArtifactStore;

impl application::approval::TemporaryArtifactStore for HostTemporaryArtifactStore {
    fn write(
        &self,
        prefix: &str,
        bytes: &[u8],
    ) -> Result<Box<dyn application::approval::TemporaryArtifactHandle>, String> {
        TemporaryArtifact::write(prefix, bytes)
            .map(|artifact| Box::new(artifact) as Box<_>)
            .map_err(|error| error.to_string())
    }
}

impl Drop for TemporaryArtifact {
    fn drop(&mut self) {
        if let Err(error) = std::fs::remove_file(&self.path) {
            if error.kind() != std::io::ErrorKind::NotFound {
                tracing::warn!(error = %error, path = %self.path.display(), "temporary artifact cleanup failed");
            }
        }
    }
}
