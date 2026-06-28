//! Rollback — reverts to a previous runtime version.
//!
//! Tracks previous genome versions in memory and restores from the
//! lineage when rollback is requested.

use anyhow::{bail, Result};
use std::sync::Mutex;
use aletheon_abi::Genome;

/// A snapshot of a genome version for rollback.
#[derive(Debug, Clone)]
struct GenomeSnapshot {
    version: String,
    genome: Genome,
}

/// Manages rollback to previous genome versions.
///
/// Maintains an in-memory stack of genome snapshots. Each migration
/// pushes the current genome before applying changes; rollback pops
/// and restores the previous version.
pub struct RollbackManager {
    snapshots: Mutex<Vec<GenomeSnapshot>>,
    current_version: Mutex<Option<String>>,
}

impl RollbackManager {
    pub fn new() -> Self {
        Self {
            snapshots: Mutex::new(Vec::new()),
            current_version: Mutex::new(None),
        }
    }

    /// Save a genome snapshot before migration (call before applying changes).
    pub fn save_snapshot(&self, version: &str, genome: &Genome) {
        let mut snapshots = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        snapshots.push(GenomeSnapshot {
            version: version.to_string(),
            genome: genome.clone(),
        });
        let mut current = self.current_version.lock().unwrap_or_else(|e| e.into_inner());
        *current = Some(version.to_string());
    }

    /// Rollback to the previous genome version.
    ///
    /// Pops the most recent snapshot and returns it.
    /// Fails if there are no previous versions to roll back to.
    pub async fn rollback(&self) -> Result<Genome> {
        let mut snapshots = self.snapshots.lock().unwrap_or_else(|e| e.into_inner());
        match snapshots.pop() {
            Some(snapshot) => {
                let mut current = self.current_version.lock().unwrap_or_else(|e| e.into_inner());
                *current = Some(snapshot.version.clone());
                tracing::info!("Rolled back to genome version {}", snapshot.version);
                Ok(snapshot.genome)
            }
            None => {
                bail!("No previous genome version to roll back to");
            }
        }
    }

    /// Get the current version string, if set.
    pub fn current_version(&self) -> Option<String> {
        self.current_version.lock().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Get the number of snapshots available for rollback.
    pub fn snapshot_count(&self) -> usize {
        self.snapshots.lock().unwrap_or_else(|e| e.into_inner()).len()
    }
}

impl Default for RollbackManager {
    fn default() -> Self {
        Self::new()
    }
}
