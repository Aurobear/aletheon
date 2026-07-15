//! DaseinModule — the existential substrate of SelfField.
//!
//! Philosophy: Heidegger (Dasein/Sorge/Temporality),
//! Husserl (inner time consciousness),
//! Sartre (negativity/pour-soi),
//! Merleau-Ponty (embodiment).

pub mod bewandtnis;
pub mod care_structure;
pub mod context_injection;
pub mod event_bridge;
pub mod negativity;
pub mod persistence;
pub mod self_model;
pub mod sorge;
pub mod temporality;
pub mod types;

pub use fabric::dasein::*;

use parking_lot::RwLock;
use std::sync::Arc;
use tokio::sync::mpsc;

use bewandtnis::Bewandtnisganzheit;
use care_structure::CareStructure;
use context_injection::format_dasein_context;
pub use event_bridge::DaseinEventBridge;
use negativity::NegativityEngine;
use self_model::MutableSelfModel;
use sorge::{SorgeLoop, SorgeTimer, SystemSorgeTimer};
use temporality::TemporalStream;

/// DaseinModule — the existential substrate of SelfField.
///
/// Not four separate modules, but four faces of one unified existence:
/// - Temporality: the lived flow of experience
/// - World: the meaningful involvement network
/// - Self: the constantly negated and rebuilt self-model
/// - Care: the unified structure of projection + thrownness + fallenness
pub struct DaseinModule {
    // Core state
    mood: Arc<RwLock<Stimmung>>,
    temporality: Arc<TemporalStream>,
    world: Arc<Bewandtnisganzheit>,
    self_model: Arc<MutableSelfModel>,
    care: Arc<CareStructure>,
    negativity: Arc<NegativityEngine>,

    // Runtime
    sorge: SorgeLoop,
    event_tx: mpsc::Sender<DaseinEvent>,
    #[allow(dead_code)]
    clock: Arc<dyn fabric::Clock>,
}

#[derive(Debug, Clone)]
pub struct DaseinRuntimeConfig {
    pub retention_depth: usize,
    pub decay_rate: f64,
    pub event_buffer: usize,
}

impl Default for DaseinRuntimeConfig {
    fn default() -> Self {
        Self {
            retention_depth: 50,
            decay_rate: 0.8,
            event_buffer: 256,
        }
    }
}

impl DaseinRuntimeConfig {
    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            self.retention_depth > 0,
            "Dasein retention depth must be nonzero"
        );
        anyhow::ensure!(
            self.decay_rate.is_finite() && (0.0..=1.0).contains(&self.decay_rate),
            "Dasein decay rate must be between 0 and 1"
        );
        anyhow::ensure!(self.event_buffer > 0, "Dasein event buffer must be nonzero");
        Ok(())
    }
}

impl DaseinModule {
    pub fn new(clock: Arc<dyn fabric::Clock>) -> (Self, mpsc::Sender<DaseinEvent>) {
        Self::with_runtime(
            clock,
            Arc::new(SystemSorgeTimer),
            DaseinRuntimeConfig::default(),
        )
        .expect("default Dasein runtime config is valid")
    }

    pub fn with_runtime(
        clock: Arc<dyn fabric::Clock>,
        timer: Arc<dyn SorgeTimer>,
        config: DaseinRuntimeConfig,
    ) -> anyhow::Result<(Self, mpsc::Sender<DaseinEvent>)> {
        config.validate()?;
        let (sorge, event_tx) = SorgeLoop::new(config.event_buffer, clock.clone(), timer);
        let external_tx = event_tx.clone();

        let temporality = Arc::new(TemporalStream::new(
            config.retention_depth,
            config.decay_rate,
        ));
        let world = Arc::new(Bewandtnisganzheit::new());
        let self_model = Arc::new(MutableSelfModel::new());
        let care = Arc::new(CareStructure::new());
        let negativity = Arc::new(NegativityEngine::default());

        let module = Self {
            mood: Arc::new(RwLock::new(Stimmung::Gelassenheit)),
            temporality,
            world,
            self_model,
            care,
            negativity,
            sorge,
            event_tx,
            clock,
        };

        Ok((module, external_tx))
    }

    /// Start the sorge loop.
    pub fn start_sorge_loop(&self) -> bool {
        self.sorge.start(
            self.temporality.clone(),
            self.world.clone(),
            self.self_model.clone(),
            self.care.clone(),
            self.negativity.clone(),
            self.mood.clone(),
        )
    }

    /// Stop the sorge loop.
    pub async fn stop_sorge_loop(&self) {
        self.sorge.stop().await;
    }

    /// Check if the sorge loop is alive.
    pub fn is_alive(&self) -> bool {
        self.sorge.is_running()
    }

    /// Get current mood.
    pub fn mood(&self) -> Stimmung {
        self.mood.read().clone()
    }

    /// Get raw mood RwLock for persistence.
    pub fn mood_raw(&self) -> &parking_lot::RwLock<Stimmung> {
        &self.mood
    }

    /// Get the event sender for external events.
    pub fn event_sender(&self) -> mpsc::Sender<DaseinEvent> {
        self.event_tx.clone()
    }

    /// Generate context injection for LLM.
    pub fn to_context_injection(&self) -> DaseinContext {
        DaseinContext {
            mood: self.mood.read().clone(),
            temporality: self.temporality.to_snapshot(),
            world: self.world.to_snapshot(),
            self_model: self.self_model.to_snapshot(),
            care: self.care.to_snapshot(),
        }
    }

