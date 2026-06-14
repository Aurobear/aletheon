//! Self-reader — reads the agent's own genome and runtime state.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;
use crate::core::types::Genome;

pub struct SelfReader;

impl SelfReader {
    pub fn new() -> Self { Self }

    /// Read the current genome from the runtime environment.
    pub async fn read(&self) -> Result<Genome> {
        todo!("SelfReader: read not yet implemented")
    }
}
