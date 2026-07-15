use crate::dasein::context::{
    BewandtnisSnapshot, CareStructureSnapshot, DaseinContext, SelfModelSnapshot,
    TemporalStreamSnapshot,
};
use crate::dasein::event::DaseinEvent;
use crate::dasein::transition::{SelfTransitionReceipt, SelfTransitionRequest, SelfVersion};
use crate::dasein::types::Stimmung;

// ═══ DaseinOps Trait ═══

/// The Dasein module's public interface.
#[async_trait::async_trait]
pub trait DaseinOps: Send + Sync {
    /// Get current mood (Stimmung)
    fn mood(&self) -> Stimmung;

    /// Get temporal stream snapshot
    fn temporality_snapshot(&self) -> TemporalStreamSnapshot;

    /// Get involvement network snapshot
    fn world_snapshot(&self) -> BewandtnisSnapshot;

    /// Get self model snapshot
    fn self_model_snapshot(&self) -> SelfModelSnapshot;

    /// Get care structure snapshot
    fn care_snapshot(&self) -> CareStructureSnapshot;

    /// Generate complete context for LLM prompt injection
    fn to_context_injection(&self) -> DaseinContext;

    /// Apply one canonical interpreted experience transition.
    async fn transition(
        &self,
        request: SelfTransitionRequest,
    ) -> anyhow::Result<SelfTransitionReceipt>;

    /// Current reducer version.
    async fn self_version(&self) -> SelfVersion;

    /// Feed a legacy event into the canonical reducer.
    async fn handle_event(&self, event: DaseinEvent) -> anyhow::Result<SelfTransitionReceipt>;

    /// Start the sorge loop (background task)
    async fn start_sorge_loop(&self) -> anyhow::Result<()>;

    /// Stop the sorge loop
    async fn stop_sorge_loop(&self) -> anyhow::Result<()>;

    /// Check if sorge loop is running
    fn is_alive(&self) -> bool;
}
