//! EvolutionCoordinator — post-turn self-evolution orchestrator.
//!
//! After each ReAct turn, the coordinator:
//! 1. Reflects on the execution outcome (Reflector)
//! 2. Accumulates reflection entries in a sliding window
//! 3. Periodically triggers the morphogenesis pipeline
//! 4. Records successful migrations to the lineage tracker

use crate::core::config::GenomeConfig;
use aletheon_abi::brain::{ExecutionResult, ReflectionEntry, ReflectionTrigger};
use aletheon_abi::meta::MetaRuntimeOps;
use aletheon_brain::core::reflector::Reflector;
use aletheon_meta::r#impl::meta_runtime::lineage::LineageTracker;
use aletheon_meta::r#impl::morphogenesis::mutation_intent::MutationIntentGenerator;
use aletheon_meta::r#impl::morphogenesis::pipeline::{MorphogenesisPipeline, PipelineResult};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Configuration for when to trigger the evolution pipeline.
#[derive(Debug, Clone)]
pub struct EvolutionConfig {
    /// Trigger evolution every N turns (0 = disabled).
    pub trigger_every_n_turns: usize,
    /// Also trigger evolution after any failed turn.
    pub trigger_on_failure: bool,
    /// Maximum number of recent reflections to keep.
    pub window_size: usize,
    /// Directory for lineage persistence.
    pub lineage_dir: PathBuf,
}

impl Default for EvolutionConfig {
    fn default() -> Self {
        Self {
            trigger_every_n_turns: 5,
            trigger_on_failure: true,
            window_size: 20,
            lineage_dir: PathBuf::from("/var/lib/aletheon/lineage"),
        }
    }
}

/// Summary of what the EvolutionCoordinator did after a turn.
#[derive(Debug)]
pub struct EvolutionSummary {
    pub reflected: bool,
    pub reflection_id: Option<String>,
    pub evolution_triggered: bool,
    pub pipeline_results: Vec<PipelineResult>,
    pub lineage_entries_added: usize,
}

/// Orchestrates post-turn self-evolution.
///
/// Not `Clone` because it owns mutable state (Arc<Mutex<...>>).
/// Thread-safe via internal Arc + Mutex.
pub struct EvolutionCoordinator {
    config: EvolutionConfig,
    reflector: Reflector,
    intent_generator: MutationIntentGenerator,
    lineage: LineageTracker,
    recent_reflections: Arc<Mutex<Vec<ReflectionEntry>>>,
    turn_counter: Arc<Mutex<usize>>,
    genome_config: Arc<Mutex<GenomeConfig>>,
}

impl EvolutionCoordinator {
    pub fn new(config: EvolutionConfig) -> Result<Self> {
        let lineage = LineageTracker::with_path(config.lineage_dir.join("lineage.jsonl"))?;
        Ok(Self {
            config,
            reflector: Reflector::new(),
            intent_generator: MutationIntentGenerator::new(),
            lineage,
            recent_reflections: Arc::new(Mutex::new(Vec::new())),
            turn_counter: Arc::new(Mutex::new(0)),
            genome_config: Arc::new(Mutex::new(GenomeConfig::default())),
        })
    }

    /// Set the genome configuration.
    pub fn with_genome_config(mut self, config: GenomeConfig) -> Self {
        self.genome_config = Arc::new(Mutex::new(config));
        self
    }

    /// Clone of the current genome configuration.
    pub async fn genome_config(&self) -> GenomeConfig {
        self.genome_config.lock().await.clone()
    }

