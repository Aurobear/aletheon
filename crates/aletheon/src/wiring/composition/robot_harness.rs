//! RobotHarness composition root.
//!
//! Assembles the robot main chain: polling world state, deterministic verifier,
//! durable episode sink, embodied-execution adapter and policy provider
//! into a `cognit::harness::robot::RobotHarness`. Construction is explicit —
//! An explicit Robot turn target must have every port or the build fails
//! closed; it never falls back to a Linear session that still claims to be a
//! robot task.

use std::sync::Arc;

use ::contracts::types::embodiment::{DeviceId, SkillDescriptor};
use ::contracts::types::expected_outcome::OutcomePredicate;
use ::contracts::types::world_state::WorldStatePort;
use ::contracts::Clock;
use async_trait::async_trait;
use cognit::harness::robot::session::RobotCognitiveSession;
use cognit::harness::robot::state::RobotHarnessConfig;
use cognit::harness::robot::{
    EmbodiedExecutionPort, EpisodeAuditPort, EpisodePromotionPort, EpisodeSink,
    OutcomeVerifierPort, RobotHarness, RobotPerceptionPort,
};
use cognit::ports::policy_provider::PolicyProviderPort;

use cognit::harness::robot::embodied_execution_adapter::EmbodiedExecutionAdapter;
use cognit::harness::robot::outcome_verifier::DeterministicOutcomeVerifier;
use cognit::harness::robot::perception_store::{
    EmbodimentPerceptionStore, RobotPerceptionRuntimeConfig,
};
use cognit::harness::CognitiveSessionFactory;
use hardware::world_state::{EmbodimentWorldState, WorldStatePump};

/// All robot-main-chain dependencies, assembled by the daemon bootstrap.
#[derive(Clone)]
pub struct RobotHarnessDependencies {
    pub config: RobotHarnessConfig,
    pub world_state: Arc<dyn WorldStatePort>,
    pub executor: Arc<dyn EmbodiedExecutionPort>,
    pub verifier: Arc<dyn OutcomeVerifierPort>,
    pub episodes: Arc<dyn EpisodeSink>,
    pub policy: Arc<dyn PolicyProviderPort>,
    pub perception: Arc<dyn RobotPerceptionPort>,
    pub allowed_skills: Vec<SkillDescriptor>,
}

/// Assemble the RobotHarness. Returns `Err` (fail closed) when a required port
/// is missing — the caller must not fall back to a Linear session.
pub fn build_robot_harness(dependencies: RobotHarnessDependencies) -> Result<RobotHarness, String> {
    if dependencies.allowed_skills.is_empty() {
        return Err("robot composition requires a non-empty skill allowlist".into());
    }
    Ok(RobotHarness::new(
        dependencies.config,
        dependencies.world_state,
        dependencies.executor,
        dependencies.verifier,
        dependencies.episodes,
        dependencies.policy,
        dependencies.perception,
        dependencies.allowed_skills,
    ))
}

/// `CognitiveSessionFactory` that builds a RobotHarness per session and drives
/// it through the turn contract via `RobotCognitiveSession`.
pub struct RobotCognitiveSessionFactory {
    deps: RobotHarnessDependencies,
    clock: Arc<dyn Clock>,
    device: DeviceId,
    sim_scene_version: String,
    aletheon_commit: String,
    bridge_protocol_digest: String,
    skill_descriptor_digest: String,
    promoter: Option<Arc<dyn EpisodePromotionPort>>,
    auditor: Arc<dyn EpisodeAuditPort>,
}

impl RobotCognitiveSessionFactory {
    pub fn new(
        deps: RobotHarnessDependencies,
        clock: Arc<dyn Clock>,
        device: DeviceId,
        sim_scene_version: impl Into<String>,
        aletheon_commit: impl Into<String>,
        bridge_protocol_digest: impl Into<String>,
        skill_descriptor_digest: impl Into<String>,
        promoter: Option<Arc<dyn EpisodePromotionPort>>,
        auditor: Arc<dyn EpisodeAuditPort>,
    ) -> Result<Self, String> {
        // Validate the composition eagerly — fail closed at bootstrap.
        let _ = build_robot_harness(deps.clone())?;
        Ok(Self {
            deps,
            clock,
            device,
            sim_scene_version: sim_scene_version.into(),
            aletheon_commit: aletheon_commit.into(),
            bridge_protocol_digest: bridge_protocol_digest.into(),
            skill_descriptor_digest: skill_descriptor_digest.into(),
            promoter,
            auditor,
        })
    }
}

