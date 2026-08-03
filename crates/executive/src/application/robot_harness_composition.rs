//! RobotHarness composition root.
//!
//! Assembles the robot main chain: polling world state, deterministic verifier,
//! durable episode sink, embodied-execution adapter, policy provider and planner
//! into a `cognit::harness::robot::RobotHarness`. Construction is explicit —
//! `HarnessKind::Robot` configuration must supply every port or the build fails
//! closed; it never falls back to a Linear session that still claims to be a
//! robot task.

use std::sync::Arc;

use cognit::harness::robot::state::RobotHarnessConfig;
use cognit::harness::robot::{
    EmbodiedExecutionPort, EpisodeSink, OutcomeVerifierPort, PlanPort, RobotHarness,
};
use cognit::ports::policy_provider::PolicyProviderPort;
use fabric::types::embodiment::SkillDescriptor;
use fabric::types::world_state::WorldStatePort;

/// All robot-main-chain dependencies, assembled by the daemon bootstrap.
pub struct RobotHarnessDependencies {
    pub config: RobotHarnessConfig,
    pub world_state: Arc<dyn WorldStatePort>,
    pub executor: Arc<dyn EmbodiedExecutionPort>,
    pub verifier: Arc<dyn OutcomeVerifierPort>,
    pub planner: Arc<dyn PlanPort>,
    pub episodes: Arc<dyn EpisodeSink>,
    pub policy: Arc<dyn PolicyProviderPort>,
    pub allowed_skills: Vec<SkillDescriptor>,
}

/// Assemble the RobotHarness. Returns `Err` (fail closed) when a required port
/// is missing — the caller must not fall back to a Linear session.
pub fn build_robot_harness(
    dependencies: RobotHarnessDependencies,
) -> Result<RobotHarness, String> {
    if dependencies.allowed_skills.is_empty() {
        return Err("robot composition requires a non-empty skill allowlist".into());
    }
    Ok(RobotHarness::new(
        dependencies.config,
        dependencies.world_state,
        dependencies.executor,
        dependencies.verifier,
        dependencies.planner,
        dependencies.episodes,
        dependencies.policy,
        dependencies.allowed_skills,
    ))
}

#[cfg(test)]
mod tests {
    use super::*;
    use async_trait::async_trait;
    use cognit::harness::robot::{EmbodiedExecutionPort, EpisodeSink, OutcomeVerifierPort, PlanPort};
    use cognit::ports::policy_provider::PolicyProviderPort;
    use fabric::types::embodiment::{DeviceId, SkillId, SkillRequest, SkillResult};
    use fabric::types::expected_outcome::ExpectedOutcome;
    use fabric::types::outcome_verification::{VerificationDecision, VerificationReport};
    use fabric::types::perception_observation::PerceptionObservation;
    use fabric::types::skill_proposal::{PolicyProvenance, SkillProposal};
    use fabric::types::world_state::{WorldSnapshot, WorldStatePort};
    use fabric::{MonoDeadline, MonoTime};

    struct NoopWorld;
    #[async_trait]
    impl WorldStatePort for NoopWorld {
        async fn latest(&self, _d: &DeviceId) -> Option<WorldSnapshot> {
            None
        }
        async fn observe_until(
            &self,
            _d: &DeviceId,
            _after: u64,
            _deadline: MonoDeadline,
        ) -> Option<WorldSnapshot> {
            None
        }
    }
    struct NoopExecutor;
    #[async_trait]
    impl EmbodiedExecutionPort for NoopExecutor {
        async fn execute(&self, _r: SkillRequest) -> Result<SkillResult, String> {
            Err("noop".into())
        }
        async fn cancel(&self, _d: &DeviceId) -> Result<(), String> {
            Ok(())
        }
        async fn safe_stop(&self, _d: &DeviceId) -> Result<(), String> {
            Ok(())
        }
    }
    struct NoopVerifier;
    #[async_trait]
    impl OutcomeVerifierPort for NoopVerifier {
        async fn verify(
            &self,
            _e: &ExpectedOutcome,
            _b: Option<&WorldSnapshot>,
            _a: Option<&WorldSnapshot>,
            _attempt: u32,
        ) -> VerificationReport {
            VerificationReport {
                decision: VerificationDecision::Matched,
                evaluated_sequence: 0,
                observed_paths: vec![],
                reasons: vec![],
                evidence: vec![],
            }
        }
    }
    struct NoopPlanner;
    #[async_trait]
    impl PlanPort for NoopPlanner {
        async fn plan(&self, _d: &DeviceId, _s: &WorldSnapshot, _g: &str) -> Result<SkillRequest, String> {
            Err("noop".into())
        }
        async fn replan(&self, _d: &DeviceId, _s: &WorldSnapshot, _f: &str) -> Result<SkillRequest, String> {
            Err("noop".into())
        }
    }
    struct NoopEpisodes;
    #[async_trait]
    impl EpisodeSink for NoopEpisodes {
        async fn append_attempt(
            &self,
            _e: &str, _a: u32, _o: &str, _x: &ExpectedOutcome,
            _b: Option<&WorldSnapshot>, _af: Option<&WorldSnapshot>,
            _r: Option<&SkillResult>, _v: Option<&VerificationReport>,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn close_episode(&self, _e: &str, _o: &str) -> Result<(), String> {
            Ok(())
        }
    }
    struct NoopPolicy;
    #[async_trait]
    impl PolicyProviderPort for NoopPolicy {
        async fn propose(
            &self,
            _g: &str, _d: &DeviceId, _s: &[WorldSnapshot], _v: &[PerceptionObservation],
            _a: &[SkillDescriptor],
        ) -> Result<Vec<SkillProposal>, String> {
            Ok(vec![])
        }
        async fn health(&self) -> Result<String, String> {
            Ok("ready".into())
        }
    }

    fn deps(allowlist: Vec<SkillDescriptor>) -> RobotHarnessDependencies {
        RobotHarnessDependencies {
            config: RobotHarnessConfig::default(),
            world_state: Arc::new(NoopWorld),
            executor: Arc::new(NoopExecutor),
            verifier: Arc::new(NoopVerifier),
            planner: Arc::new(NoopPlanner),
            episodes: Arc::new(NoopEpisodes),
            policy: Arc::new(NoopPolicy),
            allowed_skills: allowlist,
        }
    }

    #[test]
    fn empty_allowlist_fails_closed() {
        assert!(build_robot_harness(deps(vec![])).is_err());
    }

    #[test]
    fn full_allowlist_builds() {
        let skills = vec![SkillDescriptor {
            skill: SkillId("kuavo.stance".into()),
            device: DeviceId("bot".into()),
            summary: "stance".into(),
            input_schema: serde_json::json!({}),
            risk: fabric::types::embodiment::RiskClass::Low,
            timeout_ms: 10_000,
            cancellable: false,
            preconditions: vec![],
            success_criteria: vec![],
        }];
        assert!(build_robot_harness(deps(skills)).is_ok());
    }
}
