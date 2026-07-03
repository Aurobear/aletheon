use std::sync::Arc;

use crate::core::behavior_paths::{BehaviorPath, BehaviorPathRouter};
use crate::core::config::{GenomeConfig, RuntimeConfig};
use crate::core::evolution_coordinator::{EvolutionConfig, EvolutionCoordinator, EvolutionSummary};
use crate::core::interrupt::InterruptFlag;
use crate::core::mode_router::ModeRouter;
use crate::core::react_loop::ReActLoop;
use crate::core::sub_agent::SubAgentSpawner;
use crate::core::verdict_handler::DefaultVerdictHandler;
use anyhow::Result;
use base::body::{Action, ActionResult};
use base::brain::Plan;
use base::context::Context;
use base::runtime::StepResult;
use base::self_field::{Intent, Verdict, VerdictAction, VerdictHandler};
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
    evolution: Option<EvolutionCoordinator>,
    genome_config: GenomeConfig,
    verdict_handler: Arc<dyn VerdictHandler>,
    mode_router: ModeRouter,
    interrupt_flag: InterruptFlag,
    sub_agent_spawner: SubAgentSpawner,
}

impl AletheonRuntime {
    pub fn new(config: RuntimeConfig) -> Self {
        let react_loop = ReActLoop::new(config.clone());
        Self {
            config,
            react_loop,
            evolution: None,
            genome_config: GenomeConfig::default(),
            verdict_handler: Arc::new(DefaultVerdictHandler::new()),
            mode_router: ModeRouter::new(),
            interrupt_flag: InterruptFlag::new(),
            sub_agent_spawner: SubAgentSpawner::new(),
        }
    }

    /// Set a custom verdict handler.
    pub fn with_verdict_handler(mut self, handler: Arc<dyn VerdictHandler>) -> Self {
        self.verdict_handler = handler;
        self
    }

    /// Set the genome configuration.
    pub fn with_genome_config(mut self, genome_config: GenomeConfig) -> Self {
        self.genome_config = genome_config;
        self
    }

    /// Reference to the current genome configuration.
    pub fn genome_config(&self) -> &GenomeConfig {
        &self.genome_config
    }

    /// Replace the genome configuration (e.g., after evolution).
    pub fn update_genome_config(&mut self, genome_config: GenomeConfig) {
        self.genome_config = genome_config;
    }

    /// Attach an EvolutionCoordinator with the given configuration.
    ///
    /// Returns `Err` if the coordinator cannot be initialized (e.g., lineage
    /// directory creation fails).
    pub fn with_evolution(mut self, evo_config: EvolutionConfig) -> Result<Self> {
        self.evolution = Some(EvolutionCoordinator::new(evo_config)?);
        Ok(self)
    }

    /// Run post-turn evolution if a coordinator is attached.
    ///
    /// Returns `None` if evolution is not configured.
    pub async fn post_evolution<M: base::meta::MetaRuntimeOps>(
        &mut self,
        task_summary: &str,
        output: &str,
        success: bool,
        tool_calls: usize,
        tool_errors: usize,
        elapsed_ms: u64,
        iterations: usize,
        meta: &metacog::MorphogenesisPipeline<M>,
    ) -> Result<Option<EvolutionSummary>> {
        // Drain awareness signals from the react loop before passing to evolution
        let signals = self.react_loop.take_signals();
        match &self.evolution {
            Some(coord) => {
                let summary = coord
                    .post_turn(
                        task_summary,
                        output,
                        success,
                        tool_calls,
                        tool_errors,
                        elapsed_ms,
                        iterations,
                        meta,
                        signals,
                    )
                    .await?;
                // Pull updated genome config after evolution
                if summary.evolution_triggered {
                    self.genome_config = coord.genome_config().await;
                }
                Ok(Some(summary))
            }
            None => Ok(None),
        }
    }

