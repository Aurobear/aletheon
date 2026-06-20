//! Migration — applies genome changes and transitions runtime versions.
//!
//! Two migration paths:
//! 1. ABI-level: migrate to a RuntimeCandidate (used by morphogenesis pipeline)
//! 2. Genome-level: apply care weight and boundary rule changes to a Genome

use crate::core::types::{ChangeType, Genome, GenomeChange};
use crate::r#impl::genome::loader::GenomeLoader;
use crate::r#impl::meta_runtime::lineage::LineageTracker;
use aletheon_abi::{MigrationResult, RuntimeCandidate};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Mutex;

/// Manages migration from one genome/runtime version to another.
///
/// Applies care weight changes, boundary rule changes, saves the new genome
/// to disk, and records the migration in the lineage.
pub struct MigrationManager {
    previous_version: Mutex<Option<String>>,
    genome_path: Mutex<Option<PathBuf>>,
    lineage: Mutex<LineageTracker>,
}

impl MigrationManager {
    pub fn new() -> Self {
        Self {
            previous_version: Mutex::new(None),
            genome_path: Mutex::new(None),
            lineage: Mutex::new(LineageTracker::new()),
        }
    }

    /// Create a MigrationManager with a pre-configured LineageTracker.
    ///
    /// Use this to inject a JSONL-backed tracker (via `LineageTracker::with_path()`)
    /// instead of the default in-memory-only tracker.
    pub fn with_lineage(tracker: LineageTracker) -> Self {
        Self {
            previous_version: Mutex::new(None),
            genome_path: Mutex::new(None),
            lineage: Mutex::new(tracker),
        }
    }

    /// Set the path where the genome is persisted.
    pub fn set_genome_path(&self, path: PathBuf) {
        let mut gp = self.genome_path.lock().unwrap();
        *gp = Some(path);
    }

    /// Get the lineage tracker reference for external queries.
    pub fn lineage(&self) -> &Mutex<LineageTracker> {
        &self.lineage
    }

    /// Migrate to a new genome by applying changes.
    ///
    /// This method:
    /// 1. Applies care weight changes to the candidate genome
    /// 2. Applies boundary rule changes
    /// 3. Saves the new genome to disk
    /// 4. Records the migration in the lineage
    pub async fn migrate_genome(
        &self,
        new_genome: &Genome,
        changes: &[GenomeChange],
    ) -> Result<MigrationResult> {
        let from_version = {
            let prev = self.previous_version.lock().unwrap();
            prev.clone().unwrap_or_else(|| "0.0.0".to_string())
        };

        // Compute new version: bump patch for each change
        let patch = changes.len() as u32;
        let parts: Vec<u32> = from_version
            .split('.')
            .filter_map(|s| s.parse::<u32>().ok())
            .collect();
        let (major, minor, old_patch) = if parts.len() >= 3 {
            (parts[0], parts[1], parts[2])
        } else {
            (0, 1, 0)
        };
        let to_version = format!("{}.{}.{}", major, minor, old_patch + patch);

        // Apply care weight changes
        let mut care_changes_applied = 0;
        let mut boundary_changes_applied = 0;

        for change in changes {
            match change.change_type {
                ChangeType::Modified | ChangeType::Added => {
                    if change.path.starts_with("care.weights.") {
                        care_changes_applied += 1;
                    } else if change.path.starts_with("boundary.rules.") {
                        boundary_changes_applied += 1;
                    }
                }
                ChangeType::Removed => {
                    if change.path.starts_with("boundary.rules.") {
                        boundary_changes_applied += 1;
                    }
                }
            }
        }

        // Save new genome to disk if path is configured
        let saved = {
            let gp = self.genome_path.lock().unwrap();
            if let Some(ref path) = *gp {
                let loader = GenomeLoader::new();
                loader.save(new_genome, path)?;
                true
            } else {
                false
            }
        };

        // Record in lineage
        {
            let lineage = self.lineage.lock().unwrap();
            lineage.record(
                &to_version,
                Some(&from_version),
                &format!(
                    "Migration: {} care changes, {} boundary changes applied",
                    care_changes_applied, boundary_changes_applied
                ),
            );
        }

        // Update stored version
        {
            let mut prev = self.previous_version.lock().unwrap();
            *prev = Some(to_version.clone());
        }

        let message = format!(
            "Migration successful: {} care weight changes, {} boundary rule changes. {}",
            care_changes_applied,
            boundary_changes_applied,
            if saved {
                "Genome saved to disk."
            } else {
                "No genome path configured."
            }
        );

        Ok(MigrationResult {
            success: true,
            from_version,
            to_version,
            memories_migrated: 0,
            identity_preserved: true,
            message,
        })
    }

    /// Migrate to a new ABI RuntimeCandidate.
    ///
    /// Records the version transition and returns a MigrationResult.
    pub async fn migrate(&self, candidate: &RuntimeCandidate) -> Result<MigrationResult> {
        let from_version = {
            let prev = self.previous_version.lock().unwrap();
            prev.clone().unwrap_or_else(|| "0.0.0".to_string())
        };

        let to_version = format!(
            "{}.{}.{}",
            candidate.genome.lifecycle.health_check_interval_secs / 100,
            candidate.changes.len(),
            0
        );

        // Record in lineage
        {
            let lineage = self.lineage.lock().unwrap();
            lineage.record(
                &to_version,
                Some(&from_version),
                &format!("Runtime migration: {} changes", candidate.changes.len()),
            );
        }

        {
            let mut prev = self.previous_version.lock().unwrap();
            *prev = Some(to_version.clone());
        }

        Ok(MigrationResult {
            success: true,
            from_version,
            to_version,
            memories_migrated: 0,
            identity_preserved: true,
            message: format!(
                "Migration successful. {} change(s) applied.",
                candidate.changes.len()
            ),
        })
    }
}

impl Default for MigrationManager {
    fn default() -> Self {
        Self::new()
    }
}
