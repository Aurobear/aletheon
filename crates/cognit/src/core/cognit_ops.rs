//! CognitOps implementation for CognitCore.

use anyhow::Result;
use async_trait::async_trait;

use fabric::cognit::{
    CognitOps, Critique, ExecutionResult, Experience, LearnedRule, Observation, Plan, Reflection,
};
use fabric::context::Context;
use fabric::message::ContentBlock;
use fabric::self_field::Intent;

use super::CognitCore;
use crate::bridge::dual_model::TaskComplexity;
use crate::bridge::learning::LearningBridge;
use crate::bridge::llm::LlmBridge;

#[async_trait]
impl CognitOps for CognitCore {
    /// Think about an intent and produce a plan.
    ///
    /// When a dual-model bridge is configured and the task is complex, the planner
    /// model analyzes first and its output guides the executor. Otherwise uses the
    /// single LLM bridge. Falls back to the template-based reasoner if no LLM is set.
    async fn think(&self, intent: &Intent, ctx: &Context) -> Result<Plan> {
        let world_state = self.world_model.snapshot();
        let complexity = Self::estimate_complexity(intent);

        // Two-pass flow: planner analyzes, then executor produces the plan.
        let use_two_pass = self.dual_model.is_some() && complexity == TaskComplexity::Complex;

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

            // Validate executor's plan against planner's analysis.
            // Extract key terms from analysis and check coverage in reasoning.
            let reasoning =
                Self::validate_and_reprompt(dm, &planner_analysis, &reasoning, &executor_prompt)
                    .await;

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
            let learned = new_rules
                .iter()
                .map(LearningBridge::to_learned_rule)
                .collect();

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
