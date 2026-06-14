//! ContinuityLayer — lineage records for identity continuity.
//!
//! Tracks identity changes over time. `is_continuous()` returns true
//! if there are no gaps longer than a configured threshold (default 24h)
//! between consecutive identity records.

use chrono::{DateTime, Duration, Utc};
use parking_lot::RwLock;

/// A lineage record — a snapshot of identity at a point in time.
#[derive(Debug, Clone)]
pub struct LineageRecord {
    pub identity_name: String,
    pub identity_version: String,
    pub recorded_at: DateTime<Utc>,
    pub event: String,
}

/// ContinuityLayer — tracks identity lineage and checks for continuity gaps.
pub struct ContinuityLayer {
    records: RwLock<Vec<LineageRecord>>,
    /// Maximum allowed gap between records (default 24 hours).
    max_gap: Duration,
}

impl ContinuityLayer {
    pub fn new(max_gap: Duration) -> Self {
        Self {
            records: RwLock::new(Vec::new()),
            max_gap,
        }
    }

    /// Record a lineage event.
    pub fn record(
        &self,
        identity_name: &str,
        identity_version: &str,
        event: &str,
    ) {
        let entry = LineageRecord {
            identity_name: identity_name.to_string(),
            identity_version: identity_version.to_string(),
            recorded_at: Utc::now(),
            event: event.to_string(),
        };
        self.records.write().push(entry);
    }

    /// Check if the lineage is continuous (no gap > max_gap between records).
    pub fn is_continuous(&self) -> bool {
        let records = self.records.read();
        if records.len() < 2 {
            return true;
        }
        for window in records.windows(2) {
            let gap = window[1].recorded_at - window[0].recorded_at;
            if gap > self.max_gap {
                return false;
            }
        }
        true
    }

    /// Get all lineage records.
    pub fn all_records(&self) -> Vec<LineageRecord> {
        self.records.read().clone()
    }

    /// Number of lineage records.
    pub fn len(&self) -> usize {
        self.records.read().len()
    }

    /// Whether there are no records.
    pub fn is_empty(&self) -> bool {
        self.records.read().is_empty()
    }
}

impl Default for ContinuityLayer {
    fn default() -> Self {
        Self::new(Duration::hours(24))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chrono::Duration;

    #[test]
    fn single_record_is_continuous() {
        let layer = ContinuityLayer::default();
        layer.record("aurb", "0.1.0", "initialized");
        assert!(layer.is_continuous());
    }

    #[test]
    fn empty_is_continuous() {
        let layer = ContinuityLayer::default();
        assert!(layer.is_continuous());
    }

    #[test]
    fn no_gap_is_continuous() {
        let layer = ContinuityLayer::new(Duration::hours(1));
        layer.record("aurb", "0.1.0", "init");
        layer.record("aurb", "0.2.0", "upgrade");
        assert!(layer.is_continuous());
    }

    #[test]
    fn gap_exceeds_threshold() {
        // We can't easily simulate time passing in a unit test,
        // but we can test the structure by manually constructing records.
        let layer = ContinuityLayer {
            records: RwLock::new(vec![
                LineageRecord {
                    identity_name: "aurb".to_string(),
                    identity_version: "0.1.0".to_string(),
                    recorded_at: Utc::now() - Duration::hours(48),
                    event: "init".to_string(),
                },
                LineageRecord {
                    identity_name: "aurb".to_string(),
                    identity_version: "0.2.0".to_string(),
                    recorded_at: Utc::now(),
                    event: "upgrade".to_string(),
                },
            ]),
            max_gap: Duration::hours(24),
        };
        assert!(!layer.is_continuous());
    }
}
