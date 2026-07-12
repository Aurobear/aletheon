//! CognitCore — the cognitive computation engine.
//!
//! Wires all components (Reasoner, Planner, Critic, Reflector, Learner, WorldModel)
//! into a single struct that implements `CognitOps` and `Subsystem`.
//!
//! CognitCore has NO self — it's a pure computation engine.
//! "Should I?" is SelfField's job. "How do I?" is CognitCore's job.

pub mod awareness;
pub mod awareness_signal;
pub mod brain_core_subsystem;
pub mod cognit_ops;
pub mod critic;
pub mod evolution_trigger;
pub mod experience_summarizer;
pub mod learner;
pub mod planner;
pub mod reasoner;
pub mod reflector;
pub mod skill_extractor;
pub mod world_model;

#[cfg(test)]
#[path = "tests.rs"]
mod tests;

use anyhow::Result;
use fabric::message::ContentBlock;
use fabric::self_field::Intent;
use std::sync::Arc;

use self::awareness::{AwarenessContext, AwarenessGenerator};
use self::critic::Critic;
pub use self::experience_summarizer::ExperienceSummarizer;
use self::learner::Learner;
use self::planner::Planner;
use self::reasoner::{Reasoner, ReasoningStrategy};
use self::reflector::Reflector;
use self::skill_extractor::SkillExtractor;
use self::world_model::WorldModel;
use crate::bridge::dual_model::{DualModelBridge, TaskComplexity};
use crate::bridge::learning::LearningBridge;
use crate::bridge::llm::LlmBridge;
use fabric::{
    cognit::Plan, context::Context, self_field::AwarenessGrowthSuggestion, Clock, SelfAwareness,
};

/// Configuration for CognitCore construction.
pub struct CognitCoreConfig {
    /// Default reasoning strategy.
    pub reasoning_strategy: ReasoningStrategy,
    /// Maximum number of learned rules.
    pub max_learned_rules: usize,
    /// Maximum number of world observations.
    pub max_world_observations: usize,
    /// Clock for deterministic time (test harness injection).
    pub clock: Arc<dyn Clock>,
}

impl Default for CognitCoreConfig {
    fn default() -> Self {
        Self {
            reasoning_strategy: ReasoningStrategy::ChainOfThought,
            max_learned_rules: 200,
            max_world_observations: 500,
            clock: Arc::new(aletheon_kernel::chronos::SystemClock::new()),
        }
    }
}

/// CognitCore — the cognitive computation engine.
///
/// Wires all components and implements `CognitOps` + `Subsystem`.
pub struct CognitCore {
    // Keep existing components (they provide structure)
    reasoner: Reasoner,
    planner: Planner,
    critic: Critic,
    reflector: Reflector,
    learner: Learner,
    world_model: WorldModel,
    skill_extractor: SkillExtractor,
    pub(crate) initialized: bool,
    // Real implementations
    llm: Option<LlmBridge>,
    dual_model: Option<DualModelBridge>,
    learning: Option<LearningBridge>,
    awareness_generator: AwarenessGenerator,
    clock: Arc<dyn Clock>,
}

impl CognitCore {
    pub fn new(config: CognitCoreConfig) -> Self {
        let clock = config.clock;
        Self {
            reasoner: Reasoner::new(config.reasoning_strategy),
            planner: Planner::new(),
            critic: Critic::new(),
            reflector: Reflector::new(clock.clone()),
            learner: Learner::new(config.max_learned_rules),
            world_model: WorldModel::new(config.max_world_observations, clock.clone()),
            skill_extractor: SkillExtractor::new(clock.clone()),
            initialized: false,
            llm: None,
            dual_model: None,
            learning: None,
            awareness_generator: AwarenessGenerator::new(),
            clock,
        }
    }

    /// Set the LLM provider for real reasoning.
    pub fn with_llm(mut self, llm: LlmBridge) -> Self {
        self.llm = Some(llm);
        self
    }

    /// Set the dual-model bridge for planner/executor routing.
    pub fn with_dual_model(mut self, dual_model: DualModelBridge) -> Self {
        self.dual_model = Some(dual_model);
        self
    }

    /// Set the learning pipeline.
    pub fn with_learning(mut self, learning: LearningBridge) -> Self {
        self.learning = Some(learning);
        self
    }

    /// Access the world model (for external observation injection).
    pub fn world_model(&self) -> &WorldModel {
        &self.world_model
    }

    /// Access the learner (for rule inspection).
    pub fn learner(&self) -> &Learner {
        &self.learner
    }

    /// Access the reasoner (for strategy changes).
    pub fn reasoner_mut(&mut self) -> &mut Reasoner {
        &mut self.reasoner
    }