#[async_trait]
impl CognitiveSessionFactory for RobotCognitiveSessionFactory {
    async fn create(
        &self,
        _session: &::contracts::SessionRecord,
        _policy: &runtime::turn_policy::TurnPolicy,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        let harness = build_robot_harness(self.deps.clone()).map_err(anyhow::Error::msg)?;
        Ok(Box::new(
            RobotCognitiveSession::new(
                harness,
                self.clock.clone(),
                cancellation,
                self.device.clone(),
                self.sim_scene_version.clone(),
                self.aletheon_commit.clone(),
                self.bridge_protocol_digest.clone(),
                self.skill_descriptor_digest.clone(),
                self.promoter.clone(),
            )
            .with_auditor(self.auditor.clone()),
        ))
    }
}

/// Assemble the robot session factory from the daemon's embodied execution port.
/// A Robot capability requires a configured provider; otherwise it fails closed.
/// The policy provider is injected by the caller — production must supply a real
/// `GrpcPolicyProvider` (or an explicitly chosen fallback), never a silent stub.
pub async fn build_robot_session_factory(
    executor: Arc<dyn ::contracts::types::embodiment::EmbodimentExecutionPort>,
    clock: Arc<dyn Clock>,
    data_dir: &std::path::Path,
    device: DeviceId,
    unsafe_predicates: Vec<OutcomePredicate>,
    policy: Arc<dyn PolicyProviderPort>,
    harness_config: RobotHarnessConfig,
    perception_config: RobotPerceptionRuntimeConfig,
    perception_poll_interval: std::time::Duration,
    sim_scene_version: impl Into<String>,
    aletheon_commit: impl Into<String>,
    bridge_protocol_digest: impl Into<String>,
    promoter: Option<Arc<dyn EpisodePromotionPort>>,
) -> Result<Arc<dyn CognitiveSessionFactory>, String> {
    let allowed_skills = executor
        .list_skills(&device)
        .await
        .map_err(|e| format!("robot provider list_skills: {e}"))?;
    if allowed_skills.is_empty() {
        return Err(format!("robot provider exposed no skills for {}", device.0));
    }
    for descriptor in &allowed_skills {
        descriptor.validate_contract(&device)?;
    }
    for skill in harness_config.required_perception.keys() {
        if !allowed_skills
            .iter()
            .any(|descriptor| &descriptor.skill == skill)
        {
            return Err(format!(
                "perception requirement names skill outside startup allowlist: {}",
                skill.0
            ));
        }
    }
    let skill_descriptor_digest =
        ::contracts::types::embodiment::skill_descriptor_digest(&allowed_skills)?;
    let world = Arc::new(EmbodimentWorldState::new(
        perception_config.max_devices,
        clock.clone(),
    ));
    let perception = Arc::new(EmbodimentPerceptionStore::from_config(
        clock.clone(),
        perception_config,
    )?);
    let pump = WorldStatePump::new(
        world.clone(),
        executor.clone(),
        clock.clone(),
        perception_poll_interval,
    )
    .with_perception(perception.clone());
    Arc::new(pump).spawn(vec![device.clone()]);
    let executor_adapter: Arc<dyn EmbodiedExecutionPort> =
        Arc::new(EmbodiedExecutionAdapter::new(executor));
    let verifier: Arc<dyn OutcomeVerifierPort> = Arc::new(DeterministicOutcomeVerifier::new(
        world.clone(),
        clock.clone(),
        unsafe_predicates,
    ));
    let episodes: Arc<dyn EpisodeSink> =
        Arc::new(adapters_sqlite::episode::SqliteEpisodeSink::open(
            data_dir.join("robot-episodes.db"),
            clock.clone(),
        )?);
    let deps = RobotHarnessDependencies {
        config: harness_config,
        world_state: world.clone(),
        executor: executor_adapter,
        verifier,
        episodes,
        policy,
        perception,
        allowed_skills,
    };
    let auditor: Arc<dyn EpisodeAuditPort> =
        Arc::new(cognit::harness::robot::audit_chain::AuditChain::new(4_096));
    Ok(Arc::new(RobotCognitiveSessionFactory::new(
        deps,
        clock,
        device,
        sim_scene_version,
        aletheon_commit,
        bridge_protocol_digest,
        skill_descriptor_digest,
        promoter,
        auditor,
    )?))
}

