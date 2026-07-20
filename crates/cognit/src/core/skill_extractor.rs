//! SkillExtractor — analyzes reflections to identify reusable patterns
//! and persist them as markdown skill files.
//!
//! Extraction rules:
//! - 3+ reflections with similar `what_worked` → success pattern
//! - 3+ reflections with similar `what_failed` + `learned` → avoidance guide
//! - High confidence (>0.8) reflection with specific lessons → best practice

use std::collections::HashMap;
use std::path::{Path, PathBuf};

use anyhow::{Context, Result};
use fabric::Clock;
use std::sync::Arc;

use fabric::cognit::ReflectionEntry;

/// Category of an extracted skill.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum SkillCategory {
    /// Repeated success across multiple reflections.
    SuccessPattern,
    /// Lessons learned from repeated failures.
    AvoidanceGuide,
    /// A single high-confidence insight worth codifying.
    BestPractice,
}

impl std::fmt::Display for SkillCategory {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::SuccessPattern => write!(f, "success_pattern"),
            Self::AvoidanceGuide => write!(f, "avoidance_guide"),
            Self::BestPractice => write!(f, "best_practice"),
        }
    }
}

/// A skill extracted from a set of reflections.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct ExtractedSkill {
    /// Human-readable skill name.
    pub name: String,
    /// Short description of the skill.
    pub description: String,
    /// Category of the skill.
    pub category: SkillCategory,
    /// Markdown content describing the skill in detail.
    pub content: String,
    /// IDs of the source reflections that contributed to this skill.
    pub source_reflections: Vec<String>,
}

/// Extracts reusable skills from batches of reflection entries.
pub struct SkillExtractor {
    /// Minimum number of reflections sharing a pattern to qualify.
    min_occurrences: usize,
    /// Confidence threshold for single-entry best-practice extraction.
    best_practice_threshold: f64,
    clock: Arc<dyn Clock>,
}

