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
pub mod evolution_trigger;
pub mod skill_extractor;

use aletheon_abi::{
    brain::{
        BehaviorAdjustment, BrainCoreOps, Critique, EvolutionLogEntry, ExecutionResult, Experience,
        LearnedRule, Observation, Plan, Reflection, ReflectionEntry, ReflectionOutcome,
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
use self::skill_extractor::SkillExtractor;
use self::world_model::WorldModel;
use crate::bridge::learning::LearningBridge;
use crate::bridge::llm::LlmBridge;
use crate::bridge::dual_model::{DualModelBridge, TaskComplexity};

/// ExperienceSummarizer — analyzes accumulated reflections and produces evolution log entries.
///
/// Detects behavioral patterns (repeated topics, repeated failures, success strategies)
/// and generates behavior adjustment suggestions.
pub struct ExperienceSummarizer;

impl ExperienceSummarizer {
    /// Analyze a batch of reflections and produce an EvolutionLogEntry.
    ///
    /// Returns `None` if no patterns are detected (fewer than 2 reflections
    /// and no significant signal).
    pub fn summarize(reflections: &[ReflectionEntry]) -> Option<EvolutionLogEntry> {
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
                patterns.push(format!(
                    "Repeated topic '{}' appeared {} times",
                    topic, count
                ));
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
                        "Recurring lesson: '{}' (mentioned {} times)",
                        lesson, count
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
        let avg_confidence: f64 = reflections.iter().map(|r| r.confidence).sum::<f64>()
            / reflections.len() as f64;
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
            timestamp: chrono::Utc::now(),
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
    skill_extractor: SkillExtractor,
    initialized: bool,
    // Real implementations
    llm: Option<LlmBridge>,
    dual_model: Option<DualModelBridge>,
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
            skill_extractor: SkillExtractor::new(),
            initialized: false,
            llm: None,
            dual_model: None,
            learning: None,
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
    /// When a dual-model bridge is configured and the task is complex, the planner
    /// model analyzes first and its output guides the executor. Otherwise uses the
    /// single LLM bridge. Falls back to the template-based reasoner if no LLM is set.
    async fn think(&self, intent: &Intent, ctx: &Context) -> Result<Plan> {
        let world_state = self.world_model.snapshot();
        let complexity = Self::estimate_complexity(intent);

        // Two-pass flow: planner analyzes, then executor produces the plan.
        let use_two_pass =
            self.dual_model.is_some() && complexity == TaskComplexity::Complex;

        if use_two_pass {
            let dm = self.dual_model.as_ref().unwrap();
            let learned_rules = self
                .learning
                .as_ref()
                .map(|l| l.rules_for_context())
                .unwrap_or_default();

            // Pass 1: planner analyzes the task
            let planner_prompt = format!(
                "You are an analytical planning model. Current world state:\n{}\n\n\
                 Learned rules:\n{}\n\n\
                 Intent: {}\nParameters: {}\nDescription: {}\n\n\
                 Analyze the intent and produce a brief analysis of the best approach, \
                 potential risks, and recommended steps.",
                world_state, learned_rules, intent.action, intent.parameters, intent.description
            );
            let planner_msgs = vec![
                LlmBridge::system_message("You are a planning/analysis model."),
                LlmBridge::user_message(&planner_prompt),
            ];
            let planner_resp = dm.planner().complete(&planner_msgs, &[]).await?;
            let planner_analysis: String = planner_resp
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            // Pass 2: executor produces the final plan, guided by the planner's analysis
            let executor_prompt = format!(
                "Intent: {}\nParameters: {}\nDescription: {}\n\n\
                 Planner analysis:\n{}\n\n\
                 Based on the above analysis, generate a plan with rollback actions.",
                intent.action, intent.parameters, intent.description, planner_analysis
            );
            let executor_msgs = vec![
                LlmBridge::system_message(
                    "You are an execution model that produces actionable plans.",
                ),
                LlmBridge::user_message(&executor_prompt),
            ];
            let executor_resp = dm.executor().complete(&executor_msgs, &[]).await?;
            let reasoning: String = executor_resp
                .content
                .iter()
                .filter_map(|b| match b {
                    ContentBlock::Text { text } => Some(text.as_str()),
                    _ => None,
                })
                .collect::<Vec<_>>()
                .join("");

            let plan = self.planner.generate_plan(intent, &reasoning, ctx);
            Ok(plan)
        } else if let Some(llm) = self.effective_llm(complexity) {
            // Single-LLM reasoning path
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

    // --- ExperienceSummarizer tests ---

    fn make_reflection_entry(
        outcome: ReflectionOutcome,
        task: &str,
        confidence: f64,
    ) -> ReflectionEntry {
        use aletheon_abi::ReflectionTrigger;
        ReflectionEntry {
            id: format!("ref-{}", uuid::Uuid::new_v4()),
            timestamp: chrono::Utc::now(),
            trigger: ReflectionTrigger::TaskComplete,
            task_summary: task.to_string(),
            outcome,
            what_worked: vec![],
            what_failed: vec![],
            learned: vec![],
            behavior_changes: vec![],
            confidence,
        }
    }

    #[test]
    fn summarizer_empty_input() {
        assert!(ExperienceSummarizer::summarize(&[]).is_none());
    }

    #[test]
    fn summarizer_single_reflection_no_pattern() {
        let entries = vec![make_reflection_entry(
            ReflectionOutcome::Success,
            "deploy feature",
            0.9,
        )];
        // Single entry with no strong pattern -> None
        assert!(ExperienceSummarizer::summarize(&entries).is_none());
    }

    #[test]
    fn summarizer_detects_high_failure_rate() {
        let entries = vec![
            make_reflection_entry(ReflectionOutcome::Failure, "parse input", 0.2),
            make_reflection_entry(ReflectionOutcome::Failure, "parse config", 0.1),
            make_reflection_entry(ReflectionOutcome::Success, "list files", 0.9),
        ];
        let result = ExperienceSummarizer::summarize(&entries).unwrap();
        assert!(result
            .patterns_detected
            .iter()
            .any(|p| p.contains("failure rate")));
        assert!(result
            .adjustments
            .iter()
            .any(|a| a.target == "care.safety.weight"));
    }

    #[test]
    fn summarizer_detects_repeated_topics() {
        let entries = vec![
            make_reflection_entry(ReflectionOutcome::Success, "deploy the service", 0.8),
            make_reflection_entry(ReflectionOutcome::Success, "deploy the service", 0.8),
            make_reflection_entry(ReflectionOutcome::Success, "deploy the service", 0.8),
        ];
        let result = ExperienceSummarizer::summarize(&entries).unwrap();
        assert!(result
            .patterns_detected
            .iter()
            .any(|p| p.contains("Repeated topic")));
    }

    #[test]
    fn summarizer_detects_low_confidence() {
        let entries = vec![
            make_reflection_entry(ReflectionOutcome::Partial, "debug crash A", 0.2),
            make_reflection_entry(ReflectionOutcome::Partial, "debug crash B", 0.3),
            make_reflection_entry(ReflectionOutcome::Partial, "debug crash C", 0.1),
            make_reflection_entry(ReflectionOutcome::Partial, "debug crash D", 0.3),
        ];
        let result = ExperienceSummarizer::summarize(&entries).unwrap();
        assert!(result
            .patterns_detected
            .iter()
            .any(|p| p.contains("Low average confidence")));
        assert!(result
            .adjustments
            .iter()
            .any(|a| a.target == "care.efficiency.weight"));
    }

    #[test]
    fn summarizer_success_strategy_with_common_lessons() {
        let mut e1 = make_reflection_entry(ReflectionOutcome::Success, "task A", 0.9);
        e1.learned = vec!["always validate inputs".to_string()];
        let mut e2 = make_reflection_entry(ReflectionOutcome::Success, "task B", 0.85);
        e2.learned = vec!["always validate inputs".to_string()];
        let entries = vec![e1, e2];

        let result = ExperienceSummarizer::summarize(&entries).unwrap();
        assert!(result
            .patterns_detected
            .iter()
            .any(|p| p.contains("Recurring lesson")));
        assert!(result
            .adjustments
            .iter()
            .any(|a| a.target == "care.learning.weight"));
    }

    // --- Dual-model tests ---

    use crate::bridge::dual_model::{DualModelBridge, DualModelConfig, TaskComplexity};
    use crate::r#impl::llm::{LlmProvider, LlmResponse, LlmStream, StopReason, ToolDefinition, Usage};
    use aletheon_abi::message::Message;
    use std::sync::Arc;

    /// Stub provider whose name appears in its response text.
    struct StubProvider {
        tag: String,
    }

    #[async_trait]
    impl LlmProvider for StubProvider {
        async fn complete(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmResponse> {
            Ok(LlmResponse {
                content: vec![ContentBlock::Text {
                    text: format!("{} response", self.tag),
                }],
                stop_reason: StopReason::EndTurn,
                usage: Usage {
                    input_tokens: 1,
                    output_tokens: 1,
                },
                cache_hit_tokens: 0,
                cache_miss_tokens: 0,
            })
        }
        async fn complete_stream(
            &self,
            _messages: &[Message],
            _tools: &[ToolDefinition],
        ) -> anyhow::Result<LlmStream> {
            unimplemented!()
        }
        fn name(&self) -> &str {
            &self.tag
        }
        fn max_context_length(&self) -> usize {
            128_000
        }
    }

    fn make_dual_brain_core() -> BrainCore {
        let planner = LlmBridge::new(Arc::new(StubProvider { tag: "planner".into() }));
        let executor = LlmBridge::new(Arc::new(StubProvider { tag: "executor".into() }));
        let dm = DualModelBridge::new(planner, executor, DualModelConfig::default());
        BrainCore::new(make_config()).with_dual_model(dm)
    }

    #[tokio::test]
    async fn dual_model_think_simple_uses_executor_only() {
        let bc = make_dual_brain_core();
        let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
        // Simple task → executor only, so reasoning should contain "executor"
        assert!(plan.reasoning.contains("executor response"));
    }

    #[tokio::test]
    async fn dual_model_think_complex_uses_planner_then_executor() {
        let bc = make_dual_brain_core();
        // Build a complex intent (description > 512 chars)
        let long_desc = "x".repeat(600);
        let intent = Intent {
            action: "complex.task".to_string(),
            parameters: json!({"data": "small"}),
            source: IntentSource::User,
            description: long_desc,
        };
        let plan = bc.think(&intent, &make_ctx()).await.unwrap();
        // Complex task → two-pass: executor is the final responder
        assert!(plan.reasoning.contains("executor response"));
    }

    #[test]
    fn estimate_complexity_simple() {
        let intent = make_intent(); // short description
        assert_eq!(BrainCore::estimate_complexity(&intent), TaskComplexity::Simple);
    }

    #[test]
    fn estimate_complexity_complex() {
        let intent = Intent {
            action: "test".into(),
            parameters: json!({}),
            source: IntentSource::User,
            description: "y".repeat(600),
        };
        assert_eq!(BrainCore::estimate_complexity(&intent), TaskComplexity::Complex);
    }

    #[test]
    fn estimate_complexity_medium() {
        let intent = Intent {
            action: "test".into(),
            parameters: json!({}),
            source: IntentSource::User,
            description: "z".repeat(200),
        };
        assert_eq!(BrainCore::estimate_complexity(&intent), TaskComplexity::Medium);
    }

    #[tokio::test]
    async fn dual_model_fallback_single_llm() {
        // When dual_model is set but task is Simple, effective_llm returns executor
        let bc = make_dual_brain_core();
        let plan = bc.think(&make_intent(), &make_ctx()).await.unwrap();
        assert!(!plan.steps.is_empty());
    }
}
