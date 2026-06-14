//! Genome YAML loader — reads genome files and produces a Genome struct.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;
use std::path::Path;
use crate::core::types::Genome;

/// Loads a genome from a directory of YAML files.
pub struct GenomeLoader;

impl GenomeLoader {
    pub fn new() -> Self { Self }

    /// Load genome from a directory containing topology.yaml, identity.yaml, etc.
    pub async fn load(&self, _dir: &Path) -> Result<Genome> {
        todo!("Genome YAML loading not yet implemented")
    }

    /// Save genome to a directory of YAML files.
    pub async fn save(&self, _genome: &Genome, _dir: &Path) -> Result<()> {
        todo!("Genome YAML saving not yet implemented")
    }
}