impl SkillExtractor {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            min_occurrences: 3,
            best_practice_threshold: 0.8,
            clock,
        }
    }

    /// Analyze reflections and extract reusable skills.
    pub fn extract_skills(&self, reflections: &[ReflectionEntry]) -> Vec<ExtractedSkill> {
        if reflections.is_empty() {
            return Vec::new();
        }

        let mut skills = Vec::new();

        skills.extend(self.extract_success_patterns(reflections));
        skills.extend(self.extract_avoidance_guides(reflections));
        skills.extend(self.extract_best_practices(reflections));

        skills
    }

    /// Save a skill as a markdown file in the given skills directory.
    pub fn save_skill(&self, skill: &ExtractedSkill, skills_dir: &Path) -> Result<PathBuf> {
        std::fs::create_dir_all(skills_dir).with_context(|| {
            format!(
                "Failed to create skills directory: {}",
                skills_dir.display()
            )
        })?;

        let slug = slugify(&skill.name);
        let filename = format!("{}.md", slug);
        let path = skills_dir.join(&filename);

        let markdown = self.render_markdown(skill);
        std::fs::write(&path, &markdown)
            .with_context(|| format!("Failed to write skill file: {}", path.display()))?;

        Ok(path)
    }

    // --- Internal extraction rules ---

    /// Rule 1: 3+ reflections with similar `what_worked` items → success pattern.
    fn extract_success_patterns(&self, reflections: &[ReflectionEntry]) -> Vec<ExtractedSkill> {
        let mut worked_counts: HashMap<String, (usize, Vec<String>)> = HashMap::new();

        for r in reflections {
            for item in &r.what_worked {
                let key = normalize_text(item);
                let entry = worked_counts.entry(key).or_insert_with(|| (0, Vec::new()));
                entry.0 += 1;
                if !entry.1.contains(&r.id) {
                    entry.1.push(r.id.clone());
                }
            }
        }

        worked_counts
            .into_iter()
            .filter(|(_, (count, _))| *count >= self.min_occurrences)
            .map(|(_, (count, source_ids))| {
                let item_name = format!("Success pattern ({} occurrences)", count);
                ExtractedSkill {
                    name: item_name.clone(),
                    description: format!(
                        "Pattern observed across {} reflections where the same approach succeeded.",
                        count
                    ),
                    category: SkillCategory::SuccessPattern,
                    content: format!(
                        "## Pattern\n\nThis pattern was consistently successful across {} reflections.\n\n\
                         ## Recommendation\n\nContinue applying this approach. It has proven reliable.\n\n\
                         ## Source Reflections\n\n{}",
                        count,
                        source_ids
                            .iter()
                            .map(|id| format!("- {}", id))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    source_reflections: source_ids,
                }
            })
            .collect()
    }

    /// Rule 2: 3+ reflections with similar `what_failed` + `learned` → avoidance guide.
    fn extract_avoidance_guides(&self, reflections: &[ReflectionEntry]) -> Vec<ExtractedSkill> {
        let mut failed_learned_counts: HashMap<String, (usize, Vec<String>)> = HashMap::new();

        for r in reflections {
            // Combine what_failed and learned into a single key for matching
            let mut items: Vec<String> = r.what_failed.to_vec();
            items.extend(r.learned.iter().cloned());

            for item in &items {
                let key = normalize_text(item);
                let entry = failed_learned_counts
                    .entry(key)
                    .or_insert_with(|| (0, Vec::new()));
                entry.0 += 1;
                if !entry.1.contains(&r.id) {
                    entry.1.push(r.id.clone());
                }
            }
        }

        failed_learned_counts
            .into_iter()
            .filter(|(_, (count, _))| *count >= self.min_occurrences)
            .map(|(_, (count, source_ids))| {
                let item_name = format!("Avoidance guide ({} occurrences)", count);
                ExtractedSkill {
                    name: item_name.clone(),
                    description: format!(
                        "Recurring failure pattern across {} reflections with a derived lesson.",
                        count
                    ),
                    category: SkillCategory::AvoidanceGuide,
                    content: format!(
                        "## Anti-Pattern\n\nThis pattern has repeatedly led to failures across {} reflections.\n\n\
                         ## Avoid\n\nBe cautious when encountering similar situations. Review the lessons learned \
                         from these reflections before proceeding.\n\n\
                         ## Source Reflections\n\n{}",
                        count,
                        source_ids
                            .iter()
                            .map(|id| format!("- {}", id))
                            .collect::<Vec<_>>()
                            .join("\n")
                    ),
                    source_reflections: source_ids,
                }
            })
            .collect()
    }

    /// Rule 3: Single reflection with high confidence and specific lessons → best practice.
    fn extract_best_practices(&self, reflections: &[ReflectionEntry]) -> Vec<ExtractedSkill> {
        reflections
            .iter()
            .filter(|r| r.confidence > self.best_practice_threshold && !r.learned.is_empty())
            .map(|r| {
                let lessons = r
                    .learned
                    .iter()
                    .map(|l| format!("- {}", l))
                    .collect::<Vec<_>>()
                    .join("\n");

                let item_name = format!("Best practice: {}", truncate(&r.task_summary, 60));
                ExtractedSkill {
                    name: item_name.clone(),
                    description: format!(
                        "High-confidence insight (confidence={:.2}) from task: {}",
                        r.confidence, r.task_summary
                    ),
                    category: SkillCategory::BestPractice,
                    content: format!(
                        "## Context\n\nTask: {}\nConfidence: {:.2}\n\n\
                         ## Lessons\n\n{}\n\n\
                         ## Source Reflection\n\n- {}",
                        r.task_summary, r.confidence, lessons, r.id
                    ),
                    source_reflections: vec![r.id.clone()],
                }
            })
            .collect()
    }

    /// Render an ExtractedSkill as a full markdown document.
    fn render_markdown(&self, skill: &ExtractedSkill) -> String {
        format!(
            "# {}\n\n\
             **Category:** {}  \n\
             **Generated:** {}  \n\
             **Source reflections:** {}  \n\n\
             {}\n\n\
             ---\n\n\
             {}\n",
            skill.name,
            skill.category,
            fabric::wall_to_datetime(self.clock.wall_now()).format("%Y-%m-%d %H:%M:%S UTC"),
            skill.source_reflections.len(),
            skill.description,
            skill.content,
        )
    }
}

#[cfg(test)]
impl Default for SkillExtractor {
    fn default() -> Self {
        Self::new(Arc::new(kernel::chronos::TestClock::default()))
    }
}

/// Normalize text for fuzzy matching (lowercase, collapse whitespace).
fn normalize_text(text: &str) -> String {
    text.to_lowercase()
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
}

