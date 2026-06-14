//! Migration — transitions from old runtime to new candidate.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;
use aletheon_abi::{RuntimeCandidate, MigrationResult};

pub struct MigrationManager;

impl MigrationManager {
    pub fn new() -> Self { Self }

    /// Migrate to a new runtime candidate.
    pub async fn migrate(&self, _candidate: &RuntimeCandidate) -> Result<MigrationResult> {
        todo!("MigrationManager: migrate not yet implemented")
    }
}
