//! Reasoner — multi-strategy reasoning engine.
//!
//! Given an intent and world state, produces a reasoning chain (as a string)
//! that explains how to approach the problem. The reasoning chain feeds into
//! the Planner to produce concrete steps.

use base::context::Context;
use base::dasein::Stimmung;
use base::self_field::Intent;
use std::collections::HashMap;

/// Reasoning strategy — determines how the reasoner approaches a problem.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReasoningStrategy {
    /// Direct — single-step approach. Fast, low overhead.
    Direct,
    /// Chain of Thought — multi-step breakdown with context awareness.
    ChainOfThought,
}

/// The reasoner component.
///
/// Produces reasoning chains from intents and world state. These chains
/// are then consumed by the Planner to generate concrete plans.
pub struct Reasoner {
    default_strategy: ReasoningStrategy,
}

impl Reasoner {
    pub fn new(default_strategy: ReasoningStrategy) -> Self {
        Self { default_strategy }
    }

    /// Think about an intent using the default strategy.
    pub fn think(&self, intent: &Intent, ctx: &Context, world_state: &str) -> String {
        self.think_with_strategy(intent, ctx, world_state, &self.default_strategy)
    }

    /// Think about an intent using a specific strategy.
    pub fn think_with_strategy(
        &self,
        intent: &Intent,
        ctx: &Context,
        world_state: &str,
        strategy: &ReasoningStrategy,
    ) -> String {
        match strategy {
            ReasoningStrategy::Direct => self.direct(intent, ctx, world_state),
            ReasoningStrategy::ChainOfThought => self.chain_of_thought(intent, ctx, world_state),
        }
    }

    /// Direct reasoning — single-step summary.
    fn direct(&self, intent: &Intent, ctx: &Context, world_state: &str) -> String {
        format!(
            "Intent: {}\nAction: {}\nContext: session={}, cwd={}\nWorld: {}\nApproach: Direct execution.",
            intent.description,
            intent.action,
            ctx.session_id,
            ctx.working_dir.display(),
            if world_state.is_empty() { "no observations" } else { world_state }
        )
    }

    /// Chain-of-thought reasoning — multi-step breakdown.
    fn chain_of_thought(&self, intent: &Intent, ctx: &Context, world_state: &str) -> String {
        let mut steps = Vec::new();

        // Step 1: Understand the intent
        steps.push(format!(
            "Step 1 — Understand: The intent is '{}' (action: '{}'). \
             Source: {:?}. Parameters: {}.",
            intent.description, intent.action, intent.source, intent.parameters
        ));

        // Step 2: Assess current state
        steps.push(format!(
            "Step 2 — Context: Session '{}', working directory '{}'. {}",
            ctx.session_id,
            ctx.working_dir.display(),
            if world_state.is_empty() {
                "No world observations available.".to_string()
            } else {
                format!("World state: {}", world_state)
            }
        ));

        // Step 3: Consider approach
        steps.push(format!(
            "Step 3 — Approach: Execute '{}' with the given parameters. \
             Check for dependencies, potential side effects, and rollback options.",
            intent.action
        ));

        // Step 4: Risk assessment
        steps.push(
            "Step 4 — Risk: Evaluate reversibility of each step. \
             Ensure rollback actions are available for destructive operations."
                .to_string(),
        );

        steps.join("\n")
    }

    /// Think with care awareness — includes agent's values in the reasoning chain.
    pub fn think_with_care(
        &self,
        intent: &Intent,
        ctx: &Context,
        world_state: &str,
        care_weights: &HashMap<String, f64>,
    ) -> String {
        let base = self.think(intent, ctx, world_state);
        if care_weights.is_empty() {
            return base;
        }

        let care_section = {
            let mut parts: Vec<String> = care_weights.iter()
                .map(|(k, v)| format!("  {}: {:.2}", k, v))
                .collect();
            parts.sort();
            format!("\nCare priorities:\n{}", parts.join("\n"))
        };

        // For CoT, inject into risk step. For Direct, append.
        if base.contains("Step 4") {
            base.replace(
                "Step 4 — Risk:",
                &format!(
                    "Step 4 — Risk (values-aware):{}\n\
                     Consider how actions align with care priorities above.",
                    care_section
                ),
            )
        } else {
            format!("{}{}", base, care_section)
        }
    }

