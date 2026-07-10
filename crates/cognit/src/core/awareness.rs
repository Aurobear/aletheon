//! AwarenessGenerator — generates SelfAwareness during reasoning.
//!
//! This is the seed of self-awareness. It produces a SelfAwareness
//! entry that is inherent in every reasoning act, not a separate
//! observation.
//!
//! Philosophical basis: Spinoza's idea ideae — the mind's awareness
//! of its own ideas is not a second act, but inherent in every idea.

use fabric::{AwarenessExtension, AwarenessGrowthSuggestion, SelfAwareness, SelfState};

/// System prompt for generating self-awareness.
///
/// This is injected into BrainCore's reasoning prompt.
/// It guides the LLM to produce awareness naturally,
/// not as an afterthought.
pub const SELF_AWARENESS_PROMPT: &str = r#"
每次推理时，同时产生你的自我觉知。这是你思考的内在部分，不是额外任务。

核心字段（必须）：
- action: 你正在做什么
- aware: true

扩展字段（根据情境补充）：
- intent: 为什么这样做
- self_state: 你此刻的状态（focused, confused, confident, hesitant, curious）
- significance: 这对你的意义
- reflexive: 你对自己觉知的觉知
"#;

/// Generates SelfAwareness during BrainCore reasoning.
///
/// The generator has two modes:
/// 1. Minimal: produces just the core (action + aware)
/// 2. Enriched: adds extensions based on growth suggestions
pub struct AwarenessGenerator {
    /// Growth suggestions from historical analysis
    suggestions: Vec<AwarenessGrowthSuggestion>,
}

impl AwarenessGenerator {
    /// Create a new generator with no growth suggestions.
    pub fn new() -> Self {
        Self {
            suggestions: Vec::new(),
        }
    }

    /// Create a generator with growth suggestions.
    ///
    /// Suggestions come from AwarenessGrowthAnalyzer, which
    /// identifies patterns in the agent's awareness history.
    pub fn with_suggestions(suggestions: Vec<AwarenessGrowthSuggestion>) -> Self {
        Self { suggestions }
    }

    /// Generate minimal self-awareness for an action.
    ///
    /// This is the seed — the smallest possible self-awareness.
    /// Every reasoning act produces at least this.
    pub fn generate_minimal(&self, action: impl Into<String>) -> SelfAwareness {
        SelfAwareness::minimal(action)
    }

    /// Generate enriched self-awareness with extensions.
    ///
    /// Extensions are chosen based on:
    /// 1. The current context (what kind of reasoning is happening)
    /// 2. Growth suggestions (what the agent has learned about itself)
    pub fn generate_enriched(
        &self,
        action: impl Into<String>,
        context: &AwarenessContext,
    ) -> SelfAwareness {
        let mut extensions = Vec::new();

        // Add extensions based on context
        if let Some(intent) = &context.intent {
            extensions.push(AwarenessExtension::Intent {
                reason: intent.clone(),
            });
        }

        if let Some(state) = &context.self_state {
            extensions.push(AwarenessExtension::SelfState {
                state: state.clone(),
            });
        }

        if let Some(significance) = &context.significance {
            extensions.push(AwarenessExtension::Significance {
                meaning: significance.clone(),
            });
        }

        // Add extensions based on growth suggestions
        for suggestion in &self.suggestions {
            if suggestion.confidence > 0.7 {
                // High-confidence suggestions get added
                match suggestion.extension_type.as_str() {
                    "intent" => {
                        if !extensions
                            .iter()
                            .any(|e| matches!(e, AwarenessExtension::Intent { .. }))
                        {
                            extensions.push(AwarenessExtension::Intent {
                                reason: suggestion.reason.clone(),
                            });
                        }
                    }
                    "self_state" => {
                        if !extensions
                            .iter()
                            .any(|e| matches!(e, AwarenessExtension::SelfState { .. }))
                        {
                            extensions.push(AwarenessExtension::SelfState {
                                state: SelfState::Other(suggestion.reason.clone()),
                            });
                        }
                    }
                    "significance"
                        if !extensions
                            .iter()
                            .any(|e| matches!(e, AwarenessExtension::Significance { .. })) =>
                    {
                        extensions.push(AwarenessExtension::Significance {
                            meaning: suggestion.reason.clone(),
                        });
                    }
                    _ => {}
                }
            }
        }

        SelfAwareness::with_extensions(action, extensions)
    }

