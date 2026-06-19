//! Memory extraction pipeline — bridges RecallMemory and CoreMemory.
//!
//! Periodically scans recent conversation history (RecallMemory) and
//! reflections (EpisodicMemory) to extract facts and populate CoreMemory
//! blocks: "human" (user preferences), "learned" (knowledge), and
//! "system_state" (observations about tool usage, paths, etc.).

use std::sync::Arc;

use aletheon_abi::ReflectionEntry;
use chrono::{DateTime, Utc};
use tokio::sync::Mutex;
use tracing::{debug, info};

use super::core_memory::CoreMemory;
use super::recall_memory::RecallMemory;

use aletheon_memory::episodic::EpisodicMemory;

// ---------------------------------------------------------------------------
// Types
// ---------------------------------------------------------------------------

/// Categories for extracted facts, mapped to CoreMemory blocks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum FactCategory {
    /// User preferences and personal information -> "human" block.
    UserPreference,
    /// Knowledge learned from tasks or reflections -> "learned" block.
    LearnedKnowledge,
    /// System-level observations (tool patterns, file paths) -> "system_state" block.
    SystemObservation,
}

/// A single fact extracted from conversation or reflections.
#[derive(Debug, Clone)]
pub struct ExtractedFact {
    pub content: String,
    pub category: FactCategory,
    pub confidence: f64,
}

/// Result of an extraction-and-consolidation run.
#[derive(Debug, Clone)]
pub struct ExtractionResult {
    /// Facts extracted from recent conversation messages.
    pub from_conversation: Vec<ExtractedFact>,
    /// Facts extracted from recent reflections.
    pub from_reflections: Vec<ExtractedFact>,
    /// Timestamp of this extraction run.
    pub timestamp: DateTime<Utc>,
}

/// Configuration for the memory pipeline.
#[derive(Debug, Clone)]
pub struct MemoryPipelineConfig {
    /// How many recent messages to scan from RecallMemory.
    pub recent_message_limit: usize,
    /// How many recent reflections to scan from EpisodicMemory.
    pub recent_reflection_limit: usize,
    /// Minimum hours between extraction runs.
    pub extraction_interval_hours: u64,
}

impl Default for MemoryPipelineConfig {
    fn default() -> Self {
        Self {
            recent_message_limit: 50,
            recent_reflection_limit: 10,
            extraction_interval_hours: 6,
        }
    }
}

// ---------------------------------------------------------------------------
// MemoryPipeline
// ---------------------------------------------------------------------------

/// Pipeline that periodically extracts facts from RecallMemory and
/// EpisodicMemory, then consolidates them into CoreMemory blocks.
pub struct MemoryPipeline {
    recall_memory: Arc<Mutex<RecallMemory>>,
    core_memory: Arc<Mutex<CoreMemory>>,
    episodic_memory: Arc<Mutex<EpisodicMemory>>,
    last_extraction: Option<DateTime<Utc>>,
    config: MemoryPipelineConfig,
}

impl MemoryPipeline {
    pub fn new(
        recall_memory: Arc<Mutex<RecallMemory>>,
        core_memory: Arc<Mutex<CoreMemory>>,
        episodic_memory: Arc<Mutex<EpisodicMemory>>,
        config: MemoryPipelineConfig,
    ) -> Self {
        Self {
            recall_memory,
            core_memory,
            episodic_memory,
            last_extraction: None,
            config,
        }
    }

    /// Returns true if enough time has elapsed since the last extraction.
    pub fn should_extract(&self) -> bool {
        match self.last_extraction {
            Some(last) => {
                let elapsed = Utc::now().signed_duration_since(last);
                elapsed.num_hours() >= self.config.extraction_interval_hours as i64
            }
            None => true,
        }
    }

