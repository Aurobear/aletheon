//! L2 -> L3 memory consolidation.
//!
//! Promotes frequently-accessed episodic memories (reflections) to semantic
//! knowledge. High-activation reflections become long-term semantic entries.

use anyhow::Result;
use chrono::Utc;
use uuid::Uuid;

use base::{MemoryBackend, MemoryEntry, MemoryType};

use crate::ops::activation::{compute_activation, ActivationEntry};
use crate::backends::episodic::EpisodicMemory;
use crate::backends::semantic::SemanticMemory;

/// Configuration for memory consolidation.
pub struct ConsolidationConfig {
    /// Minimum activation score to qualify for promotion.
    pub min_activation: f64,
    /// Minimum access count to qualify.
    pub min_access_count: usize,
    /// Maximum items to consolidate per run.
    pub batch_size: usize,
}

impl Default for ConsolidationConfig {
    fn default() -> Self {
        Self {
            min_activation: 0.7,
            min_access_count: 3,
            batch_size: 10,
        }
    }
}

/// Summary of a consolidation run.
pub struct ConsolidationResult {
    /// Number of episodic entries promoted to semantic.
    pub promoted: usize,
    /// Human-readable summaries of promoted items.
    pub entries: Vec<String>,
}

/// Promote high-activation episodic memories to semantic knowledge.
///
/// For each qualifying reflection (activation >= threshold AND access_count >= threshold):
/// 1. Extract knowledge from reflection fields
/// 2. Store as semantic memory with consolidated tags
/// 3. Lower the episodic entry's importance (soft archival)
pub async fn consolidate(
    episodic: &EpisodicMemory,
    semantic: &SemanticMemory,
    config: &ConsolidationConfig,
) -> Result<ConsolidationResult> {
    // 1. Query recent reflections with access counts from episodic memory.
    //    We use the join helper to get (ReflectionEntry, access_count, importance).
    let reflections_with_access =
        episodic.recall_reflections_with_access(config.batch_size * 2)?;
    if reflections_with_access.is_empty() {
        return Ok(ConsolidationResult {
            promoted: 0,
            entries: vec![],
        });
    }

    // 2. Filter by activation score and access count.
    let now = Utc::now().timestamp();
    let mut qualified: Vec<(&base::ReflectionEntry, u64, &str)> = Vec::new();

    for (reflection, access_count, importance) in &reflections_with_access {
        let activation = compute_activation(
            &ActivationEntry::new(
                *importance,
                *access_count as i64,
                reflection.timestamp.timestamp(),
            ),
            now,
        );

        if activation >= config.min_activation
            && *access_count as usize >= config.min_access_count
        {
            // We need the memory_id for soft archival. Since recall_reflections_with_access
            // doesn't return it, we'll get it from the reflection_events table.
            // For now, store the reflection id — we'll look up memory_id below.
            qualified.push((reflection, *access_count, ""));
        }
    }

    if qualified.is_empty() {
        return Ok(ConsolidationResult {
            promoted: 0,
            entries: vec![],
        });
    }

    // 3. Get memory_ids for soft archival by querying reflection_events.
    let memory_ids = episodic.get_reflection_memory_ids(
        qualified.iter().map(|(r, _, _)| r.id.as_str()).collect(),
    )?;

    // 4. Promote qualifying entries.
    let mut promoted = 0;
    let mut entries = Vec::new();

    for (i, (reflection, _access_count, _)) in
        qualified.iter().take(config.batch_size).enumerate()
    {
        // a. Extract knowledge from reflection.
        let knowledge = extract_knowledge(reflection);

        // b. Store as semantic memory with consolidated tags.
        let semantic_entry = MemoryEntry {
            id: Uuid::new_v4(),
            memory_type: MemoryType::Semantic,
            content: knowledge.content.into_bytes(),
            tags: vec![
                "consolidated".into(),
                "from_episodic".into(),
                format!("source:{}", reflection.id),
            ],
            created_at: Utc::now(),
            access_count: 0,
            importance: knowledge.importance,
            decay_rate: 0.01, // slow decay for consolidated knowledge
            associations: vec![],
        };

        semantic.store(semantic_entry).await?;

        // c. Soft-archive: lower importance of the source episodic entry.
        if let Some(memory_id) = memory_ids.get(i) {
            episodic.lower_importance(memory_id, 0.1)?;
        }

        entries.push(reflection.task_summary.clone());
        promoted += 1;
    }

    Ok(ConsolidationResult { promoted, entries })
}

/// Extract structured knowledge from a reflection entry.
struct ExtractedKnowledge {
    content: String,
    importance: f64,
}

