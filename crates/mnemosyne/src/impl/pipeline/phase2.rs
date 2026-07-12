use std::path::{Path, PathBuf};
use std::sync::Arc;

use fabric::wall_to_datetime;
use tokio::fs;
use tracing::{debug, info, warn};
use uuid::Uuid;

use super::state_db::StateDatabase;
use super::Phase2Config;

/// Result of a Phase 2 consolidation run.
#[derive(Debug, Clone)]
pub struct ConsolidationResult {
    /// Number of raw memories incorporated.
    pub memories_processed: usize,
    /// The generated diff (what changed in the consolidated memory).
    pub diff: String,
    /// Path to the written raw_memories.md.
    pub output_path: PathBuf,
}

/// Phase 2 consolidator: merges individual session extractions into a
/// single global memory document.
///
/// Only one Phase 2 can run at a time (global lock). It loads succeeded
/// Phase 1 outputs, sorts by usage/recency, syncs the workspace with
/// rollout summaries, and produces a consolidated raw_memories.md.
pub struct Phase2Consolidator {
    config: Phase2Config,
    clock: Arc<dyn fabric::Clock>,
}

impl Phase2Consolidator {
    pub fn new(config: Phase2Config, clock: Arc<dyn fabric::Clock>) -> Self {
        Self { config, clock }
    }

    /// Run Phase 2 consolidation.
    ///
    /// Acquires the global Phase 2 lock, processes succeeded sessions,
    /// writes the consolidated output, and releases the lock.
    pub async fn run(
        &self,
        state_db: &mut StateDatabase,
        memory_root: &Path,
    ) -> anyhow::Result<ConsolidationResult> {
        let claim_id = Uuid::new_v4().to_string();

        if !state_db.acquire_phase2_lock(&claim_id) {
            anyhow::bail!(
                "Phase 2 is already running (held by {})",
                state_db.phase2_lock_holder().unwrap_or("unknown")
            );
        }

        let result = self.do_consolidate(state_db, memory_root, &claim_id).await;

        // Always release the lock, even on failure.
        if let Err(e) = state_db.release_phase2_lock(&claim_id) {
            warn!(error = %e, "Failed to release Phase 2 lock");
        }

        result
    }

    async fn do_consolidate(
        &self,
        state_db: &mut StateDatabase,
        memory_root: &Path,
        _claim_id: &str,
    ) -> anyhow::Result<ConsolidationResult> {
        let succeeded = state_db.succeeded_sessions();

        if succeeded.is_empty() {
            info!("No succeeded sessions to consolidate");
            return Ok(ConsolidationResult {
                memories_processed: 0,
                diff: String::new(),
                output_path: memory_root.join("raw_memories.md"),
            });
        }

        // Take up to max_raw_memories.
        let to_process: Vec<_> = succeeded
            .into_iter()
            .take(self.config.max_raw_memories)
            .collect();

        info!(count = to_process.len(), "Consolidating Phase 1 outputs");

        // Sync workspace: write individual rollout summaries.
        let summaries_dir = memory_root.join("rollout_summaries");
        fs::create_dir_all(&summaries_dir).await?;

        for record in &to_process {
            if let (Some(summary), Some(slug)) = (&record.summary, &record.slug) {
                let summary_path = summaries_dir.join(format!("{}.md", slug));
                fs::write(&summary_path, summary).await?;
                debug!(path = %summary_path.display(), "Wrote rollout summary");
            }
        }

        // Rebuild raw_memories.md from all raw memories.
        let output_path = memory_root.join("raw_memories.md");
        let old_content = fs::read_to_string(&output_path).await.unwrap_or_default();

        let mut sections = Vec::new();
        sections.push("# Raw Memories\n".to_string());
        sections.push(format!(
            "_Consolidated from {} sessions at {}_\n",
            to_process.len(),
            wall_to_datetime(self.clock.wall_now()).to_rfc3339()
        ));

        for record in &to_process {
            if let Some(raw) = &record.raw_memory {
                sections.push(format!("## Session: {}\n\n{}\n", record.session_id, raw));
            }
        }

        let new_content = sections.join("\n");

        // Generate diff summary.
        let diff = generate_diff_summary(&old_content, &new_content);

        fs::write(&output_path, &new_content).await?;
        info!(path = %output_path.display(), "Wrote consolidated raw_memories.md");

        Ok(ConsolidationResult {
            memories_processed: to_process.len(),
            diff,
            output_path,
        })
    }