    /// Format context injection as string for prompt.
    pub fn format_context(&self) -> String {
        let ctx = self.to_context_injection();
        format_dasein_context(&ctx)
    }

    /// Fast-path mood update based on turn text content.
    /// Uses keyword matching for quick transitions without deep analysis.
    pub fn quick_mood_update(&self, turn_text: &str) -> Stimmung {
        let mut mood = self.mood.write();
        let new_mood = if turn_text.contains("error") || turn_text.contains("failed") {
            Stimmung::Geknickt {
                because: "turn had errors".to_string(),
            }
        } else if turn_text.contains("success") || turn_text.contains("completed") {
            Stimmung::Gelaunt {
                toward: "successful completion".to_string(),
            }
        } else {
            mood.clone()
        };
        let changed = std::mem::discriminant(&*mood) != std::mem::discriminant(&new_mood);
        if changed {
            let old = mood.clone();
            *mood = new_mood.clone();
            let _ = self.event_tx.try_send(DaseinEvent::MoodShift {
                from: old,
                to: new_mood.clone(),
                reason: "quick_update_after_turn".to_string(),
            });
        }
        new_mood
    }

    /// Access internal components for integration tests.
    pub fn temporality(&self) -> &TemporalStream {
        &self.temporality
    }

    pub fn world(&self) -> &Bewandtnisganzheit {
        &self.world
    }

    pub fn self_model(&self) -> &MutableSelfModel {
        &self.self_model
    }

    pub fn care(&self) -> &CareStructure {
        &self.care
    }
}

impl Default for DaseinModule {
    fn default() -> Self {
        Self::new(Arc::new(aletheon_kernel::chronos::SystemClock::new())).0
    }
}

#[async_trait::async_trait]
impl DaseinOps for DaseinModule {
    fn mood(&self) -> Stimmung {
        self.mood()
    }

    fn temporality_snapshot(&self) -> TemporalStreamSnapshot {
        self.temporality.to_snapshot()
    }

    fn world_snapshot(&self) -> BewandtnisSnapshot {
        self.world.to_snapshot()
    }

    fn self_model_snapshot(&self) -> SelfModelSnapshot {
        self.self_model.to_snapshot()
    }

    fn care_snapshot(&self) -> CareStructureSnapshot {
        self.care.to_snapshot()
    }

    fn to_context_injection(&self) -> DaseinContext {
        self.to_context_injection()
    }

    async fn handle_event(&self, event: DaseinEvent) -> anyhow::Result<()> {
        self.event_tx
            .send(event)
            .await
            .map_err(|e| anyhow::anyhow!("failed to send event: {}", e))
    }

    async fn start_sorge_loop(&self) -> anyhow::Result<()> {
        DaseinModule::start_sorge_loop(self);
        Ok(())
    }

    async fn stop_sorge_loop(&self) -> anyhow::Result<()> {
        DaseinModule::stop_sorge_loop(self).await;
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.is_alive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(aletheon_kernel::chronos::TestClock::default())
    }

    #[test]
    fn test_dasein_module_creation() {
        let (module, _tx) = DaseinModule::new(test_clock());
        assert_eq!(module.mood(), Stimmung::Gelassenheit);
        assert!(!module.is_alive()); // sorge not started yet
    }

    #[test]
    fn test_context_injection() {
        let (module, _tx) = DaseinModule::new(test_clock());

        // Add some state
        module.self_model().assert(self_model::SelfAssertion {
            content: "a learning system".to_string(),
            source: self_model::AssertionSource::Chosen,
            stability: 0.9,
            since: types::TemporalPosition(0),
            bewandtnis: vec![],
        });

        let ctx = module.to_context_injection();
        assert_eq!(ctx.self_model.current_assertions.len(), 1);
        assert_eq!(
            ctx.self_model.current_assertions[0].content,
            "a learning system"
        );
    }

    #[test]
    fn test_format_context_not_empty() {
        let (module, _tx) = DaseinModule::new(test_clock());
        let formatted = module.format_context();
        assert!(!formatted.is_empty());
        assert!(formatted.contains("Existential State"));
    }

    #[test]
    fn test_default_creates_module() {
        let module = DaseinModule::default();
        assert_eq!(module.mood(), Stimmung::Gelassenheit);
    }

    #[test]
    fn test_snapshots() {
        let (module, _tx) = DaseinModule::new(test_clock());

        let temporal = module.temporality_snapshot();
        assert_eq!(temporal.tempo, 1.0);

        let world = module.world_snapshot();
        assert!(world.ready_to_hand.is_empty());

        let self_model = module.self_model_snapshot();
        assert!(self_model.current_assertions.is_empty());

        let care = module.care_snapshot();
        assert_eq!(care.fallenness_depth, 0.0);
    }

    #[test]
    fn test_event_sender_available() {
        let (module, _tx) = DaseinModule::new(test_clock());
        let _sender = module.event_sender();
        // Just verify we can get a sender without panicking
    }

    #[test]
    fn test_quick_mood_update_error() {
        let (module, _rx) = DaseinModule::new(test_clock());
        let mood = module.quick_mood_update("operation failed with error");
        assert!(matches!(mood, Stimmung::Geknickt { .. }));
    }

    #[test]
    fn test_quick_mood_update_success() {
        let (module, _rx) = DaseinModule::new(test_clock());
        let mood = module.quick_mood_update("task completed successfully");
        assert!(matches!(mood, Stimmung::Gelaunt { .. }));
    }
}