/// Create a filesystem-safe slug from a name.
fn slugify(name: &str) -> String {
    name.to_lowercase()
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' || c == '_' {
                c
            } else {
                '-'
            }
        })
        .collect::<String>()
        .split('-')
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("-")
}

/// Truncate a string to a maximum length, appending "..." if truncated.
fn truncate(s: &str, max_len: usize) -> String {
    if s.len() <= max_len {
        s.to_string()
    } else {
        format!("{}...", &s[..max_len])
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::cognit::{ReflectionEntry, ReflectionOutcome, ReflectionTrigger};
    use std::sync::Arc;

    fn make_extractor() -> SkillExtractor {
        SkillExtractor::new(Arc::new(kernel::chronos::TestClock::default()))
    }

    fn make_reflection(
        id: &str,
        what_worked: Vec<&str>,
        what_failed: Vec<&str>,
        learned: Vec<&str>,
        confidence: f64,
    ) -> ReflectionEntry {
        ReflectionEntry {
            id: id.to_string(),
            timestamp: fabric::wall_to_datetime(kernel::chronos::TestClock::default().wall_now()),
            trigger: ReflectionTrigger::TaskComplete,
            task_summary: format!("task {}", id),
            outcome: ReflectionOutcome::Success,
            what_worked: what_worked.into_iter().map(String::from).collect(),
            what_failed: what_failed.into_iter().map(String::from).collect(),
            learned: learned.into_iter().map(String::from).collect(),
            behavior_changes: vec![],
            confidence,
        }
    }

    #[test]
    fn extract_success_pattern_from_3_similar_what_worked() {
        let extractor = make_extractor();
        let reflections = vec![
            make_reflection("r1", vec!["use batch processing"], vec![], vec![], 0.7),
            make_reflection("r2", vec!["use batch processing"], vec![], vec![], 0.8),
            make_reflection("r3", vec!["use batch processing"], vec![], vec![], 0.9),
        ];

        let skills = extractor.extract_skills(&reflections);
        let success = skills
            .iter()
            .filter(|s| s.category == SkillCategory::SuccessPattern)
            .collect::<Vec<_>>();

        assert_eq!(success.len(), 1);
        assert!(success[0].name.contains("Success pattern"));
        assert_eq!(success[0].source_reflections.len(), 3);
    }

    #[test]
    fn no_success_pattern_with_only_2_occurrences() {
        let extractor = make_extractor();
        let reflections = vec![
            make_reflection("r1", vec!["use caching"], vec![], vec![], 0.7),
            make_reflection("r2", vec!["use caching"], vec![], vec![], 0.8),
        ];

        let skills = extractor.extract_skills(&reflections);
        let success = skills
            .iter()
            .filter(|s| s.category == SkillCategory::SuccessPattern)
            .collect::<Vec<_>>();

        assert_eq!(success.len(), 0);
    }

    #[test]
    fn extract_avoidance_guide_from_3_similar_failures() {
        let extractor = make_extractor();
        let reflections = vec![
            make_reflection(
                "r1",
                vec![],
                vec!["skip validation"],
                vec!["always validate inputs"],
                0.3,
            ),
            make_reflection(
                "r2",
                vec![],
                vec!["skip validation"],
                vec!["always validate inputs"],
                0.2,
            ),
            make_reflection(
                "r3",
                vec![],
                vec!["skip validation"],
                vec!["always validate inputs"],
                0.4,
            ),
        ];

        let skills = extractor.extract_skills(&reflections);
        let avoidance = skills
            .iter()
            .filter(|s| s.category == SkillCategory::AvoidanceGuide)
            .collect::<Vec<_>>();

        // "skip validation" appears 3x, "always validate inputs" appears 3x
        assert!(!avoidance.is_empty());
        for skill in &avoidance {
            assert!(skill.name.contains("Avoidance guide"));
        }
    }

    #[test]
    fn extract_best_practice_from_high_confidence_reflection() {
        let extractor = make_extractor();
        let reflections = vec![make_reflection(
            "r1",
            vec![],
            vec![],
            vec!["always use structured logging", "prefer async I/O"],
            0.9,
        )];

        let skills = extractor.extract_skills(&reflections);
        let best = skills
            .iter()
            .filter(|s| s.category == SkillCategory::BestPractice)
            .collect::<Vec<_>>();

        assert_eq!(best.len(), 1);
        assert!(best[0].name.contains("Best practice"));
        assert!(best[0].content.contains("structured logging"));
        assert!(best[0].content.contains("async I/O"));
    }

    #[test]
    fn no_best_practice_below_threshold() {
        let extractor = make_extractor();
        let reflections = vec![make_reflection(
            "r1",
            vec![],
            vec![],
            vec!["some lesson"],
            0.5,
        )];

        let skills = extractor.extract_skills(&reflections);
        let best = skills
            .iter()
            .filter(|s| s.category == SkillCategory::BestPractice)
            .collect::<Vec<_>>();

        assert_eq!(best.len(), 0);
    }

    #[test]
    fn no_best_practice_without_learned() {
        let extractor = make_extractor();
        let reflections = vec![make_reflection("r1", vec![], vec![], vec![], 0.95)];

        let skills = extractor.extract_skills(&reflections);
        let best = skills
            .iter()
            .filter(|s| s.category == SkillCategory::BestPractice)
            .collect::<Vec<_>>();

        assert_eq!(best.len(), 0);
    }

    #[test]
    fn extract_empty_input_returns_empty() {
        let extractor = make_extractor();
        let skills = extractor.extract_skills(&[]);
        assert!(skills.is_empty());
    }

    #[test]
    fn save_skill_writes_markdown_file() {
        let extractor = make_extractor();
        let skill = ExtractedSkill {
            name: "Test Skill".to_string(),
            description: "A test skill.".to_string(),
            category: SkillCategory::BestPractice,
            content: "## Details\n\nSome content here.".to_string(),
            source_reflections: vec!["r1".to_string()],
        };

        let tmp_dir = std::env::temp_dir().join("aletheon-skill-test");
        let _ = std::fs::remove_dir_all(&tmp_dir);

        let path = extractor.save_skill(&skill, &tmp_dir).unwrap();
        assert!(path.exists());
        assert!(path.to_string_lossy().ends_with(".md"));

        let content = std::fs::read_to_string(&path).unwrap();
        assert!(content.contains("# Test Skill"));
        assert!(content.contains("best_practice"));
        assert!(content.contains("Some content here."));

        // Cleanup
        let _ = std::fs::remove_dir_all(&tmp_dir);
    }

    #[test]
    fn slugify_basic() {
        assert_eq!(slugify("Hello World"), "hello-world");
        assert_eq!(slugify("success_pattern"), "success_pattern");
        assert_eq!(slugify("  spaces  "), "spaces");
    }

    #[test]
    fn normalize_text_collapses_whitespace() {
        assert_eq!(
            normalize_text("  Use   Batch  Processing  "),
            "use batch processing"
        );
    }

    #[test]
    fn truncate_short_string_unchanged() {
        assert_eq!(truncate("hello", 10), "hello");
    }

    #[test]
    fn truncate_long_string_is_cut() {
        let result = truncate("a long string that goes on and on", 10);
        assert_eq!(result, "a long str...");
        assert!(result.len() <= 13); // 10 + 3 for "..."
    }

    #[test]
    fn multiple_categories_extracted_simultaneously() {
        let extractor = make_extractor();
        let reflections = vec![
            make_reflection("r1", vec!["use retry"], vec![], vec![], 0.7),
            make_reflection("r2", vec!["use retry"], vec![], vec![], 0.8),
            make_reflection(
                "r3",
                vec!["use retry"],
                vec!["ignore errors"],
                vec!["always handle errors"],
                0.9,
            ),
            make_reflection(
                "r4",
                vec![],
                vec!["ignore errors"],
                vec!["always handle errors"],
                0.3,
            ),
            make_reflection(
                "r5",
                vec![],
                vec!["ignore errors"],
                vec!["always handle errors"],
                0.2,
            ),
        ];

        let skills = extractor.extract_skills(&reflections);
        let categories: Vec<SkillCategory> = skills.iter().map(|s| s.category.clone()).collect();

        // Should have at least one success pattern ("use retry" x3)
        assert!(categories.contains(&SkillCategory::SuccessPattern));
        // Should have at least one avoidance guide ("ignore errors" x3 or "always handle errors" x3)
        assert!(categories.contains(&SkillCategory::AvoidanceGuide));
        // Should have best practice (r3 has confidence 0.9 and non-empty learned)
        assert!(categories.contains(&SkillCategory::BestPractice));
    }
}