    /// Estimate task complexity from intent.
    ///
    /// Simple heuristics based on parameter/description size.
    pub fn estimate_complexity(intent: &Intent) -> TaskComplexity {
        let param_len = intent.parameters.to_string().len();
        let desc_len = intent.description.len();

        if param_len > 512 || desc_len > 512 {
            TaskComplexity::Complex
        } else if param_len > 128 || desc_len > 128 {
            TaskComplexity::Medium
        } else {
            TaskComplexity::Simple
        }
    }

    /// Validate executor's plan against planner's analysis.
    ///
    /// Extracts key nouns (words >= 4 chars) from the planner's analysis and checks
    /// whether enough appear in the executor's reasoning. If coverage is low,
    /// re-prompts the executor once with the missing key terms highlighted.
    ///
    /// Returns the (possibly re-prompted) reasoning text.
    pub(crate) async fn validate_and_reprompt(
        dm: &DualModelBridge,
        planner_analysis: &str,
        executor_reasoning: &str,
        original_executor_prompt: &str,
    ) -> String {
        let key_terms = Self::extract_key_terms(planner_analysis);
        if key_terms.is_empty() {
            return executor_reasoning.to_string();
        }

        let reasoning_lower = executor_reasoning.to_lowercase();
        let (covered, missing): (Vec<_>, Vec<_>) = key_terms
            .iter()
            .partition(|term| reasoning_lower.contains(&term.to_lowercase()));

        // Require at least 40% coverage of key terms
        let total = key_terms.len();
        let coverage = covered.len() as f64 / total as f64;
        if coverage >= 0.4 {
            return executor_reasoning.to_string();
        }

        // Re-prompt once with missing terms highlighted
        let missing_str = missing
            .iter()
            .take(10)
            .map(|s| s.as_str())
            .collect::<Vec<_>>()
            .join(", ");
        let correction_prompt = format!(
            "{}\n\nIMPORTANT: Your plan must address these key points from the planner's analysis: {}",
            original_executor_prompt, missing_str
        );
        let msgs = vec![
            LlmBridge::system_message(
                "You are an execution model that produces actionable plans. \
                 You MUST address all key points from the planner's analysis.",
            ),
            LlmBridge::user_message(&correction_prompt),
        ];

        match dm.executor().complete(&msgs, &[]).await {
            Ok(resp) => resp
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join(""),
            Err(_) => executor_reasoning.to_string(), // fallback to original
        }
    }

    /// Extract key terms from text — words >= 4 chars, excluding common stopwords.
    fn extract_key_terms(text: &str) -> Vec<String> {
        let stopwords: &[&str] = &[
            "this",
            "that",
            "with",
            "from",
            "have",
            "been",
            "will",
            "would",
            "could",
            "should",
            "into",
            "about",
            "also",
            "more",
            "some",
            "than",
            "them",
            "then",
            "there",
            "these",
            "they",
            "very",
            "what",
            "when",
            "your",
            "each",
            "make",
            "most",
            "only",
            "over",
            "such",
            "take",
            "well",
            "just",
            "like",
            "using",
            "based",
            "after",
            "before",
            "does",
            "done",
            "ensure",
            "consider",
            "possible",
            "potential",
            "recommended",
            "analysis",
            "approach",
            "best",
            "brief",
            "produce",
            "generate",
            "above",
            "following",
            "result",
        ];

        text.split(|c: char| !c.is_alphanumeric())
            .filter(|w| w.len() >= 4)
            .map(|w| w.to_lowercase())
            .filter(|w| !stopwords.contains(&w.as_str()))
            .collect::<std::collections::HashSet<_>>()
            .into_iter()
            .collect()
    }

    /// Get the effective LLM for a given complexity level.
    ///
    /// If dual_model is configured, routes through it. Otherwise falls back to
    /// the single `llm` bridge.
    fn effective_llm(&self, complexity: TaskComplexity) -> Option<&LlmBridge> {
        if let Some(dm) = &self.dual_model {
            Some(dm.route(complexity))
        } else {
            self.llm.as_ref()
        }
    }

    /// Access the skill extractor (for extracting skills from reflections).
    pub fn skill_extractor(&self) -> &SkillExtractor {
        &self.skill_extractor
    }

    /// Generate self-awareness for a reasoning act.
    ///
    /// This is a side-channel method — it does NOT change the
    /// CognitOps::think() return type. Instead, awareness
    /// is generated alongside reasoning and stored separately.
    ///
    /// Call this after think() to produce awareness for the
    /// reasoning that just happened.
    pub fn generate_awareness(&self, action: &str, context: &AwarenessContext) -> SelfAwareness {
        self.awareness_generator.generate_enriched(action, context)
    }

    /// Update awareness growth suggestions.
    ///
    /// Called when AwarenessGrowthAnalyzer produces new suggestions.
    pub fn update_awareness_suggestions(&mut self, suggestions: Vec<AwarenessGrowthSuggestion>) {
        self.awareness_generator.update_suggestions(suggestions);
    }

