pub mod phase1;
pub mod phase2;
pub mod state_db;

use std::path::PathBuf;

use tracing::info;

pub use phase1::{ExtractionResult, Phase1Extractor};
pub use phase2::{ConsolidationResult, Phase2Consolidator};
pub use state_db::{SessionRecord, Stage1Status, StateDatabase};

/// Configuration for Phase 1 (Session Extraction).
#[derive(Debug, Clone)]
pub struct Phase1Config {
    /// Maximum number of concurrent extraction tasks.
    pub concurrency_limit: usize,
    /// Maximum sessions to claim per pipeline startup.
    pub max_claims_per_startup: usize,
    /// Skip sessions older than this many days.
    pub max_age_days: u32,
    /// Only claim sessions idle for at least this many hours.
    pub min_idle_hours: u32,
    /// Duration of a Phase 1 claim lease in seconds.
    pub lease_seconds: u64,
    /// Model identifier for extraction.
    pub model: String,
}

impl Default for Phase1Config {
    fn default() -> Self {
        Self {
            concurrency_limit: 8,
            max_claims_per_startup: 50,
            max_age_days: 30,
            min_idle_hours: 2,
            lease_seconds: 3600,
            model: "default".to_string(),
        }
    }
}

/// Configuration for Phase 2 (Global Consolidation).
#[derive(Debug, Clone)]
pub struct Phase2Config {
    /// Duration of the Phase 2 global lock lease in seconds.
    pub lease_seconds: u64,
    /// Maximum raw memories to consolidate in one pass.
    pub max_raw_memories: usize,
    /// Mark sessions unused for this many days as stale.
    pub max_unused_days: u32,
    /// Model identifier for consolidation.
    pub model: String,
}

impl Default for Phase2Config {
    fn default() -> Self {
        Self {
            lease_seconds: 3600,
            max_raw_memories: 20,
            max_unused_days: 14,
            model: "default".to_string(),
        }
    }
}

/// Two-phase memory pipeline for processing session rollouts into
/// consolidated long-term memory.
///
/// **Phase 1** (`Phase1Extractor`): Claims eligible sessions, extracts
/// memory-relevant content, redacts secrets, and stores per-session
/// raw memories.
///
/// **Phase 2** (`Phase2Consolidator`): Merges individual extractions
/// into a single global `raw_memories.md` under the memory root.
pub struct MemoryPipeline {
    state_db: StateDatabase,
    memory_root: PathBuf,
    phase1_config: Phase1Config,
    phase2_config: Phase2Config,
}

impl MemoryPipeline {
    /// Create a new pipeline with the given configuration.
    pub fn new(
        memory_root: PathBuf,
        phase1_config: Phase1Config,
        phase2_config: Phase2Config,
    ) -> Self {
        Self {
            state_db: StateDatabase::new(),
            memory_root,
            phase1_config,
            phase2_config,
        }
    }

    /// Create a pipeline with default configurations.
    pub fn with_defaults(memory_root: PathBuf) -> Self {
        Self::new(memory_root, Phase1Config::default(), Phase2Config::default())
    }

    /// Get a reference to the state database.
    pub fn state_db(&self) -> &StateDatabase {
        &self.state_db
    }

    /// Get a mutable reference to the state database.
    pub fn state_db_mut(&mut self) -> &mut StateDatabase {
        &mut self.state_db
    }

    /// Get the memory root path.
    pub fn memory_root(&self) -> &std::path::Path {
        &self.memory_root
    }

    /// Register a session for tracking.
    pub fn register_session(&mut self, session_id: String, session_path: PathBuf) {
        let now = current_timestamp();
        let record = SessionRecord::new(session_id.clone(), session_path, now);
        self.state_db.upsert_session(record);
        info!(session_id, "Session registered in pipeline");
    }

    /// Run Phase 1 extraction on eligible sessions.
    pub async fn run_phase1(&mut self) -> anyhow::Result<usize> {
        let extractor = Phase1Extractor::new(self.phase1_config.clone());
        extractor.run(&mut self.state_db, &self.memory_root).await
    }

    /// Run Phase 2 consolidation.
    pub async fn run_phase2(&mut self) -> anyhow::Result<ConsolidationResult> {
        let consolidator = Phase2Consolidator::new(self.phase2_config.clone());
        consolidator
            .run(&mut self.state_db, &self.memory_root)
            .await
    }

    /// Run the full pipeline: Phase 1 then Phase 2.
    pub async fn run_full(&mut self) -> anyhow::Result<(usize, ConsolidationResult)> {
        let phase1_count = self.run_phase1().await?;
        info!(count = phase1_count, "Phase 1 complete, starting Phase 2");

        let phase2_result = self.run_phase2().await?;
        Ok((phase1_count, phase2_result))
    }
}