#[cfg(test)]
mod tests {
    use super::*;
    use ::contracts::types::embodiment::{DeviceId, SkillId, SkillRequest, SkillResult};
    use ::contracts::types::expected_outcome::ExpectedOutcome;
    use ::contracts::types::outcome_verification::{VerificationDecision, VerificationReport};
    use ::contracts::types::skill_proposal::SkillProposal;
    use ::contracts::types::world_state::{WorldSnapshot, WorldStatePort};
    use ::contracts::MonoDeadline;
    use async_trait::async_trait;
    use cognit::harness::robot::PerceptionObservation;
    use cognit::harness::robot::{
        EmbodiedExecutionPort, EpisodeSink, OutcomeVerifierPort, RobotExecutionError,
    };
    use cognit::ports::policy_provider::PolicyProviderPort;

    struct NoopWorld;
    #[async_trait]
    impl WorldStatePort for NoopWorld {
        async fn latest(&self, _d: &DeviceId, _schema: &str) -> Option<WorldSnapshot> {
            None
        }
        async fn observe_until(
            &self,
            _d: &DeviceId,
            _schema: &str,
            _after: u64,
            _deadline: MonoDeadline,
        ) -> Option<WorldSnapshot> {
            None
        }
    }
    struct NoopExecutor;
    #[async_trait]
    impl EmbodiedExecutionPort for NoopExecutor {
        async fn execute(&self, _r: SkillRequest) -> Result<SkillResult, RobotExecutionError> {
            Err(RobotExecutionError::Control("noop".into()))
        }
        async fn cancel(&self, _d: &DeviceId) -> Result<(), RobotExecutionError> {
            Ok(())
        }
        async fn safe_stop(&self, _d: &DeviceId) -> Result<(), RobotExecutionError> {
            Ok(())
        }
    }
    struct NoopVerifier;
    #[async_trait]
    impl OutcomeVerifierPort for NoopVerifier {
        async fn verify(
            &self,
            _e: &ExpectedOutcome,
            _d: &DeviceId,
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
    struct NoopEpisodes;
    #[async_trait]
    impl EpisodeSink for NoopEpisodes {
        async fn append_attempt(
            &self,
            _e: &str,
            _a: u32,
            _ai: &str,
            _o: Option<&::contracts::OperationId>,
            _request: &::contracts::types::embodiment::SkillRequest,
            _x: &ExpectedOutcome,
            _b: Option<&WorldSnapshot>,
            _af: Option<&WorldSnapshot>,
            _r: Option<&SkillResult>,
            _v: Option<&VerificationReport>,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn close_episode(
            &self,
            _e: &str,
            _o: ::contracts::types::episode_report::EpisodeSettlement,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn update_verification(
            &self,
            _e: &str,
            _ai: &str,
            _after: Option<&WorldSnapshot>,
            _v: &VerificationReport,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn load_attempts(
            &self,
            _e: &str,
        ) -> Result<Vec<::contracts::types::episode_report::AttemptRecord>, String> {
            Ok(vec![])
        }
    }
    struct NoopPolicy;
    #[async_trait]
    impl PolicyProviderPort for NoopPolicy {
        async fn propose(
            &self,
            _g: &str,
            _d: &DeviceId,
            _s: &[WorldSnapshot],
            _v: &[PerceptionObservation],
            _a: &[SkillDescriptor],
        ) -> Result<Vec<SkillProposal>, cognit::ports::policy_provider::PolicyProviderError>
        {
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
            episodes: Arc::new(NoopEpisodes),
            policy: Arc::new(NoopPolicy),
            perception: Arc::new(cognit::harness::robot::NoopRobotPerception),
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
            risk: ::contracts::types::embodiment::RiskClass::Low,
            timeout_ms: 10_000,
            cancellable: false,
            preconditions: vec![],
            success_criteria: vec![],
        }];
        assert!(build_robot_harness(deps(skills)).is_ok());
    }
}