    /// Called after each ReAct turn. Reflects on the outcome and
    /// optionally triggers the evolution pipeline.
    pub async fn post_turn<M: MetaRuntimeOps>(
        &self,
        task_summary: &str,
        output: &str,
        success: bool,
        tool_calls: usize,
        tool_errors: usize,
        elapsed_ms: u64,
        _iterations: usize,
        meta: &MorphogenesisPipeline<M>,
    ) -> Result<EvolutionSummary> {
        // Build an ExecutionResult from turn metrics
        let exec = ExecutionResult {
            plan_id: Uuid::new_v4(),
            success,
            steps_completed: tool_calls.saturating_sub(tool_errors),
            steps_total: tool_calls,
            output: output.to_string(),
            error: if tool_errors > 0 {
                Some(format!("{tool_errors} tool errors"))
            } else {
                None
            },
            elapsed_ms,
        };

        // Reflect on the turn
        let trigger = if success {
            ReflectionTrigger::TaskComplete
        } else {
            ReflectionTrigger::Impasse
        };
        let entry = self.reflector.reflect_entry(task_summary, trigger, &exec);
        let reflection_id = entry.id.clone();

        // Add to sliding window
        {
            let mut window = self.recent_reflections.lock().await;
            window.push(entry);
            if window.len() > self.config.window_size {
                window.remove(0);
            }
        }

        // Check if we should trigger evolution
        let should_trigger = {
            let mut counter = self.turn_counter.lock().await;
            *counter += 1;
            let n = self.config.trigger_every_n_turns;
            let on_fail = self.config.trigger_on_failure && !success;
            (n > 0 && *counter % n == 0) || on_fail
        };

        let (triggered, pipeline_results, lineage_added) = if should_trigger {
            self.run_evolution(meta).await?
        } else {
            (false, Vec::new(), 0)
        };

        Ok(EvolutionSummary {
            reflected: true,
            reflection_id: Some(reflection_id),
            evolution_triggered: triggered,
            pipeline_results,
            lineage_entries_added: lineage_added,
        })
    }

    /// Run the evolution pipeline: generate mutation intents from
    /// recent reflections, execute each through the morphogenesis pipeline,
    /// and record successful migrations to lineage.
    async fn run_evolution<M: MetaRuntimeOps>(
        &self,
        meta: &MorphogenesisPipeline<M>,
    ) -> Result<(bool, Vec<PipelineResult>, usize)> {
        let intents = self
            .intent_generator
            .from_reflections(&self.recent_reflections.lock().await)
            .await;
        if intents.is_empty() {
            return Ok((false, Vec::new(), 0));
        }

        let mut results = Vec::new();
        let mut lineage_count = 0;
        for intent in &intents {
            let result = meta.run(intent).await?;
            if result.success {
                lineage_count += 1;
                if let Some(ref migration) = result.migration {
                    self.lineage.record(
                        &migration.to_version,
                        Some(&migration.from_version),
                        &result.message,
                    );
                }
                // Update care weights from the mutation intent that was applied.
                self.apply_care_mutation(intent).await;
            }
            results.push(result);
        }

        Ok((true, results, lineage_count))
    }

    /// Apply a care-related mutation intent to the tracked genome config.
    ///
    /// Only processes intents targeting "care.priorities" with adjust_weight / increase_weight
    /// actions. Other intent targets are ignored.
    async fn apply_care_mutation(&self, intent: &aletheon_abi::self_field::MutationIntent) {
        if intent.target != "care.priorities" {
            return;
        }
        let topic = intent.change.get("topic").and_then(|v| v.as_str());
        let delta = intent
            .change
            .get("delta")
            .or_else(|| intent.change.get("weight_delta"))
            .and_then(|v| v.as_f64());
        let (Some(topic), Some(delta)) = (topic, delta) else {
            return;
        };
        let mut gc = self.genome_config.lock().await;
        let entry = gc
            .care_weights
            .entry(topic.to_string())
            .or_insert(0.0);
        *entry = (*entry + delta).clamp(0.0, 2.0);
        tracing::debug!(
            "Evolution adjusted care weight: {} += {:.3} -> {:.3}",
            topic,
            delta,
            *entry
        );
    }

    /// Current turn count.
    pub async fn turn_count(&self) -> usize {
        *self.turn_counter.lock().await
    }

    /// Snapshot of recent reflections.
    pub async fn recent_reflections(&self) -> Vec<ReflectionEntry> {
        self.recent_reflections.lock().await.clone()
    }

    /// Reference to the lineage tracker.
    pub fn lineage(&self) -> &LineageTracker {
        &self.lineage
    }
}
