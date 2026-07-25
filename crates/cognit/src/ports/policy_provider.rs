//! PolicyProvider port — external policy model proposes skills from observations.

use async_trait::async_trait;
use fabric::types::embodiment::{DeviceId, SkillDescriptor};
use fabric::types::perception_observation::PerceptionObservation;
use fabric::types::skill_proposal::SkillProposal;
use fabric::types::world_state::WorldSnapshot;

/// Port for an external policy (VLA/LLM) that proposes semantic skills
/// from observations. Policy cannot directly actuate — only propose
/// registered skills through governance.
#[async_trait]
pub trait PolicyProviderPort: Send + Sync {
    /// Propose one or more skills given the current observations and goal.
    /// The provider receives the list of allowed skills; proposals for
    /// unregistered skills are rejected upstream.
    async fn propose(
        &self,
        goal: &str,
        device: &DeviceId,
        snapshots: &[WorldSnapshot],
        visual_observations: &[PerceptionObservation],
        allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, String>;

    /// Health check — return "ready" or a failure reason.
    async fn health(&self) -> Result<String, String>;
}
