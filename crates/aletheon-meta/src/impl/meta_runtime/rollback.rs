//! Rollback — reverts to a previous runtime version.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;

pub struct RollbackManager;

impl RollbackManager {
    pub fn new() -> Self { Self }

    /// Rollback to the previous runtime version.
    pub async fn rollback(&self) -> Result<()> {
        todo!("RollbackManager: rollback not yet implemented")
    }
}
