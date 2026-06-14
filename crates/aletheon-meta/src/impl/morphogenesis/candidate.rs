//! Candidate runtime generation — building the next version of the agent.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;
use aletheon_abi::{Genome, RuntimeCandidate, MutationIntent};

/// Generates candidate runtimes from genome mutations.
pub struct CandidateGenerator;

impl CandidateGenerator {
    pub fn new() -> Self { Self }

    /// Generate a candidate runtime from a genome and mutation intent.
    pub async fn generate(&self, _genome: &Genome, _intent: &MutationIntent) -> Result<RuntimeCandidate> {
        todo!("Candidate generation not yet implemented")
    }
}
