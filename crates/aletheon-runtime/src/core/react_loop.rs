use aletheon_abi::body::Action;
use aletheon_abi::self_field::{Intent, IntentSource};
use crate::core::config::RuntimeConfig;
use crate::core::goal_tracker::GoalTracker;
use crate::core::reflection::ReflectionEngine;

/// The ReAct (Reason + Act) iteration loop
/// This is the core cognitive cycle extracted from Engine::run_turn()
pub struct ReActLoop {
    config: RuntimeConfig,
    iteration: usize,
    /// Goal and sub-goal tracker (spec-driven).
    goal_tracker: GoalTracker,
    /// Periodic reflection engine with spec deviation detection.
    reflection_engine: ReflectionEngine,
}

impl ReActLoop {
    pub fn new(config: RuntimeConfig) -> Self {
        let reflection_interval = 5; // default, could come from config
        Self {
            config,
            iteration: 0,
            goal_tracker: GoalTracker::new(),
            reflection_engine: ReflectionEngine::new(reflection_interval),
        }
    }

    /// Current iteration number
    pub fn iteration(&self) -> usize {
        self.iteration
    }

    /// Reset iteration counter for a new turn
    pub fn reset(&mut self) {
        self.iteration = 0;
        self.reflection_engine.reset();
        // Note: goal_tracker is NOT reset here — spec persists across turns
    }

    /// Check if we've hit the max iterations
    pub fn should_continue(&self) -> bool {
        self.iteration < self.config.max_iterations
    }

    /// Increment iteration counter
    pub fn advance(&mut self) {
        self.iteration += 1;
    }

    /// Build an Intent from user input
    pub fn build_intent(&self, input: &str) -> Intent {
        Intent {
            action: "user_request".to_string(),
            parameters: serde_json::json!({"input": input}),
            source: IntentSource::User,
            description: input.to_string(),
        }
    }

    /// Build an Action from a plan step
    pub fn step_to_action(&self, tool_name: &str, params: serde_json::Value) -> Action {
        Action {
            name: tool_name.to_string(),
            parameters: params,
            requires_sandbox: false,
            timeout: None,
        }
    }

    /// Max iterations
    pub fn max_iterations(&self) -> usize {
        self.config.max_iterations
    }

    // --- Spec & Goal integration ---

    /// Set the goal for this turn.
    pub fn set_goal(&mut self, goal: String) {
        self.goal_tracker.set_goal(goal);
    }

    /// Load a spec file into the goal tracker.
    pub fn load_spec(&mut self, path: &str) -> Result<(), String> {
        self.goal_tracker.load_spec_from_file(path)
    }

    /// Get the current goal context for LLM prompting.
    pub fn get_goal_context(&self) -> String {
        self.goal_tracker.get_context()
    }

    /// Get the current constraints from the loaded spec.
    pub fn get_constraints(&self) -> &[String] {
        self.goal_tracker.get_constraints()
    }

    /// Get a reference to the goal tracker.
    pub fn goal_tracker(&self) -> &GoalTracker {
        &self.goal_tracker
    }

    /// Get a mutable reference to the goal tracker.
    pub fn goal_tracker_mut(&mut self) -> &mut GoalTracker {
        &mut self.goal_tracker
    }

    /// Get a reference to the reflection engine.
    pub fn reflection_engine(&self) -> &ReflectionEngine {
        &self.reflection_engine
    }

    /// Get a mutable reference to the reflection engine.
    pub fn reflection_engine_mut(&mut self) -> &mut ReflectionEngine {
        &mut self.reflection_engine
    }

    /// Compose user message with goal-context and constraints injection.
    pub fn compose_user_message(&self, input: &str) -> String {
        let mut parts = Vec::new();

        let goal_ctx = self.goal_tracker.get_context();
        if !goal_ctx.is_empty() {
            parts.push(format!("<goal-context>\n{}\n</goal-context>", goal_ctx));
        }

        parts.push(input.to_string());
        parts.join("\n\n")
    }
}