fn extract_knowledge(reflection: &base::ReflectionEntry) -> ExtractedKnowledge {
    let mut parts = Vec::new();

    // Topic: task_summary
    parts.push(format!("[Topic] {}", reflection.task_summary));

    // Positive knowledge: what worked
    if !reflection.what_worked.is_empty() {
        parts.push(format!(
            "[What worked] {}",
            reflection.what_worked.join("; ")
        ));
    }

    // Negative knowledge: what to avoid
    if !reflection.what_failed.is_empty() {
        parts.push(format!(
            "[What to avoid] {}",
            reflection.what_failed.join("; ")
        ));
    }

    // Explicit lessons
    if !reflection.learned.is_empty() {
        parts.push(format!("[Lessons] {}", reflection.learned.join("; ")));
    }

    // Importance: scale from reflection confidence (0.0-1.0)
    // Consolidated knowledge is inherently valuable, boost by 0.1
    let importance = (reflection.confidence + 0.1).min(1.0);

    ExtractedKnowledge {
        content: parts.join("\n"),
        importance,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::{
        ReflectionEntry, ReflectionOutcome, ReflectionTrigger, Subsystem, SubsystemContext,
    };

    fn make_reflection(
        task_summary: &str,
        confidence: f64,
        what_worked: Vec<&str>,
        what_failed: Vec<&str>,
        learned: Vec<&str>,
    ) -> ReflectionEntry {
        ReflectionEntry {
            id: Uuid::new_v4().to_string(),
            timestamp: Utc::now(),
            trigger: ReflectionTrigger::TaskComplete,
            task_summary: task_summary.into(),
            outcome: ReflectionOutcome::Success,
            what_worked: what_worked.into_iter().map(|s| s.into()).collect(),
            what_failed: what_failed.into_iter().map(|s| s.into()).collect(),
            learned: learned.into_iter().map(|s| s.into()).collect(),
            behavior_changes: vec![],
            confidence,
        }
    }

    async fn init_episodic(mem: &mut EpisodicMemory) {
        let ctx = SubsystemContext {
            name: "test".into(),
            working_dir: std::env::temp_dir(),
            config: serde_json::Value::Null,
            bus: std::sync::Arc::new(base::CommunicationBus::new()),
        };
        mem.init(&ctx).await.unwrap();
    }

    async fn init_semantic(mem: &mut SemanticMemory) {
        let ctx = SubsystemContext {
            name: "test".into(),
            working_dir: std::env::temp_dir(),
            config: serde_json::Value::Null,
            bus: std::sync::Arc::new(base::CommunicationBus::new()),
        };
        mem.init(&ctx).await.unwrap();
    }

    /// Helper: store a reflection and a matching base entry with given access_count.
    async fn store_reflection_with_access(
        ep: &EpisodicMemory,
        reflection: &ReflectionEntry,
        access_count: u64,
    ) {
        ep.store_reflection(reflection).unwrap();

        // The store_reflection creates a base entry with access_count=0.
        // We need to update it to have the desired access_count.
        // Use the memory_id from the reflection_events table.
        let memory_ids = ep
            .get_reflection_memory_ids(vec![reflection.id.as_str()])
            .unwrap();
        if let Some(memory_id) = memory_ids.first() {
            ep.with_conn(|conn| {
                conn.execute(
                    "UPDATE memory SET access_count = ?1 WHERE id = ?2",
                    rusqlite::params![access_count as i64, memory_id],
                )?;
                Ok(())
            })
            .unwrap();
        }
    }

    #[tokio::test]
    async fn test_empty_episodic_noop() {
        let tmp_ep = tempfile::NamedTempFile::new().unwrap();
        let tmp_se = tempfile::NamedTempFile::new().unwrap();
        let mut ep = EpisodicMemory::new(tmp_ep.path().to_path_buf());
        let mut se = SemanticMemory::new(tmp_se.path().to_path_buf());
        init_episodic(&mut ep).await;
        init_semantic(&mut se).await;

        let config = ConsolidationConfig::default();
        let result = consolidate(&ep, &se, &config).await.unwrap();
        assert_eq!(result.promoted, 0);
        assert!(result.entries.is_empty());
    }

    #[tokio::test]
    async fn test_low_activation_skipped() {
        let tmp_ep = tempfile::NamedTempFile::new().unwrap();
        let tmp_se = tempfile::NamedTempFile::new().unwrap();
        let mut ep = EpisodicMemory::new(tmp_ep.path().to_path_buf());
        let mut se = SemanticMemory::new(tmp_se.path().to_path_buf());
        init_episodic(&mut ep).await;
        init_semantic(&mut se).await;

        // Store a reflection with low confidence (low importance -> low activation)
        let reflection = make_reflection(
            "low value task",
            0.1,
            vec!["nothing special"],
            vec![],
            vec![],
        );
        store_reflection_with_access(&ep, &reflection, 1).await;

        let config = ConsolidationConfig {
            min_activation: 0.7,
            min_access_count: 3,
            batch_size: 10,
        };
        let result = consolidate(&ep, &se, &config).await.unwrap();
        assert_eq!(result.promoted, 0);
    }

    #[tokio::test]
    async fn test_high_activation_promoted() {
        let tmp_ep = tempfile::NamedTempFile::new().unwrap();
        let tmp_se = tempfile::NamedTempFile::new().unwrap();
        let mut ep = EpisodicMemory::new(tmp_ep.path().to_path_buf());
        let mut se = SemanticMemory::new(tmp_se.path().to_path_buf());
        init_episodic(&mut ep).await;
        init_semantic(&mut se).await;

        // Store a high-value reflection with high access count
        let reflection = make_reflection(
            "optimized database queries",
            0.9,
            vec!["used EXPLAIN to find missing index"],
            vec!["full table scan was slow"],
            vec!["always check query plans before deploying"],
        );
        store_reflection_with_access(&ep, &reflection, 10).await;

        let config = ConsolidationConfig {
            min_activation: 0.5,
            min_access_count: 3,
            batch_size: 10,
        };
        let result = consolidate(&ep, &se, &config).await.unwrap();
        assert_eq!(result.promoted, 1);
        assert_eq!(result.entries.len(), 1);
        assert_eq!(result.entries[0], "optimized database queries");
    }

    #[tokio::test]
    async fn test_promoted_entries_appear_in_semantic() {
        let tmp_ep = tempfile::NamedTempFile::new().unwrap();
        let tmp_se = tempfile::NamedTempFile::new().unwrap();
        let mut ep = EpisodicMemory::new(tmp_ep.path().to_path_buf());
        let mut se = SemanticMemory::new(tmp_se.path().to_path_buf());
        init_episodic(&mut ep).await;
        init_semantic(&mut se).await;

        let reflection = make_reflection(
            "learned Rust ownership patterns",
            0.95,
            vec!["borrow checker catches use-after-free"],
            vec![],
            vec!["ownership is key to memory safety"],
        );
        store_reflection_with_access(&ep, &reflection, 10).await;

        let config = ConsolidationConfig {
            min_activation: 0.5,
            min_access_count: 3,
            batch_size: 10,
        };
        let result = consolidate(&ep, &se, &config).await.unwrap();
        assert_eq!(result.promoted, 1);

        // Verify the entry appears in semantic memory
        let query = base::MemoryQuery {
            tags: Some(vec!["consolidated".into()]),
            limit: 10,
            ..Default::default()
        };
        let semantic_entries = se.recall(&query).await.unwrap();
        assert_eq!(semantic_entries.len(), 1);
        let content = String::from_utf8_lossy(&semantic_entries[0].content);
        assert!(content.contains("learned Rust ownership patterns"));
        assert!(content.contains("[What worked]"));
        assert!(content.contains("[Lessons]"));
    }

    #[tokio::test]
    async fn test_source_episodic_lowered_importance() {
        let tmp_ep = tempfile::NamedTempFile::new().unwrap();
        let tmp_se = tempfile::NamedTempFile::new().unwrap();
        let mut ep = EpisodicMemory::new(tmp_ep.path().to_path_buf());
        let mut se = SemanticMemory::new(tmp_se.path().to_path_buf());
        init_episodic(&mut ep).await;
        init_semantic(&mut se).await;

        let reflection = make_reflection(
            "important lesson",
            0.9,
            vec!["worked well"],
            vec![],
            vec!["key insight"],
        );
        store_reflection_with_access(&ep, &reflection, 10).await;

        // Get the memory_id to check importance before consolidation
        let memory_ids = ep
            .get_reflection_memory_ids(vec![reflection.id.as_str()])
            .unwrap();
        let memory_id = memory_ids.first().unwrap();

        // Verify initial importance is 0.8 (default from store_reflection)
        let initial_importance = ep.get_importance(memory_id).unwrap();
        assert!((initial_importance - 0.8).abs() < f64::EPSILON);

        let config = ConsolidationConfig {
            min_activation: 0.5,
            min_access_count: 3,
            batch_size: 10,
        };
        let result = consolidate(&ep, &se, &config).await.unwrap();
        assert_eq!(result.promoted, 1);

        // Verify importance was lowered
        let final_importance = ep.get_importance(memory_id).unwrap();
        assert!(
            (final_importance - 0.1).abs() < f64::EPSILON,
            "importance should be lowered to 0.1, got {}",
            final_importance
        );
    }

    #[tokio::test]
    async fn test_extract_knowledge_all_fields() {
        let reflection = make_reflection(
            "refactored module X",
            0.8,
            vec!["clean separation of concerns", "faster compile"],
            vec!["initial approach was too coupled"],
            vec!["prefer composition over inheritance"],
        );

        let knowledge = extract_knowledge(&reflection);
        assert!(knowledge.content.contains("[Topic] refactored module X"));
        assert!(knowledge
            .content
            .contains("[What worked] clean separation of concerns; faster compile"));
        assert!(knowledge
            .content
            .contains("[What to avoid] initial approach was too coupled"));
        assert!(knowledge
            .content
            .contains("[Lessons] prefer composition over inheritance"));
        // importance = confidence + 0.1, capped at 1.0
        assert!((knowledge.importance - 0.9).abs() < f64::EPSILON);
    }

    #[tokio::test]
    async fn test_extract_knowledge_empty_fields() {
        let reflection = make_reflection("minimal task", 0.5, vec![], vec![], vec![]);

        let knowledge = extract_knowledge(&reflection);
        assert!(knowledge.content.contains("[Topic] minimal task"));
        assert!(!knowledge.content.contains("[What worked]"));
        assert!(!knowledge.content.contains("[What to avoid]"));
        assert!(!knowledge.content.contains("[Lessons]"));
    }
}
