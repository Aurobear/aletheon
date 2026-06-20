//! DaseinModule — the existential substrate of SelfField.
//!
//! Philosophy: Heidegger (Dasein/Sorge/Temporality),
//! Husserl (inner time consciousness),
//! Sartre (negativity/pour-soi),
//! Merleau-Ponty (embodiment).

pub mod types;
pub mod temporality;
pub mod bewandtnis;
pub mod self_model;
pub mod negativity;
pub mod care_structure;
pub mod sorge;
pub mod context_injection;
pub mod event_bridge;
pub mod persistence;

pub use aletheon_abi::dasein::*;

use std::sync::Arc;
use parking_lot::RwLock;
use tokio::sync::mpsc;

use temporality::TemporalStream;
use bewandtnis::Bewandtnisganzheit;
use self_model::MutableSelfModel;
use negativity::NegativityEngine;
use care_structure::CareStructure;
use sorge::SorgeLoop;
use context_injection::format_dasein_context;
pub use event_bridge::DaseinEventBridge;

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
}

impl DaseinModule {
    pub fn new() -> (Self, mpsc::Sender<DaseinEvent>) {
        let (sorge, event_tx) = SorgeLoop::new(256);
        let external_tx = event_tx.clone();

        let temporality = Arc::new(TemporalStream::new(50, 0.8));
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
        };

        (module, external_tx)
    }

    /// Start the sorge loop.
    pub fn start_sorge_loop(&self) -> Option<tokio::task::JoinHandle<()>> {
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
    pub fn stop_sorge_loop(&self) {
        self.sorge.stop();
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
            Stimmung::Geknickt { because: "turn had errors".to_string() }
        } else if turn_text.contains("success") || turn_text.contains("completed") {
            Stimmung::Gelaunt { toward: "successful completion".to_string() }
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
        Self::new().0
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
        self.start_sorge_loop();
        Ok(())
    }

    async fn stop_sorge_loop(&self) -> anyhow::Result<()> {
        self.stop_sorge_loop();
        Ok(())
    }

    fn is_alive(&self) -> bool {
        self.is_alive()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_dasein_module_creation() {
        let (module, _tx) = DaseinModule::new();
        assert_eq!(module.mood(), Stimmung::Gelassenheit);
        assert!(!module.is_alive()); // sorge not started yet
    }

    #[test]
    fn test_context_injection() {
        let (module, _tx) = DaseinModule::new();

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
        let (module, _tx) = DaseinModule::new();
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
        let (module, _tx) = DaseinModule::new();

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
        let (module, _tx) = DaseinModule::new();
        let _sender = module.event_sender();
        // Just verify we can get a sender without panicking
    }

    #[test]
    fn test_quick_mood_update_error() {
        let (module, _rx) = DaseinModule::new();
        let mood = module.quick_mood_update("operation failed with error");
        assert!(matches!(mood, Stimmung::Geknickt { .. }));
    }

    #[test]
    fn test_quick_mood_update_success() {
        let (module, _rx) = DaseinModule::new();
        let mood = module.quick_mood_update("task completed successfully");
        assert!(matches!(mood, Stimmung::Gelaunt { .. }));
    }
}