/// Current Unix timestamp in seconds.
fn current_timestamp() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path;
    use tempfile::TempDir;
    use tokio::fs;

    #[test]
    fn test_pipeline_creation() {
        let pipeline = MemoryPipeline::with_defaults(PathBuf::from("/tmp/memories"));
        assert_eq!(pipeline.memory_root(), Path::new("/tmp/memories"));
        assert_eq!(pipeline.state_db().session_count(), 0);
    }

    #[test]
    fn test_pipeline_with_custom_config() {
        let phase1 = Phase1Config {
            concurrency_limit: 4,
            max_claims_per_startup: 10,
            max_age_days: 7,
            min_idle_hours: 1,
            lease_seconds: 1800,
            model: "gpt-4".to_string(),
        };
        let phase2 = Phase2Config {
            lease_seconds: 1800,
            max_raw_memories: 10,
            max_unused_days: 7,
            model: "gpt-4".to_string(),
        };
        let pipeline = MemoryPipeline::new(PathBuf::from("/tmp/test"), phase1, phase2);
        assert_eq!(pipeline.phase1_config.concurrency_limit, 4);
        assert_eq!(pipeline.phase2_config.max_raw_memories, 10);
    }

    #[test]
    fn test_register_session() {
        let mut pipeline = MemoryPipeline::with_defaults(PathBuf::from("/tmp/memories"));
        pipeline.register_session("sess-1".into(), PathBuf::from("/tmp/sessions/sess-1"));

        assert_eq!(pipeline.state_db().session_count(), 1);
        let rec = pipeline.state_db().get_session("sess-1").unwrap();
        assert_eq!(rec.stage1_status, Stage1Status::Pending);
    }

    #[test]
    fn test_config_defaults() {
        let p1 = Phase1Config::default();
        assert_eq!(p1.concurrency_limit, 8);
        assert_eq!(p1.max_claims_per_startup, 50);
        assert_eq!(p1.max_age_days, 30);
        assert_eq!(p1.min_idle_hours, 2);
        assert_eq!(p1.lease_seconds, 3600);
        assert_eq!(p1.model, "default");

        let p2 = Phase2Config::default();
        assert_eq!(p2.lease_seconds, 3600);
        assert_eq!(p2.max_raw_memories, 20);
        assert_eq!(p2.max_unused_days, 14);
        assert_eq!(p2.model, "default");
    }

    #[tokio::test]
    async fn test_full_pipeline_integration() {
        let tmp = TempDir::new().unwrap();
        let memory_root = tmp.path().to_path_buf();
        let sessions_dir = memory_root.join("sessions");

        // Create two sessions with rollout files.
        for sid in &["sess-a", "sess-b"] {
            let dir = sessions_dir.join(sid);
            fs::create_dir_all(&dir).await.unwrap();
            let rollout = serde_json::to_string(&vec![
                serde_json::json!({"role": "user", "content": "Help me debug the authentication module in this application."}),
                serde_json::json!({"role": "assistant", "content": "I found the issue: the JWT token validation was missing the audience claim check. Here's the fix with proper error handling."}),
            ])
            .unwrap();
            fs::write(dir.join("rollout.json"), rollout).await.unwrap();
        }

        let phase1 = Phase1Config {
            concurrency_limit: 2,
            max_claims_per_startup: 10,
            max_age_days: 30,
            min_idle_hours: 0, // Allow immediate claiming.
            lease_seconds: 3600,
            model: "test".to_string(),
        };
        let phase2 = Phase2Config::default();

        let mut pipeline = MemoryPipeline::new(memory_root.clone(), phase1, phase2);

        // Register sessions with old timestamps so they're eligible.
        let now = current_timestamp();
        for sid in &["sess-a", "sess-b"] {
            pipeline.register_session(sid.to_string(), sessions_dir.join(sid));
            // Backdate to make them idle.
            if let Some(r) = pipeline.state_db_mut().get_session_mut(sid) {
                r.last_used = now - 7200;
                r.created_at = now - 7200;
            }
        }

        let (phase1_count, phase2_result) = pipeline.run_full().await.unwrap();

        assert_eq!(phase1_count, 2);
        assert_eq!(phase2_result.memories_processed, 2);
        assert!(phase2_result.output_path.exists());

        // Verify the consolidated output.
        let content = fs::read_to_string(&phase2_result.output_path).await.unwrap();
        assert!(content.contains("# Raw Memories"));
        assert!(content.contains("Session: sess-a"));
        assert!(content.contains("Session: sess-b"));

        // Verify rollout summaries exist.
        let summaries_dir = memory_root.join("rollout_summaries");
        assert!(summaries_dir.join("sess-a.md").exists());
        assert!(summaries_dir.join("sess-b.md").exists());
    }
}
