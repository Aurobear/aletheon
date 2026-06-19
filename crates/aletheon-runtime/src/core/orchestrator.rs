use anyhow::Result;
use aletheon_abi::context::Context;
use aletheon_abi::self_field::{Intent, Verdict};
use aletheon_abi::body::{Action, ActionResult};
use aletheon_abi::brain::Plan;
use aletheon_abi::runtime::StepResult;
use crate::core::config::RuntimeConfig;
use crate::core::react_loop::ReActLoop;
use crate::core::behavior_paths::{BehaviorPathRouter, BehaviorPath};
use tracing::{debug, warn};

/// Top-level Aletheon runtime — decomposes Engine::run_turn() into 6 layers
///
/// Replaces the Engine god-object. Each layer handles its own concern:
/// - SelfField: policy review
/// - BrainCore: reasoning + planning
/// - BodyRuntime: tool execution
/// - Memory: state persistence
/// - EventBus: event routing
/// - Runtime: orchestration (this struct)
pub struct AletheonRuntime {
    config: RuntimeConfig,
    react_loop: ReActLoop,
    // Subsystem references would be injected here in production
    // For now, we define the orchestration logic that coordinates them
}

impl AletheonRuntime {
    pub fn new(config: RuntimeConfig) -> Self {
        let react_loop = ReActLoop::new(config.clone());
        Self {
            config,
            react_loop,
        }
    }

    /// Process a user input through the full Aletheon pipeline.
    /// This replaces Engine::run_turn().
    ///
    /// Flow:
    /// 1. Build Intent from input
    /// 2. SelfField.review(intent) → Verdict
    /// 3. Select behavior path based on Verdict
    /// 4. If Cognitive/Volitional: BrainCore.think(intent) → Plan
    /// 5. Execute Plan steps via BodyRuntime
    /// 6. BrainCore.reflect(execution) → Reflection
    /// 7. BrainCore.learn(experience) → LearnedRules
    /// 8. EventBus.publish(events)
    pub async fn process<F, G, H>(
        &mut self,
        input: &str,
        ctx: &Context,
        review_fn: F,
        think_fn: G,
        execute_fn: H,
    ) -> Result<String>
    where
        F: Fn(&Intent, &Context) -> Result<Verdict>,
        G: Fn(&Intent, &Context) -> Result<Plan>,
        H: Fn(&Action, &Context) -> Result<ActionResult>,
    {
        self.react_loop.reset();
        let mut all_output = Vec::new();

        // Step 1: Build Intent
        let intent = self.react_loop.build_intent(input);
        debug!("Processing intent: {}", intent.description);

        // Step 2: SelfField review
        let verdict = review_fn(&intent, ctx)?;
        debug!("SelfField verdict: {:?}", verdict);

        // Step 3: Select behavior path
        let path = BehaviorPathRouter::select_path(&intent, &verdict);
        debug!("Selected path: {:?}", path);

        match path {
            BehaviorPath::Reflex => {
                // Emergency: direct execution, no Brain involved
                warn!("Reflex path: executing directly without BrainCore");
                let action = Action {
                    name: intent.action.clone(),
                    parameters: intent.parameters.clone(),
                    requires_sandbox: false,
                    timeout: None,
                };
                let result = execute_fn(&action, ctx)?;
                return Ok(result.output);
            }
            BehaviorPath::Cognitive | BehaviorPath::Volitional => {
                // Normal path: think → plan → execute → reflect
                let plan = think_fn(&intent, ctx)?;
                debug!("Plan generated: {} steps", plan.steps.len());

                // Step 5: Execute plan steps
                let mut _steps_completed = 0;
                for step in &plan.steps {
                    if !self.react_loop.should_continue() {
                        warn!("Max iterations ({}) reached", self.react_loop.max_iterations());
                        break;
                    }

                    debug!("Executing step: {}", step.action.name);
                    match execute_fn(&step.action, ctx) {
                        Ok(result) => {
                            _steps_completed += 1;
                            all_output.push(result.output.clone());

                            if !result.success {
                                warn!("Step failed: {:?}", result.error);
                                // Try rollback if available
                                if let Some(rollback) = &step.rollback_action {
                                    debug!("Attempting rollback: {}", rollback.name);
                                    let _ = execute_fn(rollback, ctx);
                                }
                                break;
                            }
                        }
                        Err(e) => {
                            warn!("Step execution error: {}", e);
                            break;
                        }
                    }

                    self.react_loop.advance();
                }

                // Step 6-8: Reflection and learning happen at the caller level
                // (BrainCore.reflect() and BrainCore.learn() are called externally)

                let output = all_output.join("\n");
                return Ok(output);
            }
        }
    }

    /// Process a single step (for streaming/incremental execution)
    pub async fn step<F, H>(
        &mut self,
        _ctx: &Context,
        _review_fn: &F,
        _execute_fn: &H,
    ) -> Result<StepResult>
    where
        F: Fn(&Intent, &Context) -> Result<Verdict>,
        H: Fn(&Action, &Context) -> Result<ActionResult>,
    {
        if !self.react_loop.should_continue() {
            return Ok(StepResult {
                completed: true,
                output: Some("Max iterations reached".to_string()),
                tool_calls: 0,
                continue_reason: None,
            });
        }

        self.react_loop.advance();

        Ok(StepResult {
            completed: false,
            output: None,
            tool_calls: 0,
            continue_reason: Some("step completed".to_string()),
        })
    }

    /// Get current iteration count
    pub fn iteration(&self) -> usize {
        self.react_loop.iteration()
    }

    /// Process input via the interleaved ReAct loop.
    pub async fn process_react<L, R, F, Fut>(
        &mut self,
        input: &str,
        ctx: &Context,
        review_fn: R,
        llm: &L,
        tool_defs: &[aletheon_abi::ToolDefinition],
        execute_tool: F,
    ) -> Result<String>
    where
        L: aletheon_brain::r#impl::llm::provider::LlmProvider + ?Sized,
        R: Fn(&Intent, &Context) -> Result<Verdict>,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = (String, bool)>,
    {
        self.react_loop.reset();
        let intent = self.react_loop.build_intent(input);
        let verdict = review_fn(&intent, ctx)?;
        debug!("SelfField verdict: {:?}", verdict);
        if let Verdict::Deny { reason } = verdict {
            return Ok(format!("Denied by SelfField: {}", reason));
        }
        self.react_loop.run(input, llm, tool_defs, execute_tool).await
    }

    /// Get config
    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }
}
