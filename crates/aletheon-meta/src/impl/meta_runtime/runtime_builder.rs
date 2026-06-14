//! Runtime builder — constructs a new runtime from a genome.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;
use aletheon_abi::{Genome, RuntimeCandidate};

pub struct RuntimeBuilder;

impl RuntimeBuilder {
    pub fn new() -> Self { Self }

    /// Build a RuntimeCandidate from a genome.
    pub async fn build(&self, _genome: &Genome) -> Result<RuntimeCandidate> {
        todo!("RuntimeBuilder: build not yet implemented")
    }
}