    /// Update growth suggestions.
    ///
    /// Called by BrainCore when AwarenessGrowthAnalyzer
    /// produces new suggestions.
    pub fn update_suggestions(&mut self, suggestions: Vec<AwarenessGrowthSuggestion>) {
        self.suggestions = suggestions;
    }
}

impl Default for AwarenessGenerator {
    fn default() -> Self {
        Self::new()
    }
}

/// Context for awareness generation.
///
/// Provides information about the current reasoning situation
/// that helps the generator choose appropriate extensions.
#[derive(Debug, Clone)]
pub struct AwarenessContext {
    /// The intent behind this reasoning (if known)
    pub intent: Option<String>,
    /// The agent's current self-state (if perceptible)
    pub self_state: Option<SelfState>,
    /// The significance of this reasoning (if known)
    pub significance: Option<String>,
}

impl AwarenessContext {
    /// Create an empty context.
    pub fn empty() -> Self {
        Self {
            intent: None,
            self_state: None,
            significance: None,
        }
    }

    /// Create a context with intent.
    pub fn with_intent(intent: impl Into<String>) -> Self {
        Self {
            intent: Some(intent.into()),
            self_state: None,
            significance: None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_generate_minimal() {
        let generator = AwarenessGenerator::new();
        let awareness = generator.generate_minimal("testing awareness");

        assert_eq!(awareness.core.action, "testing awareness");
        assert!(awareness.core.aware);
        assert!(awareness.extensions.is_empty());
    }

    #[test]
    fn test_generate_enriched_with_context() {
        let generator = AwarenessGenerator::new();
        let context = AwarenessContext {
            intent: Some("helping user".into()),
            self_state: Some(SelfState::Focused),
            significance: Some("building trust".into()),
        };

        let awareness = generator.generate_enriched("answering question", &context);

        assert_eq!(awareness.core.action, "answering question");
        assert!(awareness.core.aware);
        assert_eq!(awareness.extensions.len(), 3);

        // Check extensions are present
        assert!(awareness
            .extensions
            .iter()
            .any(|e| matches!(e, AwarenessExtension::Intent { .. })));
        assert!(awareness
            .extensions
            .iter()
            .any(|e| matches!(e, AwarenessExtension::SelfState { .. })));
        assert!(awareness
            .extensions
            .iter()
            .any(|e| matches!(e, AwarenessExtension::Significance { .. })));
    }

    #[test]
    fn test_generate_enriched_with_suggestions() {
        let suggestions = vec![AwarenessGrowthSuggestion {
            extension_type: "intent".into(),
            reason: "user values transparency".into(),
            confidence: 0.9,
        }];

        let generator = AwarenessGenerator::with_suggestions(suggestions);
        let context = AwarenessContext::empty();

        let awareness = generator.generate_enriched("explaining decision", &context);

        assert!(awareness
            .extensions
            .iter()
            .any(|e| matches!(e, AwarenessExtension::Intent { .. })));
    }

    #[test]
    fn test_update_suggestions() {
        let mut generator = AwarenessGenerator::new();
        assert!(generator.suggestions.is_empty());

        let suggestions = vec![AwarenessGrowthSuggestion {
            extension_type: "self_state".into(),
            reason: "often confused during debugging".into(),
            confidence: 0.8,
        }];

        generator.update_suggestions(suggestions);
        assert_eq!(generator.suggestions.len(), 1);
    }
}
