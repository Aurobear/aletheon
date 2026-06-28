//! NarrativeLayer — ring buffer decision log.
//!
//! The narrative is the agent's self-narrative: a record of every significant
//! decision and why it was made. Uses a fixed-capacity ring buffer so old
//! entries are evicted when the buffer is full.

use std::collections::VecDeque;

use anyhow::{Context, Result};
use chrono::{DateTime, Utc};
use parking_lot::RwLock;
use serde::{Deserialize, Serialize};

use crate::core::store::SelfFieldStore;

/// A single narrative entry.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeEntry {
    pub event: String,
    pub reason: String,
    pub action: Option<String>,
    pub verdict: Option<String>,
    pub timestamp: DateTime<Utc>,
}

/// Summary of narrative patterns and trends.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct NarrativeSummary {
    /// Total number of entries currently in the buffer.
    pub total_entries: usize,
    /// The most frequently occurring event types (event, count), sorted descending.
    pub top_themes: Vec<(String, usize)>,
    /// Summary of the last N entries (event + reason).
    pub recent_trajectory: Vec<(String, String)>,
    /// Events that appear 3+ times — potential recurring issues.
    pub recurring_issues: Vec<String>,
}

/// Trajectory analysis: is the agent improving or declining?
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TrajectoryAnalysis {
    /// Ratio of Allow verdicts to total verdicts in the analysis window.
    pub allow_ratio: f64,
    /// Ratio of Deny verdicts to total verdicts.
    pub deny_ratio: f64,
    /// Ratio of RequireConfirmation verdicts to total verdicts.
    pub confirm_ratio: f64,
    /// Trend direction: true = more allows recently, false = more denials recently.
    pub improving: bool,
    /// Dominant event types in recent entries.
    pub dominant_topics: Vec<String>,
}

/// NarrativeLayer — in-memory ring buffer of decision records.
pub struct NarrativeLayer {
    buffer: RwLock<VecDeque<NarrativeEntry>>,
    capacity: usize,
}

impl NarrativeLayer {
    /// Create with a given capacity (default 1000).
    pub fn new(capacity: usize) -> Self {
        Self {
            buffer: RwLock::new(VecDeque::with_capacity(capacity)),
            capacity,
        }
    }

    /// Record a decision event.
    pub fn record(
        &self,
        event: &str,
        reason: &str,
        action: Option<&str>,
        verdict: &impl std::fmt::Debug,
    ) {
        let entry = NarrativeEntry {
            event: event.to_string(),
            reason: reason.to_string(),
            action: action.map(|s| s.to_string()),
            verdict: Some(format!("{:?}", verdict)),
            timestamp: Utc::now(),
        };
        let mut buffer = self.buffer.write();
        if buffer.len() >= self.capacity {
            buffer.pop_front();
        }
        buffer.push_back(entry);
    }

    /// Record a narrative event (for SelfFieldOps::narrate).
    pub fn narrate(&self, event: &str, reason: &str) {
        let entry = NarrativeEntry {
            event: event.to_string(),
            reason: reason.to_string(),
            action: None,
            verdict: None,
            timestamp: Utc::now(),
        };
        let mut buffer = self.buffer.write();
        if buffer.len() >= self.capacity {
            buffer.pop_front();
        }
        buffer.push_back(entry);
    }

    /// Get the N most recent entries (newest last).
    pub fn recent(&self, n: usize) -> Vec<NarrativeEntry> {
        let buffer = self.buffer.read();
        let skip = if n >= buffer.len() {
            0
        } else {
            buffer.len() - n
        };
        buffer.iter().skip(skip).cloned().collect()
    }

    /// Total entries currently stored.
    pub fn len(&self) -> usize {
        self.buffer.read().len()
    }

    /// Whether the buffer is empty.
    pub fn is_empty(&self) -> bool {
        self.buffer.read().is_empty()
    }

    /// Buffer capacity.
    pub fn capacity(&self) -> usize {
        self.capacity
    }

