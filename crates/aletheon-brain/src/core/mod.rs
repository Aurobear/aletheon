//! BrainCore — the cognitive computation engine.
//!
//! Wires all components (Reasoner, Planner, Critic, Reflector, Learner, WorldModel)
//! into a single struct that implements `BrainCoreOps` and `Subsystem`.
//!
//! BrainCore has NO self — it's a pure computation engine.
//! "Should I?" is SelfField's job. "How do I?" is BrainCore's job.

pub mod reasoner;
pub mod planner;
pub mod critic;
pub mod reflector;
pub mod learner;
pub mod world_model;

use aletheon_abi::{
    brain::{
        BrainCoreOps, Critique, ExecutionResult, Experience, LearnedRule, Observation, Plan,
        Reflection,
    },
    context::Context,
    self_field::Intent,
    Subsystem, SubsystemContext, SubsystemHealth, Version,
};
use anyhow::Result;
use aletheon_abi::message::ContentBlock;
use async_trait::async_trait;
use tracing::info;

use self::critic::Critic;
use self::learner::Learner;
use self::planner::Planner;
use self::reasoner::{Reasoner, ReasoningStrategy};
use self::reflector::Reflector;
use self::world_model::WorldModel;
use crate::bridge::learning::LearningBridge;
use crate::bridge::llm::LlmBridge;

/// Configuration for BrainCore construction.
pub struct BrainCoreConfig {
    /// Default reasoning strategy.
    pub reasoning_strategy: ReasoningStrategy,
    /// Maximum number of learned rules.
    pub max_learned_rules: usize,
    /// Maximum number of world observations.
    pub max_world_observations: usize,
}

impl Default for BrainCoreConfig {
    fn default() -> Self {
        Self {
            reasoning_strategy: ReasoningStrategy::ChainOfThought,
            max_learned_rules: 200,
            max_world_observations: 500,
        }
    }
}

/// BrainCore — the cognitive computation engine.
///
/// Wires all components and implements `BrainCoreOps` + `Subsystem`.
pub struct BrainCore {
    // Keep existing components (they provide structure)
    reasoner: Reasoner,
    planner: Planner,
    critic: Critic,
    reflector: Reflector,
    learner: Learner,
    world_model: WorldModel,
    initialized: bool,
    // Real implementations
    llm: Option<LlmBridge>,
    learning: Option<LearningBridge>,
}

impl BrainCore {
    pub fn new(config: BrainCoreConfig) -> Self {
        Self {
            reasoner: Reasoner::new(config.reasoning_strategy),
            planner: Planner::new(),
            critic: Critic::new(),
            reflector: Reflector::new(),
            learner: Learner::new(config.max_learned_rules),
            world_model: WorldModel::new(config.max_world_observations),
            initialized: false,
            llm: None,
            learning: None,
        }
    }

    /// Set the LLM provider for real reasoning.
    pub fn with_llm(mut self, llm: LlmBridge) -> Self {
        self.llm = Some(llm);
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
}

#[async_trait]
impl Subsystem for BrainCore {
    fn name(&self) -> &str {
        "brain_core"
    }

    async fn init(&mut self, _ctx: &SubsystemContext) -> Result<()> {
        info!("BrainCore initializing");
        self.initialized = true;
        Ok(())
    }

    async fn health(&self) -> SubsystemHealth {
        if !self.initialized {
            return SubsystemHealth::Degraded {
                reason: "Not yet initialized".to_string(),
            };
        }
        SubsystemHealth::Healthy
    }

    async fn shutdown(&mut self) -> Result<()> {
        info!("BrainCore shutting down");
        self.world_model.clear();
        self.initialized = false;
        Ok(())
    }