    /// Get the configured model name.
    pub fn model(&self) -> &str {
        &self.config.model
    }
}

/// Generate a human-readable diff summary between old and new content.
fn generate_diff_summary(old: &str, new: &str) -> String {
    let old_lines: Vec<&str> = old.lines().collect();
    let new_lines: Vec<&str> = new.lines().collect();

    let added = new_lines.len().saturating_sub(old_lines.len());
    let removed = old_lines.len().saturating_sub(new_lines.len());

    if old.is_empty() {
        format!("Initial creation: {} lines", new_lines.len())
    } else if new.is_empty() {
        format!("All content removed (was {} lines)", old_lines.len())
    } else {
        format!(
            "Updated: {} lines added, {} lines removed ({} -> {} total)",
            added,
            removed,
            old_lines.len(),
            new_lines.len()
        )
    }
}

/// Filter sessions by unused_days threshold — mark sessions as stale
/// if they haven't been used within max_unused_days.
pub fn filter_stale_sessions(
    state_db: &mut StateDatabase,
    max_unused_days: u32,
    now: u64,
) -> Vec<String> {
    let threshold_secs = max_unused_days as u64 * 86400;
    let mut stale_ids = Vec::new();

    // Collect stale session IDs first to avoid borrow conflict.
    let ids: Vec<String> = state_db
        .sessions_by_status(&super::state_db::Stage1Status::Succeeded)
        .iter()
        .filter(|r| now.saturating_sub(r.last_used) > threshold_secs)
        .map(|r| r.session_id.clone())
        .collect();

    stale_ids.extend(ids);
    stale_ids
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::r#impl::pipeline::state_db::{SessionRecord, Stage1Status};
    use std::path::PathBuf;
    use tempfile::TempDir;

    fn default_phase2_config() -> Phase2Config {
        Phase2Config {
            lease_seconds: 3600,
            max_raw_memories: 20,
            max_unused_days: 14,
            model: "test-model".to_string(),
        }
    }

    fn setup_db_with_succeeded(n: usize) -> (StateDatabase, u64) {
        let mut db = StateDatabase::new();
        let now = 1000u64;
        for i in 0..n {
            let mut r = SessionRecord::new(
                format!("s{}", i),
                PathBuf::from(format!("/tmp/s{}", i)),
                now,
            );
            r.stage1_status = Stage1Status::Succeeded;
            r.raw_memory = Some(format!("Raw memory content for session {}", i));
            r.summary = Some(format!("Summary for session {}", i));
            r.slug = Some(format!("session-{}", i));
            r.usage_count = (n - i) as u32; // decreasing usage
            r.last_used = now;
            db.upsert_session(r);
        }
        (db, now)
    }

    #[tokio::test]
    async fn test_consolidate_with_sessions() {
        let tmp = TempDir::new().unwrap();
        let memory_root = tmp.path();

        let (mut db, _) = setup_db_with_succeeded(3);
        let config = default_phase2_config();
        let clock: Arc<dyn fabric::Clock> =
            Arc::new(aletheon_kernel::chronos::TestClock::default());
        let consolidator = Phase2Consolidator::new(config, clock);

        let result = consolidator.run(&mut db, memory_root).await.unwrap();
        assert_eq!(result.memories_processed, 3);
        assert!(result.diff.contains("Initial creation"));
        assert!(result.output_path.exists());

        // Check raw_memories.md was written.
        let content = fs::read_to_string(&result.output_path).await.unwrap();
        assert!(content.contains("# Raw Memories"));
        assert!(content.contains("Session: s0"));
        assert!(content.contains("Session: s1"));
        assert!(content.contains("Session: s2"));

        // Check rollout summaries were written.
        let summaries_dir = memory_root.join("rollout_summaries");
        assert!(summaries_dir.join("session-0.md").exists());
        assert!(summaries_dir.join("session-1.md").exists());
        assert!(summaries_dir.join("session-2.md").exists());
    }

    #[tokio::test]
    async fn test_consolidate_empty() {
        let tmp = TempDir::new().unwrap();
        let memory_root = tmp.path();

        let mut db = StateDatabase::new();
        let config = default_phase2_config();
        let clock: Arc<dyn fabric::Clock> =
            Arc::new(aletheon_kernel::chronos::TestClock::default());
        let consolidator = Phase2Consolidator::new(config, clock);

        let result = consolidator.run(&mut db, memory_root).await.unwrap();
        assert_eq!(result.memories_processed, 0);
        assert!(result.diff.is_empty());
    }

    #[tokio::test]
    async fn test_consolidate_respects_max_raw_memories() {
        let tmp = TempDir::new().unwrap();
        let memory_root = tmp.path();

        let (mut db, _) = setup_db_with_succeeded(10);

        let config = Phase2Config {
            lease_seconds: 3600,
            max_raw_memories: 3,
            max_unused_days: 14,
            model: "test-model".to_string(),
        };
        let clock: Arc<dyn fabric::Clock> =
            Arc::new(aletheon_kernel::chronos::TestClock::default());
        let consolidator = Phase2Consolidator::new(config, clock);

        let result = consolidator.run(&mut db, memory_root).await.unwrap();
        assert_eq!(result.memories_processed, 3);
    }

    #[tokio::test]
    async fn test_consolidate_fails_when_locked() {
        let tmp = TempDir::new().unwrap();
        let memory_root = tmp.path();

        let mut db = StateDatabase::new();
        db.acquire_phase2_lock("other-claim");

        let config = default_phase2_config();
        let clock: Arc<dyn fabric::Clock> =
            Arc::new(aletheon_kernel::chronos::TestClock::default());
        let consolidator = Phase2Consolidator::new(config, clock);

        let result = consolidator.run(&mut db, memory_root).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("already running"));
    }

    #[tokio::test]
    async fn test_consolidate_releases_lock_on_success() {
        let tmp = TempDir::new().unwrap();
        let memory_root = tmp.path();

        let (mut db, _) = setup_db_with_succeeded(1);
        let config = default_phase2_config();
        let clock: Arc<dyn fabric::Clock> =
            Arc::new(aletheon_kernel::chronos::TestClock::default());
        let consolidator = Phase2Consolidator::new(config, clock);

        assert!(!db.is_phase2_locked());
        consolidator.run(&mut db, memory_root).await.unwrap();
        // Lock should be released after successful run.
        assert!(!db.is_phase2_locked());
    }

    #[test]
    fn test_filter_stale_sessions() {
        let mut db = StateDatabase::new();
        let now = 10_000_000u64;

        for i in 0..3 {
            let mut r = SessionRecord::new(
                format!("s{}", i),
                PathBuf::from(format!("/tmp/s{}", i)),
                now,
            );
            r.stage1_status = Stage1Status::Succeeded;
            r.last_used = now; // All current.
            db.upsert_session(r);
        }

        // Make s0 stale: last_used more than 14 days (1,209,600 seconds) ago.
        if let Some(r) = db.get_session_mut("s0") {
            r.last_used = now - 2_000_000; // ~23 days ago
        }

        let stale = filter_stale_sessions(&mut db, 14, now);
        assert!(stale.contains(&"s0".to_string()));
        assert!(!stale.contains(&"s1".to_string()));
        assert!(!stale.contains(&"s2".to_string()));
    }

    #[test]
    fn test_generate_diff_summary_initial() {
        let diff = generate_diff_summary("", "# New content\nLine 2\n");
        assert!(diff.contains("Initial creation"));
        assert!(diff.contains("2 lines"));
    }

    #[test]
    fn test_generate_diff_summary_update() {
        let diff = generate_diff_summary("line1\nline2\n", "line1\nline2\nline3\n");
        assert!(diff.contains("1 lines added"));
    }
}
