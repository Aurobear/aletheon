//! Restart-safe genome rollback snapshots.

use anyhow::{bail, Context, Result};
use fabric::Genome;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fs::OpenOptions, io::Write, path::PathBuf, sync::Mutex};

#[derive(Debug, Clone, Serialize, Deserialize)]
struct GenomeSnapshot {
    version: String,
    digest: String,
    genome: Genome,
}

#[derive(Debug, Default, Serialize, Deserialize)]
struct GenomeStore {
    current_version: Option<String>,
    current_digest: Option<String>,
    snapshots: Vec<GenomeSnapshot>,
}

pub struct RollbackManager {
    store: Mutex<GenomeStore>,
    store_path: Option<PathBuf>,
}

impl RollbackManager {
    pub fn new() -> Self {
        Self {
            store: Mutex::new(GenomeStore::default()),
            store_path: None,
        }
    }

    pub fn with_path(path: PathBuf) -> Result<Self> {
        let store = if path.exists() {
            serde_json::from_slice(
                &std::fs::read(&path).with_context(|| format!("read {}", path.display()))?,
            )
            .with_context(|| format!("parse {}", path.display()))?
        } else {
            GenomeStore::default()
        };
        Ok(Self {
            store: Mutex::new(store),
            store_path: Some(path),
        })
    }

    fn digest(genome: &Genome) -> Result<String> {
        Ok(format!("{:x}", Sha256::digest(serde_json::to_vec(genome)?)))
    }

    fn persist(&self, store: &GenomeStore) -> Result<()> {
        let Some(path) = &self.store_path else {
            return Ok(());
        };
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        let tmp = path.with_extension("tmp");
        let bytes = serde_json::to_vec_pretty(store)?;
        let mut file = OpenOptions::new()
            .create(true)
            .truncate(true)
            .write(true)
            .open(&tmp)?;
        file.write_all(&bytes)?;
        file.sync_all()?;
        drop(file);
        std::fs::rename(&tmp, path)?;
        if let Some(parent) = path.parent() {
            OpenOptions::new().read(true).open(parent)?.sync_all()?;
        }
        let readback: GenomeStore = serde_json::from_slice(&std::fs::read(path)?)?;
        if readback.current_digest != store.current_digest
            || readback.current_version != store.current_version
        {
            bail!("genome store read-back mismatch");
        }
        Ok(())
    }

    pub fn save_snapshot(&self, version: &str, genome: &Genome) -> Result<()> {
        let mut store = self.store.lock().unwrap();
        let digest = Self::digest(genome)?;
        store.snapshots.push(GenomeSnapshot {
            version: version.into(),
            digest: digest.clone(),
            genome: genome.clone(),
        });
        store.current_version = Some(version.into());
        store.current_digest = Some(digest);
        self.persist(&store)
    }

    pub fn record_current(&self, version: &str, genome: &Genome) -> Result<()> {
        let mut store = self.store.lock().unwrap();
        store.current_version = Some(version.into());
        store.current_digest = Some(Self::digest(genome)?);
        self.persist(&store)
    }

    pub async fn rollback(&self) -> Result<Genome> {
        let mut store = self.store.lock().unwrap();
        let snapshot = store
            .snapshots
            .pop()
            .ok_or_else(|| anyhow::anyhow!("No previous genome version to roll back to"))?;
        if Self::digest(&snapshot.genome)? != snapshot.digest {
            bail!("rollback snapshot digest mismatch");
        }
        store.current_version = Some(snapshot.version.clone());
        store.current_digest = Some(snapshot.digest.clone());
        self.persist(&store)?;
        Ok(snapshot.genome)
    }

    pub fn current_version(&self) -> Option<String> {
        self.store.lock().unwrap().current_version.clone()
    }
    pub fn snapshot_count(&self) -> usize {
        self.store.lock().unwrap().snapshots.len()
    }
}

impl Default for RollbackManager {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::genome::loader::GenomeLoader;
    #[tokio::test]
    async fn snapshots_survive_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("store.json");
        let genome = GenomeLoader::new()
            .load(std::path::Path::new("/missing"))
            .unwrap();
        RollbackManager::with_path(path.clone())
            .unwrap()
            .save_snapshot("0.1.0", &genome)
            .unwrap();
        let reopened = RollbackManager::with_path(path).unwrap();
        assert_eq!(reopened.snapshot_count(), 1);
        assert_eq!(
            reopened.rollback().await.unwrap().identity.name,
            genome.identity.name
        );
    }
}
