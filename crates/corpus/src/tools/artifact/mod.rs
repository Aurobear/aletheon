//! Artifact Store — content-addressed storage for large tool outputs (Wave 2A).
//! When a tool result exceeds the context budget, the full output is saved
//! here and the model receives a structured ArtifactRef + summary instead.

use anyhow::{bail, Context, Result};
use sha2::{Digest, Sha256};
use std::path::{Path, PathBuf};

#[derive(Clone, Debug)]
pub struct ArtifactRef {
    pub id: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub mime: String,
}

impl ArtifactRef {
    pub fn uri(&self) -> String {
        format!("artifact://sha256/{}", self.sha256)
    }
}

#[derive(Clone, Debug)]
pub struct ArtifactStore {
    dir: PathBuf,
}

impl ArtifactStore {
    pub fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    pub fn store(&self, content: &[u8], mime: &str) -> Result<ArtifactRef> {
        let hash = format!("{:x}", Sha256::digest(content));
        let path = self.path_for(&hash)?;
        std::fs::create_dir_all(&self.dir)
            .with_context(|| format!("create artifact directory {}", self.dir.display()))?;
        if !path.exists() {
            let temporary = self
                .dir
                .join(format!(".{}.{}.tmp", hash, uuid::Uuid::new_v4()));
            let mut options = std::fs::OpenOptions::new();
            options.write(true).create_new(true);
            #[cfg(unix)]
            {
                use std::os::unix::fs::OpenOptionsExt;
                options.mode(0o600);
            }
            use std::io::Write;
            let mut file = options
                .open(&temporary)
                .with_context(|| format!("create temporary artifact {}", temporary.display()))?;
            file.write_all(content)?;
            file.sync_all()?;
            match std::fs::rename(&temporary, &path) {
                Ok(()) => {}
                Err(error) if path.exists() => {
                    let _ = std::fs::remove_file(&temporary);
                    tracing::debug!(%error, artifact = %hash, "artifact won concurrent publish race");
                }
                Err(error) => {
                    let _ = std::fs::remove_file(&temporary);
                    return Err(error).context("publish artifact atomically");
                }
            }
        }
        let r = ArtifactRef {
            id: hash.clone(),
            sha256: hash,
            size_bytes: content.len() as u64,
            mime: mime.into(),
        };
        Ok(r)
    }

    pub fn read(&self, id: &str) -> Result<Vec<u8>> {
        let path = self.path_for(id)?;
        let content =
            std::fs::read(&path).with_context(|| format!("read artifact {}", path.display()))?;
        let digest = format!("{:x}", Sha256::digest(&content));
        if digest != id {
            bail!("artifact digest mismatch for {id}");
        }
        Ok(content)
    }

    pub fn read_page(&self, id: &str, offset: u64, limit: u64) -> Result<Vec<u8>> {
        let data = self.read(id)?;
        let start = usize::try_from(offset)
            .unwrap_or(usize::MAX)
            .min(data.len());
        let requested_end = offset.saturating_add(limit);
        let end = usize::try_from(requested_end)
            .unwrap_or(usize::MAX)
            .min(data.len());
        Ok(data[start..end].to_vec())
    }

    pub fn root(&self) -> &Path {
        &self.dir
    }

    fn path_for(&self, id: &str) -> Result<PathBuf> {
        if id.len() != 64 || !id.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            bail!("invalid sha256 artifact id");
        }
        Ok(self.dir.join(id.to_ascii_lowercase()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_store_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let store = ArtifactStore::new(tmp.path().to_path_buf());
        let content = b"x".repeat(100_000);
        let r = store.store(&content, "text/plain").unwrap();
        assert_eq!(r.size_bytes, 100_000);
        assert_eq!(r.id.len(), 64);

        let back = store.read(&r.id).unwrap();
        assert_eq!(back.len(), 100_000);

        let page = store.read_page(&r.id, 0, 100).unwrap();
        assert_eq!(page.len(), 100);
    }

    #[test]
    fn artifact_is_reconstructable_after_store_reopen() {
        let tmp = tempfile::tempdir().unwrap();
        let id = ArtifactStore::new(tmp.path().to_path_buf())
            .store(b"durable", "text/plain")
            .unwrap()
            .id;

        let reopened = ArtifactStore::new(tmp.path().to_path_buf());
        assert_eq!(reopened.read(&id).unwrap(), b"durable");
        assert!(reopened.read("../escape").is_err());
    }
}
