//! PolicyProvider port — external policy model proposes skills from observations.

use async_trait::async_trait;
use fabric::types::embodiment::{DeviceId, SkillDescriptor};
use fabric::types::perception_observation::PerceptionObservation;
use fabric::types::robot_failure::RobotFailureClass;
use fabric::types::skill_proposal::SkillProposal;
use fabric::types::world_state::WorldSnapshot;

use crate::harness::robot::state::ReplanContext;

/// Typed proposal failures. RobotHarness renders these stable codes into its
/// terminal failure state; callers never infer timeout/empty-response semantics
/// from provider prose.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum PolicyProviderError {
    #[error("policy_invalid_request: {0}")]
    InvalidRequest(String),
    #[error("policy_timeout: gateway request exceeded the configured deadline")]
    Timeout,
    #[error("policy_rpc_failure: {0}")]
    Rpc(String),
    #[error("policy_gateway_rejected: {0}")]
    Gateway(String),
    #[error("policy_empty_response: gateway returned no proposals")]
    EmptyResponse,
    #[error("policy_response_limit: received {actual} proposals above negotiated limit {limit}")]
    ResponseLimit { actual: usize, limit: usize },
    #[error("policy_invalid_response: {0}")]
    InvalidResponse(String),
}

impl PolicyProviderError {
    pub fn failure_class(&self) -> RobotFailureClass {
        match self {
            Self::Timeout | Self::Rpc(_) | Self::Gateway(_) => RobotFailureClass::PolicyUnavailable,
            Self::InvalidRequest(_)
            | Self::EmptyResponse
            | Self::ResponseLimit { .. }
            | Self::InvalidResponse(_) => RobotFailureClass::ProposalRejected,
        }
    }
}

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
    ) -> Result<Vec<SkillProposal>, PolicyProviderError>;

    /// Replan from finite typed prior-attempt state. Test providers may rely on
    /// this default; the gRPC adapter overrides it to transmit the context.
    async fn replan(
        &self,
        context: &ReplanContext,
        visual_observations: &[PerceptionObservation],
    ) -> Result<Vec<SkillProposal>, PolicyProviderError> {
        self.propose(
            &context.goal,
            &context.device,
            std::slice::from_ref(&context.latest_snapshot),
            visual_observations,
            &context.allowed_skills,
        )
        .await
    }

    /// Health check — return "ready" or a failure reason.
    async fn health(&self) -> Result<String, String>;
}
