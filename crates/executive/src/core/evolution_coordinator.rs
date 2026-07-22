//! EvolutionCoordinator — post-turn self-evolution orchestrator.
//!
//! After each ReAct turn, the coordinator:
//! 1. Reflects on the execution outcome (Reflector)
//! 2. Accumulates reflection entries in a sliding window
//! 3. Periodically triggers the morphogenesis pipeline
//! 4. Records successful migrations to the lineage tracker

use crate::composition::config::GenomeConfig;
use anyhow::Result;
use cognit::core::awareness_signal::{signals_to_awareness, AwarenessSignal};
use cognit::core::reflector::Reflector;
use fabric::cognit::{ExecutionResult, ReflectionEntry, ReflectionTrigger};
use fabric::dasein::Stimmung;
use fabric::self_field::SelfAwareness;
use fabric::Clock;
use metacog::r#impl::morphogenesis::mutation_intent::MutationIntentGenerator;
use metacog::{MetacogService, VerificationReceipt, VerifyMutation};
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tokio::sync::Mutex;
use uuid::Uuid;

/// Configuration for when to trigger the evolution pipeline.
#[derive(Debug, Clone)]
pub struct EvolutionConfig {
    /// Master switch. When false, the whole loop is inert (default).
    /// HIGH-risk autonomy -- OFF unless explicitly enabled by the operator.
    pub enabled: bool,
    /// PermissionManager approval gate. Must be true for evolution to proceed.
    /// Separate from `enabled` so operators can disable evolution entirely
    /// while keeping the permission configured.
    pub evolution_permitted: bool,
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
            enabled: false,             // HIGH-risk autonomy: OFF unless explicitly enabled
            evolution_permitted: false, // separate PermissionManager gate, OFF by default
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
    pub verification_receipts: Vec<VerificationReceipt>,
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
    recent_reflections: Arc<Mutex<Vec<ReflectionEntry>>>,
    turn_counter: Arc<AtomicUsize>,
    genome_config: Arc<Mutex<GenomeConfig>>,
}

impl EvolutionCoordinator {
    pub fn new(config: EvolutionConfig, clock: Arc<dyn Clock>) -> Result<Self> {
        std::fs::create_dir_all(&config.lineage_dir)?;
        Ok(Self {
            config,
            reflector: Reflector::new(clock.clone()),
            intent_generator: MutationIntentGenerator::new(),
            recent_reflections: Arc::new(Mutex::new(Vec::new())),
            turn_counter: Arc::new(AtomicUsize::new(0)),
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
    pub async fn post_turn(
        &self,
        task_summary: &str,
        output: &str,
        success: bool,
        tool_calls: usize,
        tool_errors: usize,
        elapsed_ms: u64,
        _iterations: usize,
        meta: &dyn MetacogService,
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

        // HIGH-risk autonomy gate. Requires both config.enabled AND PermissionManager approval.
        if !self.config.enabled || !self.config.evolution_permitted {
            return Ok(EvolutionSummary {
                reflected: false,
                reflection_id: None,
                evolution_triggered: false,
                verification_receipts: Vec::new(),
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
            let counter = self.turn_counter.fetch_add(1, Ordering::Relaxed) + 1;
            let n = self.config.trigger_every_n_turns;
            let on_fail = self.config.trigger_on_failure && !success;
            (n > 0 && counter % n == 0) || on_fail
        };

        let (triggered, verification_receipts) = if should_trigger {
            self.run_evolution(meta).await?
        } else {
            (false, Vec::new())
        };

        // Convert awareness signals to SelfAwareness entries for storage
        let awareness_entries = signals_to_awareness(&awareness_signals);

        Ok(EvolutionSummary {
            reflected: true,
            reflection_id: Some(reflection_id),
            evolution_triggered: triggered,
            verification_receipts,
            lineage_entries_added: 0,
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
    pub async fn post_turn_with_stimmung(
        &self,
        task_summary: &str,
        output: &str,
        success: bool,
        tool_calls: usize,
        tool_errors: usize,
        elapsed_ms: u64,
        iterations: usize,
        meta: &dyn MetacogService,
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
                let (triggered, verification_receipts) = self.run_evolution(meta).await?;
                summary.evolution_triggered = triggered;
                summary.verification_receipts = verification_receipts;
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
                    fabric::dasein::AngstSource::Nothingness => 1.0,
                    fabric::dasein::AngstSource::Finitude => 0.9,
                    fabric::dasein::AngstSource::Freedom => 0.8,
                    fabric::dasein::AngstSource::Responsibility => 0.7,
                },
                should_force_evolution: true,
                description: format!("Angst facing {:?} — existential negativity", facing),
            }),
            Stimmung::Langeweile {
                depth: fabric::dasein::BoredomDepth::Deep,
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

    /// Generate mutation intents from recent reflections and submit them to
    /// Metacog verification. Applying or rolling back a verified mutation
    /// remains a separate governed operation owned by Metacog.
    async fn run_evolution(
        &self,
        meta: &dyn MetacogService,
    ) -> Result<(bool, Vec<VerificationReceipt>)> {
        let intents = self
            .intent_generator
            .from_reflections(&self.recent_reflections.lock().await)
            .await;
        if intents.is_empty() {
            return Ok((false, Vec::new()));
        }

        let mut results = Vec::new();
        for intent in &intents {
            results.push(
                meta.verify(VerifyMutation {
                    mutation_id: Uuid::new_v4(),
                    intent: intent.clone(),
                })
                .await?,
            );
        }

        Ok((true, results))
    }

    /// Current turn count.
    pub async fn turn_count(&self) -> usize {
        self.turn_counter.load(Ordering::Relaxed)
    }

    /// Snapshot of recent reflections.
    pub async fn recent_reflections(&self) -> Vec<ReflectionEntry> {
        self.recent_reflections.lock().await.clone()
    }
}