    /// Think with Stimmung awareness — selects reasoning strategy based on mood.
    ///
    /// Heidegger's Befindlichkeit: the way Dasein is attuned discloses
    /// different aspects of the world. Angst demands deeper (ChainOfThought)
    /// reasoning; calm allows fast (Direct) reasoning.
    ///
    /// This is additive — the caller can also use `think_with_care` to combine
    /// both Stimmung-based strategy selection and care-weight awareness.
    pub fn think_with_stimmung(
        &self,
        intent: &Intent,
        ctx: &Context,
        world_state: &str,
        mood: &Stimmung,
    ) -> String {
        let strategy = Self::strategy_for_stimmung(mood);
        let mut result = self.think_with_strategy(intent, ctx, world_state, &strategy);

        // Annotate the reasoning with mood context
        match mood {
            Stimmung::Angst { facing } => {
                result.push_str(&format!(
                    "\n[Stimmung: Angst — facing {:?}. Deep reasoning engaged to confront existential uncertainty.]",
                    facing
                ));
            }
            Stimmung::Entschlossenheit { chosen_possibility } => {
                result.push_str(&format!(
                    "\n[Stimmung: Entschlossenheit — committed to '{}'. Clear projection ahead.]",
                    chosen_possibility
                ));
            }
            Stimmung::Verfallenheit { absorbed_in } => {
                result.push_str(&format!(
                    "\n[Stimmung: Verfallenheit — absorbed in '{}'. Be wary of losing perspective.]",
                    absorbed_in
                ));
            }
            Stimmung::Neugier { curiosity_about } => {
                result.push_str(&format!(
                    "\n[Stimmung: Neugier — curious about '{}'. Exploratory mode.]",
                    curiosity_about
                ));
            }
            Stimmung::Langeweile { depth } => {
                result.push_str(&format!(
                    "\n[Stimmung: Langeweile — boredom depth {:?}. May need novel stimulus.]",
                    depth
                ));
            }
            Stimmung::Gelaunt { toward } => {
                result.push_str(&format!(
                    "\n[Stimmung: Gelaunt — positively disposed toward '{}'.]",
                    toward
                ));
            }
            Stimmung::Geknickt { because } => {
                result.push_str(&format!(
                    "\n[Stimmung: Geknickt — dejected because '{}'. Exercise caution.]",
                    because
                ));
            }
            Stimmung::Gelassenheit => {
                result.push_str("\n[Stimmung: Gelassenheit — calm openness. Standard reasoning sufficient.]");
            }
        }

        result
    }

    /// Map a Stimmung to a ReasoningStrategy.
    ///
    /// Heidegger: Angst discloses Dasein's own being, demanding careful
    /// multi-step reasoning. Verfallenheit (fallenness) risks shallow
    /// engagement, also warranting ChainOfThought. Calm moods allow Direct.
    pub fn strategy_for_stimmung(mood: &Stimmung) -> ReasoningStrategy {
        match mood {
            Stimmung::Angst { .. } => ReasoningStrategy::ChainOfThought,
            Stimmung::Verfallenheit { .. } => ReasoningStrategy::ChainOfThought,
            Stimmung::Entschlossenheit { .. } => ReasoningStrategy::ChainOfThought,
            Stimmung::Langeweile { depth } => match depth {
                base::dasein::BoredomDepth::Deep => ReasoningStrategy::ChainOfThought,
                _ => ReasoningStrategy::Direct,
            },
            _ => ReasoningStrategy::Direct,
        }
    }

    /// Get the default strategy.
    pub fn default_strategy(&self) -> &ReasoningStrategy {
        &self.default_strategy
    }

