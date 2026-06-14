//! Lineage — tracks the history of runtime versions.
//!
//! Design skeleton. Implementation comes in a future round.

use anyhow::Result;
use aletheon_abi::Version;

/// A record of a runtime version in the lineage.
#[derive(Debug, Clone)]
pub struct LineageEntry {
    pub version: Version,
    pub parent_version: Option<Version>,
    pub description: String,
    pub timestamp: String,
}

pub struct LineageTracker;

impl LineageTracker {
    pub fn new() -> Self { Self }

    /// Record a new version in the lineage.
    pub async fn record(&self, _entry: &LineageEntry) -> Result<()> {
        todo!("LineageTracker: record not yet implemented")
    }

    /// Get the full lineage history.
    pub async fn history(&self) -> Result<Vec<LineageEntry>> {
        todo!("LineageTracker: history not yet implemented")
    }
}