    /// Run the full extraction-and-consolidation cycle.
    ///
    /// 1. Pulls recent messages from RecallMemory.
    /// 2. Pulls recent reflections from EpisodicMemory.
    /// 3. Applies pattern-based fact extraction (no LLM).
    /// 4. Appends extracted facts to appropriate CoreMemory blocks.
    /// 5. Returns the ExtractionResult.
    pub async fn extract_and_consolidate(&mut self) -> anyhow::Result<ExtractionResult> {
        let now = Utc::now();
        info!("Starting memory extraction pipeline");

        // Gather recent messages.
        let messages = {
            let recall = self.recall_memory.lock().await;
            recall.recent(self.config.recent_message_limit)?
        };

        // Gather recent reflections.
        let reflections = {
            let episodic = self.episodic_memory.lock().await;
            episodic.recall_reflections(self.config.recent_reflection_limit)?
        };

        // Extract facts from conversation messages.
        let from_conversation = extract_facts_from_messages(&messages);

        // Extract facts from reflections.
        let from_reflections = self.extract_from_reflections(&reflections);

        // Consolidate into CoreMemory blocks.
        {
            let mut core = self.core_memory.lock().await;
            for fact in from_conversation.iter().chain(from_reflections.iter()) {
                let block_label = match fact.category {
                    FactCategory::UserPreference => "human",
                    FactCategory::LearnedKnowledge => "learned",
                    FactCategory::SystemObservation => "system_state",
                };
                let snippet = format!("- {}", fact.content);
                if let Err(e) = core.append(block_label, &snippet) {
                    debug!(block = block_label, error = %e, "Failed to append fact");
                }
            }
        }

        self.last_extraction = Some(now);

        let result = ExtractionResult {
            from_conversation,
            from_reflections,
            timestamp: now,
        };

        info!(
            conversation_facts = result.from_conversation.len(),
            reflection_facts = result.from_reflections.len(),
            "Memory extraction complete"
        );

        Ok(result)
    }

    /// Extract facts from reflections, reusing CoreMemory::auto_populate_learned
    /// logic plus additional pattern detection.
    pub fn extract_from_reflections(&self, reflections: &[ReflectionEntry]) -> Vec<ExtractedFact> {
        let mut facts = Vec::new();

        for entry in reflections {
            // High-confidence learned items -> LearnedKnowledge
            for lesson in &entry.learned {
                facts.push(ExtractedFact {
                    content: lesson.clone(),
                    category: FactCategory::LearnedKnowledge,
                    confidence: entry.confidence,
                });
            }

            // what_worked items with high confidence -> UserPreference
            // (user patterns that led to success)
            for lesson in &entry.what_worked {
                if entry.confidence >= 0.7 {
                    facts.push(ExtractedFact {
                        content: lesson.clone(),
                        category: FactCategory::UserPreference,
                        confidence: entry.confidence,
                    });
                }
            }
        }

        facts
    }
}

// ---------------------------------------------------------------------------
// Pattern-based extraction from conversation messages
// ---------------------------------------------------------------------------

