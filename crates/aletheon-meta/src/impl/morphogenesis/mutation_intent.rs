//! Mutation intent generation — how the agent decides what to change.
//!
//! Design skeleton. Implementation comes in a future round.

use aletheon_abi::MutationIntent;

/// Generate mutation intents from reflection and experience.
pub struct MutationIntentGenerator;

impl MutationIntentGenerator {
    pub fn new() -> Self { Self }

    /// Generate mutation intents based on recent experience and reflection.
    pub async fn generate(&self, _context: &str) -> Vec<MutationIntent> {
        todo!("Mutation intent generation not yet implemented")
    }
}