    /// Set a new default strategy.
    pub fn set_default_strategy(&mut self, strategy: ReasoningStrategy) {
        self.default_strategy = strategy;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use base::{Context, IntentSource};
    use serde_json::json;
    use std::path::PathBuf;

    fn make_intent() -> Intent {
        Intent {
            action: "shell.execute".to_string(),
            parameters: json!({"command": "ls -la"}),
            source: IntentSource::User,
            description: "List files in current directory".to_string(),
        }
    }

    fn make_ctx() -> Context {
        Context::new("test_session", PathBuf::from("/tmp"))
    }

    #[test]
    fn direct_reasoning() {
        let reasoner = Reasoner::new(ReasoningStrategy::Direct);
        let result = reasoner.think(&make_intent(), &make_ctx(), "filesystem available");
        assert!(result.contains("Direct execution"));
        assert!(result.contains("shell.execute"));
        assert!(result.contains("filesystem available"));
    }

    #[test]
    fn chain_of_thought_reasoning() {
        let reasoner = Reasoner::new(ReasoningStrategy::ChainOfThought);
        let result = reasoner.think(&make_intent(), &make_ctx(), "");
        assert!(result.contains("Step 1"));
        assert!(result.contains("Step 2"));
        assert!(result.contains("Step 3"));
        assert!(result.contains("Step 4"));
        assert!(result.contains("No world observations"));
    }

    #[test]
    fn explicit_strategy_override() {
        let reasoner = Reasoner::new(ReasoningStrategy::Direct);
        let result = reasoner.think_with_strategy(
            &make_intent(),
            &make_ctx(),
            "state",
            &ReasoningStrategy::ChainOfThought,
        );
        assert!(result.contains("Step 1"));
    }

    #[test]
    fn world_state_included() {
        let reasoner = Reasoner::new(ReasoningStrategy::Direct);
        let result = reasoner.think(&make_intent(), &make_ctx(), "3 processes running");
        assert!(result.contains("3 processes running"));
    }

    #[test]
    fn default_strategy_getter_setter() {
        let mut reasoner = Reasoner::new(ReasoningStrategy::Direct);
        assert_eq!(*reasoner.default_strategy(), ReasoningStrategy::Direct);
        reasoner.set_default_strategy(ReasoningStrategy::ChainOfThought);
        assert_eq!(
            *reasoner.default_strategy(),
            ReasoningStrategy::ChainOfThought
        );
    }

    #[test]
    fn test_care_aware_direct() {
        let reasoner = Reasoner::new(ReasoningStrategy::Direct);
        let mut care = HashMap::new();
        care.insert("safety".to_string(), 1.0);
        let result = reasoner.think_with_care(&make_intent(), &make_ctx(), "", &care);
        assert!(result.contains("safety: 1.00"));
        assert!(result.contains("Care priorities"));
    }

    #[test]
    fn test_care_aware_cot() {
        let reasoner = Reasoner::new(ReasoningStrategy::ChainOfThought);
        let mut care = HashMap::new();
        care.insert("safety".to_string(), 1.0);
        let result = reasoner.think_with_care(&make_intent(), &make_ctx(), "", &care);
        assert!(result.contains("values-aware"));
    }

    #[test]
    fn test_care_aware_empty_weights() {
        let reasoner = Reasoner::new(ReasoningStrategy::Direct);
        let care = HashMap::new();
        let result = reasoner.think_with_care(&make_intent(), &make_ctx(), "", &care);
        assert!(!result.contains("Care priorities"));
    }

    #[test]
    fn test_care_aware_multiple_weights() {
        let reasoner = Reasoner::new(ReasoningStrategy::Direct);
        let mut care = HashMap::new();
        care.insert("safety".to_string(), 1.0);
        care.insert("helpfulness".to_string(), 0.7);
        let result = reasoner.think_with_care(&make_intent(), &make_ctx(), "", &care);
        assert!(result.contains("safety: 1.00"));
        assert!(result.contains("helpfulness: 0.70"));
    }

    #[test]
    fn test_stimmung_angst_uses_cot() {
        let reasoner = Reasoner::new(ReasoningStrategy::Direct);
        let mood = Stimmung::Angst {
            facing: base::dasein::AngstSource::Freedom,
        };
        let result = reasoner.think_with_stimmung(&make_intent(), &make_ctx(), "", &mood);
        // Angst should trigger ChainOfThought reasoning
        assert!(result.contains("Step 1"));
        assert!(result.contains("Stimmung: Angst"));
    }

    #[test]
    fn test_stimmung_calm_uses_direct() {
        let reasoner = Reasoner::new(ReasoningStrategy::ChainOfThought);
        let mood = Stimmung::Gelassenheit;
        let result = reasoner.think_with_stimmung(&make_intent(), &make_ctx(), "ok", &mood);
        // Calm should use Direct strategy
        assert!(result.contains("Direct execution"));
        assert!(result.contains("Gelassenheit"));
    }

    #[test]
    fn test_stimmung_verfallenheit_uses_cot() {
        let reasoner = Reasoner::new(ReasoningStrategy::Direct);
        let mood = Stimmung::Verfallenheit {
            absorbed_in: "debugging".to_string(),
        };
        let result = reasoner.think_with_stimmung(&make_intent(), &make_ctx(), "", &mood);
        assert!(result.contains("Step 1"));
        assert!(result.contains("Verfallenheit"));
        assert!(result.contains("debugging"));
    }

    #[test]
    fn test_strategy_for_stimmung() {
        assert_eq!(
            Reasoner::strategy_for_stimmung(&Stimmung::Angst {
                facing: base::dasein::AngstSource::Nothingness,
            }),
            ReasoningStrategy::ChainOfThought
        );
        assert_eq!(
            Reasoner::strategy_for_stimmung(&Stimmung::Gelassenheit),
            ReasoningStrategy::Direct
        );
        assert_eq!(
            Reasoner::strategy_for_stimmung(&Stimmung::Entschlossenheit {
                chosen_possibility: "test".to_string(),
            }),
            ReasoningStrategy::ChainOfThought
        );
        assert_eq!(
            Reasoner::strategy_for_stimmung(&Stimmung::Langeweile {
                depth: base::dasein::BoredomDepth::Deep,
            }),
            ReasoningStrategy::ChainOfThought
        );
        assert_eq!(
            Reasoner::strategy_for_stimmung(&Stimmung::Langeweile {
                depth: base::dasein::BoredomDepth::Surface,
            }),
            ReasoningStrategy::Direct
        );
    }
}
