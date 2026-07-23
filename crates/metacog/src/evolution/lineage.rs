//! Lineage — tracks the history of runtime versions AND causal links
//! from problem identification through to measured evolution outcomes.
//!
//! In-memory implementation using a Vec. Each migration records a new entry
//! with the version, parent version, description, and timestamp.
//! Optionally persists to a JSONL file via `with_path()`.

use anyhow::Result;
use fabric::{wall_to_datetime, Clock};
use serde::{Deserialize, Serialize};
use std::path::PathBuf;
use std::sync::Arc;

/// A record of a runtime version in the lineage.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LineageEntry {
    pub version: String,
    pub parent_version: Option<String>,
    pub description: String,
    pub timestamp: chrono::DateTime<chrono::Utc>,
}

// ---------------------------------------------------------------------------
// Lineage link — typed causal references
// ---------------------------------------------------------------------------

/// A single link in the causal chain from problem to measured outcome.
///
/// Each link carries stable string IDs and content hashes, never
/// embedded payloads.  This keeps the lineage compact and allows
/// every step of the evolution cycle to be addressed after restart.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineageLink {
    /// Stable identifier for the problem that triggered the improvement.
    pub problem_id: String,
    /// Stable identifier for the improvement proposal.
    pub proposal_id: String,
    /// Stable identifier for the mutation intent generated from the proposal.
    pub mutation_id: String,
    /// Stable identifier for the generated candidate.
    pub candidate_id: String,
    /// Stable identifier for the approval decision.
    pub approval_id: String,
    /// SHA-256 digest of the sandbox evaluation report payload.
    pub evaluation_hash: String,
    /// Stable identifier for the experiment outcome.
    pub outcome_id: String,
}

impl LineageLink {
    /// Create a new lineage link.
    pub fn new(
        problem_id: String,
        proposal_id: String,
        mutation_id: String,
        candidate_id: String,
        approval_id: String,
        evaluation_hash: String,
        outcome_id: String,
    ) -> Self {
        Self {
            problem_id,
            proposal_id,
            mutation_id,
            candidate_id,
            approval_id,
            evaluation_hash,
            outcome_id,
        }
    }
}

/// Tracks the history of runtime versions.
///
/// Thread-safe storage with optional JSONL file persistence.
/// Supports both synchronous (for MigrationManager) and async (for pipeline)
/// access patterns.
pub struct LineageTracker {
    entries: std::sync::Mutex<Vec<LineageEntry>>,
    path: Option<PathBuf>,
    clock: Arc<dyn Clock>,
}

impl LineageTracker {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            entries: std::sync::Mutex::new(Vec::new()),
            path: None,
            clock,
        }
    }

    /// Create a tracker backed by a JSONL file. If the file exists, its
    /// entries are loaded into memory on construction.
    pub fn with_path(path: PathBuf, clock: Arc<dyn Clock>) -> anyhow::Result<Self> {
        let mut entries = Vec::new();
        if path.exists() {
            let file = std::fs::File::open(&path)?;
            let reader = std::io::BufReader::new(file);
            for line in std::io::BufRead::lines(reader) {
                let line = line?;
                if line.trim().is_empty() {
                    continue;
                }
                if let Ok(entry) = serde_json::from_str::<LineageEntry>(&line) {
                    entries.push(entry);
                }
            }
        }
        if let Some(parent) = path.parent() {
            std::fs::create_dir_all(parent)?;
        }
        Ok(Self {
            entries: std::sync::Mutex::new(entries),
            path: Some(path),
            clock,
        })
    }

    /// Record a new version in the lineage (synchronous, used by MigrationManager).
    pub fn record(&self, version: &str, parent_version: Option<&str>, description: &str) {
        let entry = LineageEntry {
            version: version.to_string(),
            parent_version: parent_version.map(|s| s.to_string()),
            description: description.to_string(),
            timestamp: wall_to_datetime(self.clock.wall_now()),
        };
        self.append_to_file(&entry);
        let mut entries = self.entries.lock().unwrap();
        entries.push(entry);
    }

    /// Record a pre-built lineage entry (async, for pipeline use).
    pub async fn record_entry(&self, entry: &LineageEntry) -> Result<()> {
        self.append_to_file(entry);
        let mut entries = self.entries.lock().unwrap();
        entries.push(entry.clone());
        Ok(())
    }

    /// Append a single entry to the JSONL file if a path is configured.
    fn append_to_file(&self, entry: &LineageEntry) {
        if let Some(ref path) = self.path {
            if let Ok(mut file) = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
            {
                use std::io::Write;
                let _ = writeln!(file, "{}", serde_json::to_string(entry).unwrap_or_default());
            }
        }
    }

    /// Get the full lineage history (sync).
    pub fn history(&self) -> Vec<LineageEntry> {
        let entries = self.entries.lock().unwrap();
        entries.clone()
    }

    /// Get the full lineage history (async, for pipeline use).
    pub async fn history_async(&self) -> Result<Vec<LineageEntry>> {
        let entries = self.entries.lock().unwrap();
        Ok(entries.clone())
    }

    /// Get the latest version in the lineage, if any.
    pub fn latest(&self) -> Option<LineageEntry> {
        let entries = self.entries.lock().unwrap();
        entries.last().cloned()
    }

    /// Get the number of lineage entries.
    pub fn count(&self) -> usize {
        let entries = self.entries.lock().unwrap();
        entries.len()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn Clock> {
        Arc::new(TestClock::default())
    }

    #[test]
    fn test_persistence_roundtrip() {
        let tmp = tempfile::NamedTempFile::new().unwrap();
        let path = tmp.path().with_extension("jsonl");
        {
            let tracker = LineageTracker::with_path(path.clone(), test_clock()).unwrap();
            tracker.record("0.1.0", None, "initial");
            tracker.record("0.2.0", Some("0.1.0"), "first evolution");
            assert_eq!(tracker.count(), 2);
        }
        {
            let tracker = LineageTracker::with_path(path.clone(), test_clock()).unwrap();
            assert_eq!(tracker.count(), 2);
            let history = tracker.history();
            assert_eq!(history[0].version, "0.1.0");
            assert_eq!(history[1].parent_version, Some("0.1.0".to_string()));
        }
    }

    #[test]
    fn test_memory_only_tracker() {
        let tracker = LineageTracker::new(test_clock());
        tracker.record("0.1.0", None, "test");
        assert_eq!(tracker.count(), 1);
    }
}