    /// Summarize the narrative: entry count, top themes, recent trajectory,
    /// and recurring issues.
    pub fn summarize(&self) -> NarrativeSummary {
        let buffer = self.buffer.read();
        let total_entries = buffer.len();

        // Count event frequencies
        let mut theme_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for entry in buffer.iter() {
            *theme_counts.entry(entry.event.clone()).or_insert(0) += 1;
        }
        let mut top_themes: Vec<(String, usize)> = theme_counts.into_iter().collect();
        top_themes.sort_by(|a, b| b.1.cmp(&a.1));
        top_themes.truncate(10);

        // Recent trajectory (last 5 entries)
        let recent_n = 5.min(buffer.len());
        let recent_trajectory: Vec<(String, String)> = buffer
            .iter()
            .rev()
            .take(recent_n)
            .map(|e| (e.event.clone(), e.reason.clone()))
            .collect::<Vec<_>>()
            .into_iter()
            .rev()
            .collect();

        // Recurring issues: events appearing 3+ times
        let recurring_issues: Vec<String> = top_themes
            .iter()
            .filter(|(_, count)| *count >= 3)
            .map(|(event, _)| event.clone())
            .collect();

        NarrativeSummary {
            total_entries,
            top_themes,
            recent_trajectory,
            recurring_issues,
        }
    }

    /// Analyze trajectory: are things improving or declining?
    ///
    /// Looks at verdict types in the `verdict` field of entries to compute
    /// allow/deny/confirm ratios and determine a trend.
    pub fn analyze_trajectory(&self) -> TrajectoryAnalysis {
        let buffer = self.buffer.read();
        let mut allow_count = 0usize;
        let mut deny_count = 0usize;
        let mut confirm_count = 0usize;
        let mut total_verdicts = 0usize;

        // Count dominant topics from the second half (recent) vs first half (older)
        let half = buffer.len() / 2;
        let mut recent_denies = 0usize;
        let mut recent_allows = 0usize;
        let mut topic_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();

        for (i, entry) in buffer.iter().enumerate() {
            // Count topics
            *topic_counts.entry(entry.event.clone()).or_insert(0) += 1;

            // Parse verdict string for ratio computation
            if let Some(ref verdict_str) = entry.verdict {
                total_verdicts += 1;
                if verdict_str.starts_with("Allow") && !verdict_str.starts_with("AllowWith") {
                    allow_count += 1;
                    if i >= half {
                        recent_allows += 1;
                    }
                } else if verdict_str.starts_with("Deny") {
                    deny_count += 1;
                    if i >= half {
                        recent_denies += 1;
                    }
                } else if verdict_str.starts_with("RequireConfirmation") {
                    confirm_count += 1;
                }
            }
        }

        let allow_ratio = if total_verdicts > 0 {
            allow_count as f64 / total_verdicts as f64
        } else {
            0.0
        };
        let deny_ratio = if total_verdicts > 0 {
            deny_count as f64 / total_verdicts as f64
        } else {
            0.0
        };
        let confirm_ratio = if total_verdicts > 0 {
            confirm_count as f64 / total_verdicts as f64
        } else {
            0.0
        };

        // Trend: improving if recent half has more allows than denies relative to older half
        let recent_total = recent_allows + recent_denies;
        let improving = if recent_total > 0 {
            recent_allows >= recent_denies
        } else {
            // No recent verdict data — fall back to overall ratio
            allow_ratio > deny_ratio
        };

        // Dominant topics: top 5 by frequency
        let mut dominant_topics: Vec<(String, usize)> = topic_counts.into_iter().collect();
        dominant_topics.sort_by(|a, b| b.1.cmp(&a.1));
        let dominant_topics: Vec<String> = dominant_topics
            .into_iter()
            .take(5)
            .map(|(t, _)| t)
            .collect();

        TrajectoryAnalysis {
            allow_ratio,
            deny_ratio,
            confirm_ratio,
            improving,
            dominant_topics,
        }
    }

