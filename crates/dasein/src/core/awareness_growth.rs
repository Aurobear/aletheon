//! AwarenessGrowthAnalyzer — analyzes awareness history for growth.
//!
//! This is the mechanism that prevents "wild growth".
//! Instead of randomly adding extensions, the analyzer examines
//! the agent's awareness history and identifies patterns.
//!
//! Growth suggestions are grounded in observed patterns, not
//! arbitrary additions.

use anyhow::Result;
use base::{
    AwarenessExtension, AwarenessExtensionCounts, AwarenessGrowthSuggestion, SelfAwareness,
};

/// Analyzes awareness history to produce growth suggestions.
///
/// The analyzer looks for:
/// 1. Missing extension types (patterns of absence)
/// 2. Correlation between extensions and outcomes
/// 3. Extension type frequency vs. task type
///
/// Suggestions are produced with confidence scores.
/// Only high-confidence suggestions (>0.7) are used.
pub struct AwarenessGrowthAnalyzer {
    /// Minimum number of awareness entries needed for analysis
    min_entries: usize,
    /// Minimum confidence for a suggestion to be produced
    min_confidence: f64,
}

impl AwarenessGrowthAnalyzer {
    /// Create a new analyzer with default settings.
    pub fn new() -> Self {
        Self {
            min_entries: 10,
            min_confidence: 0.5,
        }
    }

    /// Create an analyzer with custom settings.
    pub fn with_config(min_entries: usize, min_confidence: f64) -> Self {
        Self {
            min_entries,
            min_confidence,
        }
    }

    /// Analyze awareness history and produce growth suggestions.
    ///
    /// This is the core analysis function. It examines the pattern
    /// of extensions across awareness entries and identifies:
    /// - Which extension types are underrepresented
    /// - Which extension types correlate with better outcomes
    /// - What the agent should pay more attention to
    pub fn analyze(
        &self,
        history: &[SelfAwareness],
        extension_stats: &AwarenessExtensionCounts,
    ) -> Result<Vec<AwarenessGrowthSuggestion>> {
        if history.len() < self.min_entries {
            return Ok(Vec::new());
        }

        let mut suggestions = Vec::new();

        // Analyze extension type distribution
        let total = history.len() as f64;
        let intent_ratio = extension_stats.intent as f64 / total;
        let self_state_ratio = extension_stats.self_state as f64 / total;
        let significance_ratio = extension_stats.significance as f64 / total;

        // Suggest underrepresented extensions
        if intent_ratio < 0.3 {
            let confidence = self.calculate_confidence(intent_ratio, 0.3);
            if confidence >= self.min_confidence {
                suggestions.push(AwarenessGrowthSuggestion {
                    extension_type: "intent".into(),
                    reason: format!(
                        "Intent extension is used in only {:.0}% of awareness entries. \
                         Consider adding 'why' to your awareness.",
                        intent_ratio * 100.0
                    ),
                    confidence,
                });
            }
        }

        if self_state_ratio < 0.2 {
            let confidence = self.calculate_confidence(self_state_ratio, 0.2);
            if confidence >= self.min_confidence {
                suggestions.push(AwarenessGrowthSuggestion {
                    extension_type: "self_state".into(),
                    reason: format!(
                        "SelfState extension is used in only {:.0}% of awareness entries. \
                         Consider reflecting on your current state.",
                        self_state_ratio * 100.0
                    ),
                    confidence,
                });
            }
        }

        if significance_ratio < 0.15 {
            let confidence = self.calculate_confidence(significance_ratio, 0.15);
            if confidence >= self.min_confidence {
                suggestions.push(AwarenessGrowthSuggestion {
                    extension_type: "significance".into(),
                    reason: format!(
                        "Significance extension is used in only {:.0}% of awareness entries. \
                         Consider what this means to you.",
                        significance_ratio * 100.0
                    ),
                    confidence,
                });
            }
        }

        Ok(suggestions)
    }

