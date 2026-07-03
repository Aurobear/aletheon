//! Runtime builder — constructs a new runtime from a genome.
//!
//! Converts a Genome into a RuntimeCandidate that can be tested
//! and potentially adopted by the morphogenesis pipeline.

use anyhow::Result;
use base::{Genome, RuntimeCandidate};

pub struct RuntimeBuilder;

impl Default for RuntimeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl RuntimeBuilder {
    pub fn new() -> Self {
        Self
    }

    /// Build a RuntimeCandidate from a genome.
    ///
    /// Creates a new candidate with a fresh UUID, the provided genome,
    /// and an empty changes list (changes are populated by the pipeline).
    pub async fn build(&self, genome: &Genome) -> Result<RuntimeCandidate> {
        Ok(RuntimeCandidate {
            id: uuid::Uuid::new_v4(),
            genome: genome.clone(),
            changes: Vec::new(),
            generated_at: chrono::Utc::now(),
        })
    }

    /// Build a RuntimeCandidate from a genome with explicit change descriptions.
    pub async fn build_with_changes(
        &self,
        genome: &Genome,
        changes: Vec<String>,
    ) -> Result<RuntimeCandidate> {
        Ok(RuntimeCandidate {
            id: uuid::Uuid::new_v4(),
            genome: genome.clone(),
            changes,
            generated_at: chrono::Utc::now(),
        })
    }
}