    /// Persist all current buffer entries to the store (replaces existing rows).
    pub fn save_to_store(&self, store: &SelfFieldStore) -> Result<()> {
        let conn = store.conn();
        conn.execute("DELETE FROM narrative_entries", [])
            .context("Failed to clear narrative_entries")?;

        let buffer = self.buffer.read();
        let mut stmt = conn
            .prepare(
                "INSERT INTO narrative_entries (event, reason, action, verdict, timestamp)
                 VALUES (?1, ?2, ?3, ?4, ?5)",
            )
            .context("Failed to prepare narrative insert")?;
        for entry in buffer.iter() {
            stmt.execute(rusqlite::params![
                entry.event,
                entry.reason,
                entry.action,
                entry.verdict,
                entry.timestamp.to_rfc3339(),
            ])
            .context("Failed to insert narrative entry")?;
        }
        Ok(())
    }

    /// Load entries from the store into the buffer, replacing current contents.
    pub fn load_from_store(&mut self, store: &SelfFieldStore) -> Result<()> {
        let conn = store.conn();
        let mut stmt = conn
            .prepare(
                "SELECT event, reason, action, verdict, timestamp
                 FROM narrative_entries ORDER BY id ASC",
            )
            .context("Failed to prepare narrative select")?;

        let rows = stmt
            .query_map([], |row| {
                let ts_str: String = row.get(4)?;
                Ok(NarrativeEntry {
                    event: row.get(0)?,
                    reason: row.get(1)?,
                    action: row.get(2)?,
                    verdict: row.get(3)?,
                    timestamp: DateTime::parse_from_rfc3339(&ts_str)
                        .map(|dt| dt.with_timezone(&Utc))
                        .unwrap_or_else(|_| Utc::now()),
                })
            })
            .context("Failed to query narrative_entries")?;

        let mut entries = Vec::new();
        for row in rows {
            entries.push(row.context("Failed to read narrative row")?);
        }

        // Trim to capacity if needed
        let start = if entries.len() > self.capacity {
            entries.len() - self.capacity
        } else {
            0
        };
        let entries: Vec<_> = entries[start..].to_vec();

        let mut buffer = self.buffer.write();
        *buffer = entries.into();
        Ok(())
    }
}

