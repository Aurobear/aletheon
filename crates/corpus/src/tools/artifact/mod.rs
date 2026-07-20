//! Artifact Store — content-addressed storage for large tool outputs (Wave 2A).
//! When a tool result exceeds the context budget, the full output is saved
//! here and the model receives a structured ArtifactRef + summary instead.

use sha2::{Digest, Sha256};
use std::collections::HashMap;
use std::path::PathBuf;

#[derive(Clone, Debug)]
pub struct ArtifactRef {
    pub id: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub mime: String,
}

pub struct ArtifactStore {
    dir: PathBuf,
    index: HashMap<String, ArtifactRef>,
}

impl ArtifactStore {
    pub fn new(dir: PathBuf) -> Self {
        std::fs::create_dir_all(&dir).ok();
        Self { dir, index: HashMap::new() }
    }

    pub fn store(&mut self, content: &[u8], mime: &str) -> ArtifactRef {
        let hash = format!("{:x}", Sha256::digest(content));
        let id = &hash[..12];
        let path = self.dir.join(id);
        std::fs::write(&path, content).ok();
        let r = ArtifactRef {
            id: id.to_string(),
            sha256: hash,
            size_bytes: content.len() as u64,
            mime: mime.into(),
        };
        self.index.insert(id.to_string(), r.clone());
        r
    }

    pub fn read(&self, id: &str) -> Option<Vec<u8>> {
        self.index.get(id)?;
        std::fs::read(self.dir.join(id)).ok()
    }

    pub fn read_page(&self, id: &str, offset: u64, limit: u64) -> Option<Vec<u8>> {
        let data = self.read(id)?;
        let start = offset as usize;
        let end = ((offset + limit) as usize).min(data.len());
        Some(data[start..end].to_vec())
    }

    pub fn list(&self) -> Vec<&ArtifactRef> {
        self.index.values().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn artifact_store_round_trip() {
        let tmp = tempfile::tempdir().unwrap();
        let mut store = ArtifactStore::new(tmp.path().to_path_buf());
        let content = b"x".repeat(100_000);
        let r = store.store(&content, "text/plain");
        assert_eq!(r.size_bytes, 100_000);

        let back = store.read(&r.id).unwrap();
        assert_eq!(back.len(), 100_000);

        let page = store.read_page(&r.id, 0, 100).unwrap();
        assert_eq!(page.len(), 100);
    }
}