/// Extract facts from conversation messages using pattern matching.
fn extract_facts_from_messages(
    messages: &[super::recall_memory::MemoryEntry],
) -> Vec<ExtractedFact> {
    use regex::Regex;

    // Preference patterns (case-insensitive).
    let preference_patterns: Vec<(Regex, &str)> = vec![
        (Regex::new(r"(?i)I prefer\s+(.{5,60})").unwrap(), "prefers"),
        (
            Regex::new(r"(?i)I(?:'m| am) used to\s+(.{5,60})").unwrap(),
            "is used to",
        ),
        (
            Regex::new(r"(?i)always\s+(do|use|run|check|start)\s+(.{5,60})").unwrap(),
            "always does",
        ),
        (
            Regex::new(r"(?i)don'?t\s+(ever\s+)?(.{5,60})").unwrap(),
            "doesn't want",
        ),
        (
            Regex::new(r"(?i)I (?:like|love|enjoy)\s+(.{5,60})").unwrap(),
            "likes",
        ),
        (
            Regex::new(r"(?i)I (?:hate|dislike|can't stand)\s+(.{5,60})").unwrap(),
            "dislikes",
        ),
        (
            Regex::new(r"(?i)my (?:name is|name's)\s+(.{2,40})").unwrap(),
            "is named",
        ),
        (
            Regex::new(r"(?i)I work (?:on|with)\s+(.{5,60})").unwrap(),
            "works with",
        ),
        (
            Regex::new(r"(?i)I(?:'m| am) (?:a|an)\s+(.{5,40})").unwrap(),
            "is a",
        ),
    ];

    // System observation patterns.
    let system_patterns: Vec<Regex> = vec![
        Regex::new(r"(?i)(?:project|repo|workspace)\s+(?:is (?:at|in) )?(/[\w./\-]+)").unwrap(),
        Regex::new(r"(?i)using\s+(\w+)\s+(?:as|for)\s+(.{5,40})").unwrap(),
    ];

    let mut facts = Vec::new();

    for entry in messages {
        // Only process user messages for preference extraction.
        if entry.entry_type == "user" {
            for (re, prefix) in &preference_patterns {
                for cap in re.captures_iter(&entry.content) {
                    if let Some(m) = cap.get(1).or_else(|| cap.get(2)) {
                        let text = m.as_str().trim();
                        if !text.is_empty() && text.len() > 3 {
                            facts.push(ExtractedFact {
                                content: format!("{} {}", prefix, text),
                                category: FactCategory::UserPreference,
                                confidence: 0.6,
                            });
                        }
                    }
                }
            }
        }

        // System observations from any role.
        for re in &system_patterns {
            for cap in re.captures_iter(&entry.content) {
                if let Some(m) = cap.get(1) {
                    let text = m.as_str().trim();
                    if !text.is_empty() {
                        facts.push(ExtractedFact {
                            content: text.to_string(),
                            category: FactCategory::SystemObservation,
                            confidence: 0.5,
                        });
                    }
                }
            }
        }
    }

    // Deduplicate by content (sort first so duplicates are adjacent).
    facts.sort_by(|a, b| a.content.cmp(&b.content));
    facts.dedup_by(|a, b| a.content == b.content);
    facts
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::{ReflectionOutcome, ReflectionTrigger};

    fn make_reflection(
        what_worked: Vec<&str>,
        learned: Vec<&str>,
        confidence: f64,
    ) -> ReflectionEntry {
        ReflectionEntry {
            id: "test-1".to_string(),
            timestamp: Utc::now(),
            trigger: ReflectionTrigger::TaskComplete,
            task_summary: "test task".to_string(),
            outcome: ReflectionOutcome::Success,
            what_worked: what_worked.into_iter().map(String::from).collect(),
            what_failed: vec![],
            learned: learned.into_iter().map(String::from).collect(),
            behavior_changes: vec![],
            confidence,
        }
    }

    #[test]
    fn extract_from_reflections_high_confidence() {
        // Use a dummy pipeline (we only need extract_from_reflections which
        // takes &self and does not access any fields).
        let recall = Arc::new(Mutex::new(dummy_recall()));
        let core = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        let episodic = Arc::new(Mutex::new(dummy_episodic()));

        let pipeline = MemoryPipeline {
            recall_memory: recall,
            core_memory: core,
            episodic_memory: episodic,
            last_extraction: None,
            config: MemoryPipelineConfig::default(),
        };

        let reflections = vec![make_reflection(
            vec!["use short prompts", "check disk first"],
            vec!["rust borrow checker prefers explicit lifetimes"],
            0.9,
        )];

        let facts = pipeline.extract_from_reflections(&reflections);
        assert_eq!(facts.len(), 3);

        // 2 what_worked at 0.9 confidence -> UserPreference
        let user_facts: Vec<_> = facts
            .iter()
            .filter(|f| f.category == FactCategory::UserPreference)
            .collect();
        assert_eq!(user_facts.len(), 2);

        // 1 learned -> LearnedKnowledge
        let learned_facts: Vec<_> = facts
            .iter()
            .filter(|f| f.category == FactCategory::LearnedKnowledge)
            .collect();
        assert_eq!(learned_facts.len(), 1);
        assert!(learned_facts[0].content.contains("borrow checker"));
    }

    #[test]
    fn extract_from_reflections_low_confidence_filters_what_worked() {
        let recall = Arc::new(Mutex::new(dummy_recall()));
        let core = Arc::new(Mutex::new(CoreMemory::with_defaults()));
        let episodic = Arc::new(Mutex::new(dummy_episodic()));

        let pipeline = MemoryPipeline {
            recall_memory: recall,
            core_memory: core,
            episodic_memory: episodic,
            last_extraction: None,
            config: MemoryPipelineConfig::default(),
        };

        let reflections = vec![make_reflection(
            vec!["use short prompts"],
            vec!["some lesson"],
            0.3, // below threshold
        )];

        let facts = pipeline.extract_from_reflections(&reflections);
        // what_worked should be filtered out (confidence < 0.7), only learned remains
        assert_eq!(facts.len(), 1);
        assert_eq!(facts[0].category, FactCategory::LearnedKnowledge);
    }

    #[test]
    fn should_extract_true_when_never_run() {
        let pipeline = MemoryPipeline {
            recall_memory: Arc::new(Mutex::new(dummy_recall())),
            core_memory: Arc::new(Mutex::new(CoreMemory::with_defaults())),
            episodic_memory: Arc::new(Mutex::new(dummy_episodic())),
            last_extraction: None,
            config: MemoryPipelineConfig::default(),
        };
        assert!(pipeline.should_extract());
    }

    #[test]
    fn should_extract_false_when_recent() {
        let pipeline = MemoryPipeline {
            recall_memory: Arc::new(Mutex::new(dummy_recall())),
            core_memory: Arc::new(Mutex::new(CoreMemory::with_defaults())),
            episodic_memory: Arc::new(Mutex::new(dummy_episodic())),
            last_extraction: Some(Utc::now()),
            config: MemoryPipelineConfig::default(),
        };
        assert!(!pipeline.should_extract());
    }

    #[test]
    fn should_extract_true_when_interval_elapsed() {
        let pipeline = MemoryPipeline {
            recall_memory: Arc::new(Mutex::new(dummy_recall())),
            core_memory: Arc::new(Mutex::new(CoreMemory::with_defaults())),
            episodic_memory: Arc::new(Mutex::new(dummy_episodic())),
            last_extraction: Some(Utc::now() - chrono::Duration::hours(7)),
            config: MemoryPipelineConfig {
                extraction_interval_hours: 6,
                ..Default::default()
            },
        };
        assert!(pipeline.should_extract());
    }

    #[test]
    fn extract_facts_from_messages_detects_preferences() {
        use super::super::recall_memory::MemoryEntry;

        let messages = vec![
            MemoryEntry {
                id: 1,
                timestamp: Utc::now(),
                session_id: "s1".to_string(),
                entry_type: "user".to_string(),
                content: "I prefer using vim over nano for editing".to_string(),
                metadata: None,
            },
            MemoryEntry {
                id: 2,
                timestamp: Utc::now(),
                session_id: "s1".to_string(),
                entry_type: "user".to_string(),
                content: "Always check disk space before deploying".to_string(),
                metadata: None,
            },
            MemoryEntry {
                id: 3,
                timestamp: Utc::now(),
                session_id: "s1".to_string(),
                entry_type: "user".to_string(),
                content: "Don't ever use rm -rf on production".to_string(),
                metadata: None,
            },
            MemoryEntry {
                id: 4,
                timestamp: Utc::now(),
                session_id: "s1".to_string(),
                entry_type: "user".to_string(),
                content: "My name is Aurobear".to_string(),
                metadata: None,
            },
        ];

        let facts = extract_facts_from_messages(&messages);
        assert!(!facts.is_empty());

        // Should contain preferences
        let prefs: Vec<_> = facts
            .iter()
            .filter(|f| f.category == FactCategory::UserPreference)
            .collect();
        assert!(!prefs.is_empty());

        // Check specific extractions
        let has_vim = prefs.iter().any(|f| f.content.contains("vim"));
        assert!(has_vim, "Should extract vim preference");

        let has_name = prefs.iter().any(|f| f.content.contains("Aurobear"));
        assert!(has_name, "Should extract name");
    }

    #[test]
    fn extract_facts_ignores_assistant_messages_for_preferences() {
        use super::super::recall_memory::MemoryEntry;

        let messages = vec![MemoryEntry {
            id: 1,
            timestamp: Utc::now(),
            session_id: "s1".to_string(),
            entry_type: "assistant".to_string(),
            content: "I prefer using Python for data analysis".to_string(),
            metadata: None,
        }];

        let facts = extract_facts_from_messages(&messages);
        let prefs: Vec<_> = facts
            .iter()
            .filter(|f| f.category == FactCategory::UserPreference)
            .collect();
        assert!(
            prefs.is_empty(),
            "Assistant messages should not yield user preferences"
        );
    }

    #[test]
    fn extract_facts_deduplicates() {
        use super::super::recall_memory::MemoryEntry;

        let messages = vec![
            MemoryEntry {
                id: 1,
                timestamp: Utc::now(),
                session_id: "s1".to_string(),
                entry_type: "user".to_string(),
                content: "I prefer using vim for editing".to_string(),
                metadata: None,
            },
            MemoryEntry {
                id: 2,
                timestamp: Utc::now(),
                session_id: "s1".to_string(),
                entry_type: "user".to_string(),
                content: "I prefer using vim for editing".to_string(),
                metadata: None,
            },
        ];

        let facts = extract_facts_from_messages(&messages);
        // Deduplication should collapse duplicates
        let unique_contents: Vec<_> = facts.iter().map(|f| &f.content).collect();
        let deduped: std::collections::HashSet<_> = unique_contents.iter().collect();
        assert_eq!(
            unique_contents.len(),
            deduped.len(),
            "Facts should be deduplicated"
        );
    }

    // Helper: create a dummy RecallMemory using a temp directory.
    // Uses tempfile::TempDir so the directory persists for the test duration.
    fn dummy_recall() -> RecallMemory {
        let dir = tempfile::tempdir().unwrap();
        RecallMemory::new(&dir.path().join("recall.db")).unwrap()
    }

    // Helper: create a dummy EpisodicMemory using a temp directory.
    fn dummy_episodic() -> EpisodicMemory {
        let dir = tempfile::tempdir().unwrap();
        EpisodicMemory::new(dir.path().join("episodic.db"))
    }
}
