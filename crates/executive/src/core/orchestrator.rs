use crate::core::config::{ExecutiveConfig, GenomeConfig};
use crate::core::evolution_coordinator::{EvolutionConfig, EvolutionCoordinator, EvolutionSummary};
use crate::core::mode_router::ModeRouter;
use crate::core::sub_agent::SubAgentSpawner;
use anyhow::Result;
use cognit::harness::interrupt::InterruptFlag;
use cognit::harness::linear::ReActLoop;
use fabric::body::{Action, ActionResult};
use fabric::context::Context;
use fabric::runtime::StepResult;
use fabric::self_field::{Intent, Verdict};
use fabric::Clock;
use std::sync::Arc;

/// Top-level Aletheon runtime — decomposes Engine::run_turn() into 6 layers
///
/// Replaces the Engine god-object. Each layer handles its own concern:
/// - SelfField: policy review
/// - CognitCore: reasoning + planning
/// - BodyRuntime: tool execution
/// - Memory: state persistence
/// - EventBus: event routing
/// - Runtime: orchestration (this struct)
pub struct AletheonExecutive {
    config: ExecutiveConfig,
    react_loop: ReActLoop,
    evolution: Option<EvolutionCoordinator>,
    genome_config: GenomeConfig,
    mode_router: ModeRouter,
    interrupt_flag: InterruptFlag,
    sub_agent_spawner: SubAgentSpawner,
}

impl AletheonExecutive {
    pub fn new(config: ExecutiveConfig) -> Self {
        let react_loop = crate::service::harness_factory::build_configured_react_loop(&config);
        Self {
            config,
            react_loop,
            evolution: None,
            genome_config: GenomeConfig::default(),
            mode_router: ModeRouter::new(),
            interrupt_flag: InterruptFlag::new(),
            sub_agent_spawner: SubAgentSpawner::new(),
        }
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
    pub fn with_evolution(
        mut self,
        evo_config: EvolutionConfig,
        clock: Arc<dyn Clock>,
    ) -> Result<Self> {
        self.evolution = Some(EvolutionCoordinator::new(evo_config, clock)?);
        Ok(self)
    }

    /// Run post-turn evolution if a coordinator is attached.
    ///
    /// Returns `None` if evolution is not configured.
    pub async fn post_evolution<M: fabric::meta::MetaRuntimeOps>(
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

    /// Get config
    pub fn config(&self) -> &ExecutiveConfig {
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