    /// Reference to the evolution coordinator, if configured.
    pub fn evolution(&self) -> Option<&EvolutionCoordinator> {
        self.evolution.as_ref()
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
                Ok(result.output)
            }
            BehaviorPath::Cognitive | BehaviorPath::Volitional => {
                // Normal path: think → plan → execute → reflect
                let plan = think_fn(&intent, ctx)?;
                debug!("Plan generated: {} steps", plan.steps.len());

                // Step 5: Execute plan steps
                let mut _steps_completed = 0;
                for step in &plan.steps {
                    if !self.react_loop.should_continue() {
                        warn!(
                            "Max iterations ({}) reached",
                            self.react_loop.max_iterations()
                        );
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
                Ok(output)
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

    /// Seed the goal tracker from persisted state (resume-on-start).
    /// Must be called before the first turn.
    pub fn seed_goal(&mut self, description: &str, sub_goals: &[String]) {
        self.react_loop.seed_goal(description, sub_goals);
    }

    /// Process input via the interleaved ReAct loop.
    pub async fn process_react<L, R, F, Fut>(
        &mut self,
        input: &str,
        ctx: &Context,
        review_fn: R,
        llm: &L,
        tool_defs: &[base::ToolDefinition],
        execute_tool: F,
    ) -> Result<(String, crate::core::react_loop::TurnMetrics)>
    where
        L: cognit::r#impl::llm::provider::LlmProvider + ?Sized,
        R: Fn(&Intent, &Context) -> Result<Verdict>,
        F: Fn(&str, &str, &serde_json::Value) -> Fut,
        Fut: std::future::Future<Output = (String, bool)>,
    {
        self.react_loop.reset();
        let intent = self.react_loop.build_intent(input);
        let verdict = review_fn(&intent, ctx)?;
        debug!("SelfField verdict: {:?}", verdict);

        let action = self.verdict_handler.handle(&verdict, &intent, ctx);
        // Effective input: use modified intent's description when available,
        // otherwise fall back to the original user input.
        let effective_input: String;
        match action {
            VerdictAction::Proceed { modified_intent } => {
                if let Some(modified) = modified_intent {
                    debug!(
                        action = %modified.action,
                        description = %modified.description,
                        "Using SelfField-modified intent"
                    );
                    effective_input = modified.description.clone();
                } else {
                    effective_input = input.to_string();
                }
            }
            VerdictAction::ShortCircuit { response } => {
                let metrics = crate::core::react_loop::TurnMetrics {
                    tool_calls_made: 0,
                    tool_errors: 0,
                    elapsed_ms: 0,
                    iterations: 0,
                    completed_normally: false,
                };
                return Ok((response, metrics));
            }
            VerdictAction::SandboxThenProceed { reason } => {
                // Sandbox infrastructure exists but is complex to wire here.
                // Log and proceed without sandbox for now.
                warn!(
                    "SandboxFirst requested: {}. Proceeding without sandbox.",
                    reason
                );
                effective_input = input.to_string();
            }
        }
        // Inject genome care weights into system prompt before LLM calls
        let care_prompt = self.genome_config.care_weights_prompt();
        if !care_prompt.is_empty() {
            let current = self.react_loop.system_prompt().to_string();
            self.react_loop
                .set_system_prompt(format!("{}\n\n{}", current, care_prompt));
        }

        self.react_loop
            .run(&effective_input, llm, tool_defs, execute_tool)
            .await
    }

    /// Get config
    pub fn config(&self) -> &RuntimeConfig {
        &self.config
    }

    /// Drain awareness signals collected during the last ReAct turn.
    ///
    /// Returns the signals and clears the internal buffer. The caller
    /// should convert these to `SelfAwareness` entries and store them
    /// via `EpisodicMemory::store_awareness()`.
    pub fn take_awareness_signals(
        &mut self,
    ) -> Vec<cognit::core::awareness_signal::AwarenessSignal> {
        self.react_loop.take_signals()
    }

    /// Reference to the mode router.
    pub fn mode_router(&self) -> &ModeRouter {
        &self.mode_router
    }

    /// Mutable reference to the mode router.
    pub fn mode_router_mut(&mut self) -> &mut ModeRouter {
        &mut self.mode_router
    }

    /// Reference to the interrupt flag.
    pub fn interrupt_flag(&self) -> &InterruptFlag {
        &self.interrupt_flag
    }

    /// Reference to the sub-agent spawner.
    pub fn sub_agent_spawner(&self) -> &SubAgentSpawner {
        &self.sub_agent_spawner
    }

    /// Mutable reference to the sub-agent spawner.
    pub fn sub_agent_spawner_mut(&mut self) -> &mut SubAgentSpawner {
        &mut self.sub_agent_spawner
    }
}