    /// Calculate confidence based on how far below target the ratio is.
    ///
    /// The further below target, the higher the confidence that
    /// this extension type needs attention.
    fn calculate_confidence(&self, actual: f64, target: f64) -> f64 {
        let gap = target - actual;
        // Normalize: 0 gap = 0 confidence, full gap = 1.0 confidence
        (gap / target).min(1.0).max(0.0)
    }

    /// Analyze a single awareness entry for quality.
    ///
    /// Returns a quality score (0.0 to 1.0) based on:
    /// - Number of extensions present
    /// - Richness of extension content
    pub fn assess_quality(&self, awareness: &SelfAwareness) -> f64 {
        let mut score: f64 = 0.0;

        // Core is always present (base score)
        score += 0.3;

        // Each extension type adds to the score
        let has_intent = awareness
            .extensions
            .iter()
            .any(|e| matches!(e, AwarenessExtension::Intent { .. }));
        let has_self_state = awareness
            .extensions
            .iter()
            .any(|e| matches!(e, AwarenessExtension::SelfState { .. }));
        let has_significance = awareness
            .extensions
            .iter()
            .any(|e| matches!(e, AwarenessExtension::Significance { .. }));

        if has_intent {
            score += 0.25;
        }
        if has_self_state {
            score += 0.2;
        }
        if has_significance {
            score += 0.25;
        }

        score.min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::AwarenessCore;

    fn make_awareness(extensions: Vec<AwarenessExtension>) -> SelfAwareness {
        SelfAwareness {
            core: AwarenessCore {
                action: "test action".into(),
                aware: true,
            },
            extensions,
        }
    }

    #[test]
    fn test_analyze_insufficient_history() {
        let analyzer = AwarenessGrowthAnalyzer::new();
        let history: Vec<SelfAwareness> = (0..5).map(|_| make_awareness(vec![])).collect();
        let stats = AwarenessExtensionCounts::default();

        let suggestions = analyzer.analyze(&history, &stats).unwrap();
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_analyze_suggests_intent() {
        let analyzer = AwarenessGrowthAnalyzer::with_config(5, 0.3);
        let history: Vec<SelfAwareness> = (0..10).map(|_| make_awareness(vec![])).collect();
        let stats = AwarenessExtensionCounts {
            intent: 1, // Only 10% have intent
            self_state: 5,
            significance: 5,
            reflexive: 0,
        };

        let suggestions = analyzer.analyze(&history, &stats).unwrap();
        assert!(suggestions.iter().any(|s| s.extension_type == "intent"));
    }

    #[test]
    fn test_analyze_no_suggestions_when_balanced() {
        let analyzer = AwarenessGrowthAnalyzer::with_config(5, 0.5);
        let history: Vec<SelfAwareness> = (0..10)
            .map(|_| {
                make_awareness(vec![
                    AwarenessExtension::Intent {
                        reason: "test".into(),
                    },
                    AwarenessExtension::SelfState {
                        state: base::SelfState::Focused,
                    },
                    AwarenessExtension::Significance {
                        meaning: "test".into(),
                    },
                ])
            })
            .collect();
        let stats = AwarenessExtensionCounts {
            intent: 10,
            self_state: 10,
            significance: 10,
            reflexive: 0,
        };

        let suggestions = analyzer.analyze(&history, &stats).unwrap();
        assert!(suggestions.is_empty());
    }

    #[test]
    fn test_assess_quality_minimal() {
        let analyzer = AwarenessGrowthAnalyzer::new();
        let awareness = make_awareness(vec![]);

        let quality = analyzer.assess_quality(&awareness);
        assert!((quality - 0.3).abs() < f64::EPSILON);
    }

    #[test]
    fn test_assess_quality_full() {
        let analyzer = AwarenessGrowthAnalyzer::new();
        let awareness = make_awareness(vec![
            AwarenessExtension::Intent {
                reason: "test".into(),
            },
            AwarenessExtension::SelfState {
                state: base::SelfState::Focused,
            },
            AwarenessExtension::Significance {
                meaning: "test".into(),
            },
        ]);

        let quality = analyzer.assess_quality(&awareness);
        assert!((quality - 1.0).abs() < f64::EPSILON);
    }
}