impl Default for NarrativeLayer {
    fn default() -> Self {
        Self::new(1000)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::Verdict;

    #[test]
    fn record_and_retrieve() {
        let layer = NarrativeLayer::new(100);
        layer.record(
            "boundary_check",
            "deny: rm",
            Some("rm -rf /"),
            &Verdict::Deny {
                reason: "bad".to_string(),
            },
        );

        let entries = layer.recent(10);
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].event, "boundary_check");
        assert_eq!(entries[0].reason, "deny: rm");
        assert_eq!(entries[0].action, Some("rm -rf /".to_string()));
    }

    #[test]
    fn capacity_eviction() {
        let layer = NarrativeLayer::new(3);
        layer.narrate("e1", "r1");
        layer.narrate("e2", "r2");
        layer.narrate("e3", "r3");
        assert_eq!(layer.len(), 3);

        layer.narrate("e4", "r4");
        assert_eq!(layer.len(), 3);

        let entries = layer.recent(10);
        assert_eq!(entries[0].event, "e2");
        assert_eq!(entries[1].event, "e3");
        assert_eq!(entries[2].event, "e4");
    }

    #[test]
    fn recent_limits() {
        let layer = NarrativeLayer::new(100);
        for i in 0..10 {
            layer.narrate(&format!("e{}", i), "reason");
        }

        let entries = layer.recent(3);
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].event, "e7");
        assert_eq!(entries[2].event, "e9");
    }

    #[test]
    fn save_and_load_roundtrip() {
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let store = crate::core::store::SelfFieldStore::new(tmp.path().to_path_buf()).unwrap();

        let layer = NarrativeLayer::new(100);
        layer.narrate("evt1", "reason1");
        layer.narrate("evt2", "reason2");

        layer.save_to_store(&store).unwrap();

        let mut loaded = NarrativeLayer::new(100);
        loaded.load_from_store(&store).unwrap();

        assert_eq!(loaded.len(), 2);
        let entries = loaded.recent(10);
        assert_eq!(entries[0].event, "evt1");
        assert_eq!(entries[1].reason, "reason2");
    }

    #[test]
    fn summarize_basic() {
        let layer = NarrativeLayer::new(100);
        layer.narrate("init", "startup");
        layer.narrate("review", "allowed");
        layer.narrate("review", "allowed again");
        layer.narrate("boundary_check", "denied");
        layer.narrate("review", "allowed third");

        let summary = layer.summarize();
        assert_eq!(summary.total_entries, 5);
        // "review" appears 3 times, should be the top theme
        assert_eq!(summary.top_themes[0], ("review".to_string(), 3));
        // "review" appears 3+ times, should be a recurring issue
        assert!(summary.recurring_issues.contains(&"review".to_string()));
        // Recent trajectory should have 5 entries
        assert_eq!(summary.recent_trajectory.len(), 5);
        assert_eq!(summary.recent_trajectory[0].0, "init");
        assert_eq!(summary.recent_trajectory[4].0, "review");
    }

    #[test]
    fn summarize_empty() {
        let layer = NarrativeLayer::new(100);
        let summary = layer.summarize();
        assert_eq!(summary.total_entries, 0);
        assert!(summary.top_themes.is_empty());
        assert!(summary.recent_trajectory.is_empty());
        assert!(summary.recurring_issues.is_empty());
    }

    #[test]
    fn analyze_trajectory_with_verdicts() {
        let layer = NarrativeLayer::new(100);
        // Record some entries with verdicts
        layer.record(
            "review",
            "ok",
            Some("ls"),
            &Verdict::Allow,
        );
        layer.record(
            "boundary_check",
            "deny: rm",
            Some("rm -rf /"),
            &Verdict::Deny {
                reason: "bad".to_string(),
            },
        );
        layer.record(
            "review",
            "ok again",
            Some("cat"),
            &Verdict::Allow,
        );

        let analysis = layer.analyze_trajectory();
        assert!((analysis.allow_ratio - 2.0 / 3.0).abs() < 0.01);
        assert!((analysis.deny_ratio - 1.0 / 3.0).abs() < 0.01);
        assert!(analysis.improving);
        assert!(analysis.dominant_topics.contains(&"review".to_string()));
    }

    #[test]
    fn analyze_trajectory_declining() {
        let layer = NarrativeLayer::new(100);
        // Old allows, recent denials
        layer.record("review", "ok", Some("ls"), &Verdict::Allow);
        layer.record("review", "ok", Some("cat"), &Verdict::Allow);
        layer.record(
            "boundary_check",
            "blocked",
            Some("rm"),
            &Verdict::Deny {
                reason: "bad".to_string(),
            },
        );
        layer.record(
            "boundary_check",
            "blocked",
            Some("purge"),
            &Verdict::Deny {
                reason: "bad".to_string(),
            },
        );

        let analysis = layer.analyze_trajectory();
        // Recent half has more denies than allows -> not improving
        assert!(!analysis.improving);
    }

    #[test]
    fn save_clears_previous() {
        use tempfile::NamedTempFile;

        let tmp = NamedTempFile::new().unwrap();
        let store = crate::core::store::SelfFieldStore::new(tmp.path().to_path_buf()).unwrap();

        let layer = NarrativeLayer::new(100);
        layer.narrate("old", "old_reason");
        layer.save_to_store(&store).unwrap();

        let layer2 = NarrativeLayer::new(100);
        layer2.narrate("new", "new_reason");
        layer2.save_to_store(&store).unwrap();

        let mut loaded = NarrativeLayer::new(100);
        loaded.load_from_store(&store).unwrap();
        assert_eq!(loaded.len(), 1);
        assert_eq!(loaded.recent(10)[0].event, "new");
    }
}