    /// Think with iterative refinement — the P4 plan-critique-revise loop.
    ///
    /// 1. Queries learned rules for the intent context.
    /// 2. Generates initial plan (with task decomposition if LLM output supports it).
    /// 3. Runs up to 3 critique-revise rounds, stopping when no Critical-severity
    ///    (Fatal or Error) critiques remain.
    ///
    /// Falls back to template-based reasoning when no LLM is available.
    pub async fn think_with_refinement(
        &mut self,
        intent: &Intent,
        ctx: &Context,
    ) -> Result<(Plan, String)> {
        const MAX_REFINE_ROUNDS: usize = 3;

        let world_state = self.world_model.snapshot();

        // 1. Query learned rules for this intent context
        let learned_rules_text = self.learner.rules_for_context(&intent.description);

        // 2. Generate initial reasoning + plan
        let (mut plan, mut reasoning) = self
            .generate_initial_plan(intent, ctx, &world_state, &learned_rules_text)
            .await?;

        // 3. Refinement loop: critique -> revise, max 3 rounds
        for _round in 0..MAX_REFINE_ROUNDS {
            let critiques = self.critic.critique(&plan);

            if !Critic::has_critical(&critiques) {
                break;
            }

            // Build revision prompt and call LLM
            let revision_prompt = Critic::build_revision_prompt(&critiques);
            let revised_reasoning = self.revise_via_llm(intent, &revision_prompt).await?;

            // Try to parse subtasks from revised output
            if let Some(subtasks) = self.planner.parse_subtasks(&revised_reasoning) {
                plan = self
                    .planner
                    .generate_multi_step_plan(intent, &revised_reasoning, subtasks);
            } else {
                plan = self.planner.generate_plan(intent, &revised_reasoning, ctx);
            }
            reasoning = revised_reasoning;
        }

        Ok((plan, reasoning))
    }

    /// Internal: generate the initial plan, incorporating learned rules and task decomposition.
    async fn generate_initial_plan(
        &self,
        intent: &Intent,
        ctx: &Context,
        world_state: &str,
        learned_rules: &str,
    ) -> Result<(Plan, String)> {
        let complexity = Self::estimate_complexity(intent);

        if let Some(llm) = self.effective_llm(complexity) {
            let system_prompt = if learned_rules.is_empty() {
                format!(
                    "You are a reasoning engine. Current world state:\n{}",
                    world_state
                )
            } else {
                format!(
                    "You are a reasoning engine. Current world state:\n{}\n\n{}",
                    world_state, learned_rules
                )
            };

            let user_prompt = format!(
                "Intent: {}\nParameters: {}\nDescription: {}\n\n\
                 Break this into subtasks as a JSON array of {{\"action\": \"...\", \"params\": {{...}}}}.\n\
                 If the intent is simple enough for a single step, still output a JSON array with one element.\n\
                 Include rollback considerations for each step.",
                intent.action, intent.parameters, intent.description
            );

            let messages = vec![
                LlmBridge::system_message(&system_prompt),
                LlmBridge::user_message(&user_prompt),
            ];

            let response = llm.complete(&messages, &[]).await?;
            let reasoning: String = response
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            // Try task decomposition first
            if let Some(subtasks) = self.planner.parse_subtasks(&reasoning) {
                let plan = self
                    .planner
                    .generate_multi_step_plan(intent, &reasoning, subtasks);
                Ok((plan, reasoning))
            } else {
                let plan = self.planner.generate_plan(intent, &reasoning, ctx);
                Ok((plan, reasoning))
            }
        } else {
            // Template fallback
            let reasoning = self.reasoner.think(intent, ctx, world_state);
            let plan = self.planner.generate_plan(intent, &reasoning, ctx);
            Ok((plan, reasoning))
        }
    }

    /// Internal: call LLM with a revision prompt to get a corrected plan.
    async fn revise_via_llm(&self, intent: &Intent, revision_prompt: &str) -> Result<String> {
        let complexity = Self::estimate_complexity(intent);

        if let Some(llm) = self.effective_llm(complexity) {
            let system_prompt = "You are a plan revision engine. \
                 Given a plan with identified issues, produce a corrected plan.\n\
                 Output the revised plan as a JSON array of steps with 'action' and 'params' fields.";

            let user_prompt = format!(
                "Original intent: {}\nDescription: {}\n\n{}",
                intent.action, intent.description, revision_prompt
            );

            let messages = vec![
                LlmBridge::system_message(system_prompt),
                LlmBridge::user_message(&user_prompt),
            ];

            let response = llm.complete(&messages, &[]).await?;
            let revised: String = response
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            Ok(revised)
        } else {
            // No LLM — return the original reasoning; the loop will break
            // because template plans don't produce Critical critiques.
            Ok(revision_prompt.to_string())
        }
    }
}
