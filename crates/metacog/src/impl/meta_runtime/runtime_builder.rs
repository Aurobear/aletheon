//! Runtime builder — constructs a new runtime from a genome.
//!
//! Converts a Genome into a RuntimeCandidate that can be tested
//! and potentially adopted by the morphogenesis pipeline.

use anyhow::Result;
use fabric::{wall_to_datetime, Clock, Genome, RuntimeCandidate};
use std::sync::Arc;

pub struct RuntimeBuilder {
    clock: Arc<dyn Clock>,
}

impl RuntimeBuilder {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
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
            generated_at: wall_to_datetime(self.clock.wall_now()),
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
            generated_at: wall_to_datetime(self.clock.wall_now()),
        })
    }
}
