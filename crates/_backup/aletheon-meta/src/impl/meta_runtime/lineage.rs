//! Lineage — tracks the history of runtime versions.
//!
//! In-memory implementation using a Vec. Each migration records a new entry
//! with the version, parent version, description, and timestamp.

use anyhow::Result;

/// A record of a runtime version in the lineage.
#[derive(Debug, Clone)]
pub struct LineageEntry {
    pub version: String,
    pub parent_version: Option<String>,
    pub description: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

/// Tracks the history of runtime versions.
///
/// Thread-safe in-memory storage. Supports both synchronous (for MigrationManager)
/// and async (for pipeline) access patterns.
pub struct LineageTracker {
    entries: std::sync::Mutex<Vec<LineageEntry>>,
}

impl LineageTracker {
    pub fn new() -> Self {
        Self {
            entries: std::sync::Mutex::new(Vec::new()),
        }
    }

    /// Record a new version in the lineage (synchronous, used by MigrationManager).
    pub fn record(&self, version: &str, parent_version: Option<&str>, description: &str) {
        let entry = LineageEntry {
            version: version.to_string(),
            parent_version: parent_version.map(|s| s.to_string()),
            description: description.to_string(),
            timestamp: chrono::Utc::now(),
        };
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.push(entry);
    }

    /// Record a pre-built lineage entry (async, for pipeline use).
    pub async fn record_entry(&self, entry: &LineageEntry) -> Result<()> {
        let mut entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.push(entry.clone());
        Ok(())
    }

    /// Get the full lineage history (sync).
    pub fn history(&self) -> Vec<LineageEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.clone()
    }

    /// Get the full lineage history (async, for pipeline use).
    pub async fn history_async(&self) -> Result<Vec<LineageEntry>> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        Ok(entries.clone())
    }

    /// Get the latest version in the lineage, if any.
    pub fn latest(&self) -> Option<LineageEntry> {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.last().cloned()
    }

    /// Get the number of lineage entries.
    pub fn count(&self) -> usize {
        let entries = self.entries.lock().unwrap_or_else(|e| e.into_inner());
        entries.len()
    }
}

impl Default for LineageTracker {
    fn default() -> Self {
        Self::new()
    }
}
