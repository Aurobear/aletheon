//! Reasoner — multi-strategy reasoning engine.
//!
//! Given an intent and world state, produces a reasoning chain (as a string)
//! that explains how to approach the problem. The reasoning chain feeds into
//! the Planner to produce concrete steps.

use aletheon_abi::context::Context;
use aletheon_abi::self_field::Intent;

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
    pub fn think(
        &self,
        intent: &Intent,
        ctx: &Context,
        world_state: &str,
    ) -> String {
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
            intent.description,
            intent.action,
            intent.source,
            intent.parameters
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
    use aletheon_abi::{IntentSource, Context};
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
        assert_eq!(*reasoner.default_strategy(), ReasoningStrategy::ChainOfThought);
    }
}