    fn version(&self) -> Version {
        Version::new(0, 1, 0)
    }
}

#[async_trait]
impl BrainCoreOps for BrainCore {
    /// Think about an intent and produce a plan.
    ///
    /// When an LLM bridge is available, uses real LLM reasoning.
    /// Falls back to the template-based reasoner otherwise.
    async fn think(&self, intent: &Intent, ctx: &Context) -> Result<Plan> {
        let world_state = self.world_model.snapshot();

        if let Some(llm) = &self.llm {
            // REAL reasoning path: use LLM
            let learned_rules = self
                .learning
                .as_ref()
                .map(|l| l.rules_for_context())
                .unwrap_or_default();

            let system_prompt = format!(
                "You are a reasoning engine. Current world state:\n{}\n\nLearned rules:\n{}",
                world_state, learned_rules,
            );

            let user_prompt = format!(
                "Intent: {}\nParameters: {}\nDescription: {}\n\nGenerate a plan with rollback actions.",
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

            // Use planner to structure the LLM's reasoning into a Plan
            let plan = self.planner.generate_plan(intent, &reasoning, ctx);
            Ok(plan)
        } else {
            // FALLBACK: use the existing template-based reasoner
            let reasoning = self.reasoner.think(intent, ctx, &world_state);
            let plan = self.planner.generate_plan(intent, &reasoning, ctx);
            Ok(plan)
        }
    }

    /// Reflect on an execution result.
    ///
    /// When an LLM bridge is available, uses it for deeper analysis.
    /// Falls back to the template-based reflector otherwise.
    async fn reflect(&self, execution: &ExecutionResult) -> Result<Reflection> {
        if let Some(llm) = &self.llm {
            let prompt = format!(
                "Analyze this execution result and provide reflection:\n\
                 Plan ID: {}\nSuccess: {}\nSteps: {}/{}\nOutput: {}\nError: {:?}\n\n\
                 Provide: what_worked, what_failed, what_to_improve, confidence (0.0-1.0)",
                execution.plan_id,
                execution.success,
                execution.steps_completed,
                execution.steps_total,
                execution.output,
                execution.error
            );

            let messages = vec![
                LlmBridge::system_message(
                    "You are a reflection engine. Analyze execution results.",
                ),
                LlmBridge::user_message(&prompt),
            ];

            let response = llm.complete(&messages, &[]).await?;
            let _analysis: String = response
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            // For now, use the reflector to produce a structured Reflection.
            // A future iteration can parse the LLM analysis into structured fields.
            Ok(self.reflector.reflect(execution))
        } else {
            Ok(self.reflector.reflect(execution))
        }
    }

    /// Critique a plan before execution.
    async fn critique(&self, plan: &Plan) -> Result<Vec<Critique>> {
        Ok(self.critic.critique(plan))
    }

    /// Learn from experience — extract reusable rules.
    ///
    /// When a learning bridge is available, records the outcome and extracts
    /// patterns. Falls back to the template-based learner otherwise.
    async fn learn(&self, experience: &Experience) -> Result<Vec<LearnedRule>> {
        if let Some(learning) = &self.learning {
            // Record the outcome
            learning.record_outcome(&experience.action, &experience.result, "session")?;

            // Extract patterns and get new rules
            let new_rules = learning.extract_and_update()?;

            // Convert to aletheon LearnedRule format
            let learned = new_rules.iter().map(LearningBridge::to_learned_rule).collect();

            Ok(learned)
        } else {
            // Fallback to existing learner
            Ok(self.learner.learn(experience))
        }
    }

    /// Update world model with new observation.
    async fn update_world(&self, observation: &Observation) -> Result<()> {
        self.world_model.update(observation.clone());
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_abi::body::{Action, ActionResult};
    use aletheon_abi::{IntentSource, SubsystemContext};
    use serde_json::json;
    use std::path::PathBuf;

    fn make_config() -> BrainCoreConfig {
        BrainCoreConfig::default()
    }

    fn make_intent() -> Intent {
        Intent {
            action: "shell.execute".to_string(),
            parameters: json!({"command": "ls -la"}),
            source: IntentSource::User,
            description: "List files".to_string(),
        }
    }

    fn make_ctx() -> Context {
        Context::new("test_session", PathBuf::from("/tmp"))
    }

    #[tokio::test]
    async fn think_produces_plan() {
        let bc = BrainCore::new(make_config());
        let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
        assert!(!plan.steps.is_empty());
        assert!(!plan.reasoning.is_empty());
    }

    #[tokio::test]
    async fn think_uses_world_state() {
        let bc = BrainCore::new(make_config());
        bc.world_model().update(Observation {
            what: "disk full".to_string(),
            source: "system".to_string(),
            data: json!({"usage": "95%"}),
        });
        let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
        assert!(plan.reasoning.contains("disk full"));
    }

    #[tokio::test]
    async fn critique_plan() {
        let bc = BrainCore::new(make_config());
        let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
        let critiques = bc.critique(&plan).await.unwrap();
        // A simple plan should have minimal critiques
        assert!(critiques.iter().all(|c| c.severity <= aletheon_abi::brain::CriticismSeverity::Info));
    }

    #[tokio::test]
    async fn reflect_on_execution() {
        let bc = BrainCore::new(make_config());
        let execution = ExecutionResult {
            plan_id: uuid::Uuid::new_v4(),
            success: true,
            steps_completed: 1,
            steps_total: 1,
            output: "done".to_string(),
            error: None,
            elapsed_ms: 100,
        };
        let reflection = bc.reflect(&execution).await.unwrap();
        assert!(!reflection.what_worked.is_empty());
        assert!(reflection.confidence > 0.5);
    }

    #[tokio::test]
    async fn learn_from_experience() {
        let bc = BrainCore::new(make_config());
        let experience = Experience {
            action: Action {
                name: "shell.execute".to_string(),
                parameters: json!({}),
                requires_sandbox: false,
                timeout: None,
            },
            result: ActionResult {
                success: false,
                output: String::new(),
                error: Some("command not found".to_string()),
                elapsed_ms: 50,
                truncated: false,
                side_effects: vec![],
            },
            context: make_ctx(),
        };
        let rules = bc.learn(&experience).await.unwrap();
        assert!(!rules.is_empty());
        assert!(rules[0].pattern.contains("shell.execute"));
    }

    #[tokio::test]
    async fn update_world() {
        let bc = BrainCore::new(make_config());
        let obs = Observation {
            what: "test event".to_string(),
            source: "test".to_string(),
            data: json!({"key": "value"}),
        };
        bc.update_world(&obs).await.unwrap();
        assert_eq!(bc.world_model().count(), 1);
    }

    #[tokio::test]
    async fn subsystem_lifecycle() {
        let mut bc = BrainCore::new(make_config());
        assert_eq!(bc.name(), "brain_core");
        assert!(matches!(bc.health().await, SubsystemHealth::Degraded { .. }));

        let ctx = SubsystemContext {
            name: "brain_core".to_string(),
            working_dir: PathBuf::from("/tmp"),
            config: json!({}),
        };
        bc.init(&ctx).await.unwrap();
        assert!(matches!(bc.health().await, SubsystemHealth::Healthy));

        bc.shutdown().await.unwrap();
        assert!(matches!(bc.health().await, SubsystemHealth::Degraded { .. }));
        assert_eq!(bc.world_model().count(), 0);
    }

    #[tokio::test]
    async fn full_pipeline_think_critique_execute_reflect_learn() {
        let bc = BrainCore::new(make_config());

        // 1. Think
        let intent = make_intent();
        let ctx = make_ctx();
        let plan = bc.think(&intent, &ctx).await.unwrap();
        assert!(!plan.steps.is_empty());

        // 2. Critique
        let critiques = bc.critique(&plan).await.unwrap();
        // Should be clean for a simple plan
        assert!(critiques.iter().all(|c| c.severity <= aletheon_abi::brain::CriticismSeverity::Warning));

        // 3. Simulate execution
        let execution = ExecutionResult {
            plan_id: plan.id,
            success: true,
            steps_completed: plan.steps.len(),
            steps_total: plan.steps.len(),
            output: "success".to_string(),
            error: None,
            elapsed_ms: 200,
        };

        // 4. Reflect
        let reflection = bc.reflect(&execution).await.unwrap();
        assert!(!reflection.what_worked.is_empty());
        assert!(reflection.confidence > 0.7);

        // 5. Learn
        let experience = Experience {
            action: plan.steps[0].action.clone(),
            result: ActionResult {
                success: true,
                output: "success".to_string(),
                error: None,
                elapsed_ms: 200,
                truncated: false,
                side_effects: vec![],
            },
            context: ctx,
        };
        let rules = bc.learn(&experience).await.unwrap();
        // Fast successful non-destructive action — no rules expected
        // (shell.execute is not destructive by name)
        // But this validates the pipeline runs without error
        let _ = rules;
    }

    #[tokio::test]
    async fn think_with_multiple_observations() {
        let bc = BrainCore::new(make_config());

        // Add several observations
        for i in 0..5 {
            bc.world_model().update(Observation {
                what: format!("observation_{}", i),
                source: "sensor".to_string(),
                data: json!({"index": i}),
            });
        }

        let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
        // Reasoning should reference world state
        assert!(plan.reasoning.contains("observation_4") || plan.reasoning.contains("sensor"));
    }
}
