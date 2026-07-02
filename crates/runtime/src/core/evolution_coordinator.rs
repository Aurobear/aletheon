//! EvolutionCoordinator — post-turn self-evolution orchestrator.
//!
//! After each ReAct turn, the coordinator:
//! 1. Reflects on the execution outcome (Reflector)
//! 2. Accumulates reflection entries in a sliding window
//! 3. Periodically triggers the morphogenesis pipeline
//! 4. Records successful migrations to the lineage tracker

use crate::core::config::GenomeConfig;
use base::brain::{ExecutionResult, ReflectionEntry, ReflectionTrigger};
use base::dasein::Stimmung;
use base::meta::MetaRuntimeOps;
use base::self_field::SelfAwareness;
use cognit::core::awareness_signal::{signals_to_awareness, AwarenessSignal};
use cognit::core::reflector::Reflector;
use metacog::r#impl::meta_runtime::lineage::LineageTracker;
use metacog::r#impl::morphogenesis::mutation_intent::MutationIntentGenerator;
use metacog::r#impl::morphogenesis::pipeline::{MorphogenesisPipeline, PipelineResult};
use anyhow::Result;
use std::path::PathBuf;
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Configuration for when to trigger the evolution pipeline.
#[derive(Debug, Clone)]
pub struct EvolutionConfig {
    /// Master switch. When false, the whole loop is inert (default).
    /// HIGH-risk autonomy -- OFF unless explicitly enabled by the operator.
    pub enabled: bool,
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
            enabled: false, // HIGH-risk autonomy: OFF unless explicitly enabled
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
    /// Awareness entries to store — (action, SelfAwareness) pairs.
    /// The caller stores these via EpisodicMemory::store_awareness().
    pub awareness_entries: Vec<(String, SelfAwareness)>,
}

/// Source of negativity that can trigger evolution.
#[derive(Debug, Clone)]
pub enum NegativitySource {
    /// Angst — existential confrontation (Heidegger's Angst)
    Angst(String),
    /// Meaning crisis — profound boredom or meaninglessness
    MeaningCrisis,
    /// World disclosed negatively — dejection (Geknickt)
    WorldDisclosed(String),
}

/// A negativity signal derived from Stimmung for evolution triggering.
#[derive(Debug, Clone)]
pub struct NegativitySignal {
    /// Source of the negativity
    pub source: NegativitySource,
    /// Depth of negativity (0.0 to 1.0) — affects evolution intensity
    pub depth: f64,
    /// Whether this signal should force evolution even if turn count hasn't been reached
    pub should_force_evolution: bool,
    /// Human-readable description
    pub description: String,
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
    ///
    /// If `awareness_signals` is provided, they are converted to
    /// `SelfAwareness` entries and returned in the summary for storage.
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
        awareness_signals: Vec<AwarenessSignal>,
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

        // HIGH-risk autonomy gate. TODO(Tier 2a): also require PermissionManager approval.
        if !self.config.enabled {
            return Ok(EvolutionSummary {
                reflected: false,
                reflection_id: None,
                evolution_triggered: false,
                pipeline_results: Vec::new(),
                lineage_entries_added: 0,
                awareness_entries: signals_to_awareness(&awareness_signals),
            });
        }

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

        // Convert awareness signals to SelfAwareness entries for storage
        let awareness_entries = signals_to_awareness(&awareness_signals);

        Ok(EvolutionSummary {
            reflected: true,
            reflection_id: Some(reflection_id),
            evolution_triggered: triggered,
            pipeline_results,
            lineage_entries_added: lineage_added,
            awareness_entries,
        })
    }

    /// Post-turn with Stimmung awareness — Angst signals trigger deeper evolution.
    ///
    /// Heidegger's negativity: Angst confronts Dasein with its own being-toward-death,
    /// triggering deeper self-questioning and evolution. This method extends
    /// `post_turn` by:
    /// 1. Using Angst signals to force evolution regardless of turn count
    /// 2. Adjusting the evolution depth based on the negativity source
    /// 3. Returning the mood-derived negativity signal in the summary
    ///
    /// This is additive — does not modify `post_turn`.
    pub async fn post_turn_with_stimmung<M: MetaRuntimeOps>(
        &self,
        task_summary: &str,
        output: &str,
        success: bool,
        tool_calls: usize,
        tool_errors: usize,
        elapsed_ms: u64,
        iterations: usize,
        meta: &MorphogenesisPipeline<M>,
        awareness_signals: Vec<AwarenessSignal>,
        mood: &Stimmung,
    ) -> Result<(EvolutionSummary, Option<NegativitySignal>)> {
        // Check if Stimmung triggers forced evolution
        let negativity = Self::negativity_from_stimmung(mood);

        // Run normal post-turn
        let mut summary = self
            .post_turn(
                task_summary,
                output,
                success,
                tool_calls,
                tool_errors,
                elapsed_ms,
                iterations,
                meta,
                awareness_signals,
            )
            .await?;

        // If Angst signals present and evolution wasn't already triggered,
        // force a deeper evolution pass
        if let Some(ref signal) = negativity {
            if !summary.evolution_triggered && signal.should_force_evolution {
                tracing::info!(
                    "Stimmung-driven evolution triggered by {:?} (depth={})",
                    signal.source,
                    signal.depth
                );
                let (triggered, pipeline_results, lineage_added) =
                    self.run_evolution(meta).await?;
                summary.evolution_triggered = triggered;
                summary.pipeline_results = pipeline_results;
                summary.lineage_entries_added += lineage_added;
            }
        }

        Ok((summary, negativity))
    }

    /// Derive a negativity signal from the current Stimmung.
    ///
    /// Heidegger: Angst is not a psychological state but an ontological
    /// disclosure — it reveals Dasein's own being. Deep Langeweile
    /// (profound boredom) similarly confronts meaninglessness.
    pub fn negativity_from_stimmung(mood: &Stimmung) -> Option<NegativitySignal> {
        match mood {
            Stimmung::Angst { facing } => Some(NegativitySignal {
                source: NegativitySource::Angst(format!("{:?}", facing)),
                depth: match facing {
                    base::dasein::AngstSource::Nothingness => 1.0,
                    base::dasein::AngstSource::Finitude => 0.9,
                    base::dasein::AngstSource::Freedom => 0.8,
                    base::dasein::AngstSource::Responsibility => 0.7,
                },
                should_force_evolution: true,
                description: format!("Angst facing {:?} — existential negativity", facing),
            }),
            Stimmung::Langeweile {
                depth: base::dasein::BoredomDepth::Deep,
            } => Some(NegativitySignal {
                source: NegativitySource::MeaningCrisis,
                depth: 0.6,
                should_force_evolution: true,
                description: "Deep boredom — confronting meaninglessness".to_string(),
            }),
            Stimmung::Geknickt { because } => Some(NegativitySignal {
                source: NegativitySource::WorldDisclosed(because.clone()),
                depth: 0.4,
                should_force_evolution: false,
                description: format!("Dejected — world disclosed negatively: {}", because),
            }),
            _ => None,
        }
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
    async fn apply_care_mutation(&self, intent: &base::self_field::MutationIntent) {
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
