//! ExperienceSummarizer — analyzes accumulated reflections and produces evolution log entries.
//!
//! Detects behavioral patterns (repeated topics, repeated failures, success strategies)
//! and generates behavior adjustment suggestions.

use fabric::cognit::{BehaviorAdjustment, EvolutionLogEntry, ReflectionEntry, ReflectionOutcome};
use fabric::Clock;
use std::sync::Arc;

/// ExperienceSummarizer — analyzes accumulated reflections and produces evolution log entries.
///
/// Detects behavioral patterns (repeated topics, repeated failures, success strategies)
/// and generates behavior adjustment suggestions.
pub struct ExperienceSummarizer {
    clock: Arc<dyn Clock>,
}

impl ExperienceSummarizer {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self { clock }
    }

    /// Analyze a batch of reflections and produce an EvolutionLogEntry.
    ///
    /// Returns `None` if no patterns are detected (fewer than 2 reflections
    /// and no significant signal).
    pub fn summarize(&self, reflections: &[ReflectionEntry]) -> Option<EvolutionLogEntry> {
        if reflections.is_empty() {
            return None;
        }

        let mut patterns = Vec::new();
        let mut adjustments = Vec::new();
        let basis: Vec<String> = reflections.iter().map(|r| r.id.clone()).collect();

        // --- Pattern 1: Repeated topics ---
        let mut topic_counts: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for r in reflections {
            let topic = Self::extract_topic(&r.task_summary);
            *topic_counts.entry(topic).or_insert(0) += 1;
        }
        for (topic, count) in &topic_counts {
            if *count >= 3 {
                patterns.push(format!("Repeated topic '{topic}' appeared {count} times"));
            }
        }

        // --- Pattern 2: Repeated failures ---
        let failures: Vec<&ReflectionEntry> = reflections
            .iter()
            .filter(|r| r.outcome == ReflectionOutcome::Failure)
            .collect();
        let failure_ratio = failures.len() as f64 / reflections.len() as f64;
        if failure_ratio > 0.5 && failures.len() >= 2 {
            patterns.push(format!(
                "High failure rate: {}/{} reflections are failures ({:.0}%)",
                failures.len(),
                reflections.len(),
                failure_ratio * 100.0
            ));

            // Suggest increasing safety care weight
            adjustments.push(BehaviorAdjustment {
                target: "care.safety.weight".to_string(),
                old_value: None,
                new_value: Some(1.0),
                reason: format!(
                    "High failure rate ({:.0}%) suggests cautious approach",
                    failure_ratio * 100.0
                ),
            });
        }

        // --- Pattern 3: Success strategies (high-confidence successes with common learned items) ---
        let successes: Vec<&ReflectionEntry> = reflections
            .iter()
            .filter(|r| r.outcome == ReflectionOutcome::Success && r.confidence > 0.7)
            .collect();
        if successes.len() >= 2 {
            patterns.push(format!(
                "Consistent success pattern: {} high-confidence successes",
                successes.len()
            ));

            // Collect common learnings
            let mut learning_counts: std::collections::HashMap<String, usize> =
                std::collections::HashMap::new();
            for s in &successes {
                for lesson in &s.learned {
                    *learning_counts.entry(lesson.clone()).or_insert(0) += 1;
                }
            }
            for (lesson, count) in &learning_counts {
                if *count >= 2 {
                    patterns.push(format!(
                        "Recurring lesson: '{lesson}' (mentioned {count} times)"
                    ));
                }
            }

            // Suggest increasing learning weight
            adjustments.push(BehaviorAdjustment {
                target: "care.learning.weight".to_string(),
                old_value: None,
                new_value: Some(0.5),
                reason: "Consistent successes suggest learning is effective".to_string(),
            });
        }

        // --- Pattern 4: Low confidence trend ---
        let avg_confidence: f64 =
            reflections.iter().map(|r| r.confidence).sum::<f64>() / reflections.len() as f64;
        if avg_confidence < 0.4 && reflections.len() >= 3 {
            patterns.push(format!(
                "Low average confidence: {:.2} across {} reflections",
                avg_confidence,
                reflections.len()
            ));

            adjustments.push(BehaviorAdjustment {
                target: "care.efficiency.weight".to_string(),
                old_value: None,
                new_value: Some(0.3),
                reason: "Low confidence suggests need for more careful, less efficient approach"
                    .to_string(),
            });
        }

        if patterns.is_empty() && reflections.len() < 2 {
            return None;
        }

        Some(EvolutionLogEntry {
            id: format!("evo-{}", uuid::Uuid::new_v4()),
            timestamp: fabric::wall_to_datetime(self.clock.wall_now()),
            trigger: "periodic_review".to_string(),
            basis,
            patterns_detected: patterns,
            adjustments,
        })
    }

    /// Extract a coarse topic from a task summary (first 3 words or first noun phrase).
    fn extract_topic(summary: &str) -> String {
        let words: Vec<&str> = summary.split_whitespace().take(3).collect();
        words.join(" ").to_lowercase()
    }
}
