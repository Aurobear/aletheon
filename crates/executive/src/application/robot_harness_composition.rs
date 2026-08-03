//! RobotHarness composition root.
//!
//! Assembles the robot main chain: polling world state, deterministic verifier,
//! durable episode sink, embodied-execution adapter, policy provider and planner
//! into a `cognit::harness::robot::RobotHarness`. Construction is explicit —
//! `HarnessKind::Robot` configuration must supply every port or the build fails
//! closed; it never falls back to a Linear session that still claims to be a
//! robot task.

use std::sync::Arc;

use async_trait::async_trait;
use cognit::harness::robot::session::RobotCognitiveSession;
use cognit::harness::robot::state::RobotHarnessConfig;
use cognit::harness::robot::{
    EmbodiedExecutionPort, EpisodePromotionPort, EpisodeSink, OutcomeVerifierPort, PlanPort,
    RobotHarness,
};
use cognit::ports::policy_provider::PolicyProviderPort;
use fabric::types::embodiment::{DeviceId, SkillDescriptor, SkillRequest};
use fabric::types::expected_outcome::{ExpectedOutcome, OutcomePredicate};
use fabric::types::perception_observation::PerceptionObservation;
use fabric::types::skill_proposal::{PolicyProvenance, SkillProposal};
use fabric::types::world_state::{WorldSnapshot, WorldStatePort};
use fabric::Clock;

use super::deterministic_outcome_verifier::DeterministicOutcomeVerifier;
use super::embodied_execution_adapter::EmbodiedExecutionAdapter;
use super::harness_factory::CognitiveSessionFactory;
use super::world_state::{EmbodimentWorldState, WorldStatePump};

/// All robot-main-chain dependencies, assembled by the daemon bootstrap.
#[derive(Clone)]
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

/// Fail-closed planner: replanning is not configured yet, so a replan request
/// fails the harness closed instead of silently re-issuing a stale skill.
pub struct DefaultPlanPort;
#[async_trait]
impl PlanPort for DefaultPlanPort {
    async fn plan(
        &self,
        _d: &DeviceId,
        _s: &WorldSnapshot,
        _g: &str,
    ) -> Result<SkillRequest, String> {
        Err("robot planner not configured".into())
    }
    async fn replan(
        &self,
        _d: &DeviceId,
        _s: &WorldSnapshot,
        _f: &str,
    ) -> Result<SkillRequest, String> {
        Err("robot replan not configured".into())
    }
}

/// Default policy: proposes the first allowed skill with a generic expected
/// outcome. Production now requires a real policy provider (fail closed);
/// this remains as the explicit dev/test fallback.
#[allow(dead_code)]
pub struct StubRobotPolicy;
#[async_trait]
impl PolicyProviderPort for StubRobotPolicy {
    async fn propose(
        &self,
        _goal: &str,
        device: &DeviceId,
        _snapshots: &[WorldSnapshot],
        _visual: &[PerceptionObservation],
        allowed_skills: &[SkillDescriptor],
    ) -> Result<Vec<SkillProposal>, String> {
        let Some(skill) = allowed_skills.first() else {
            return Ok(vec![]);
        };
        Ok(vec![SkillProposal {
            skill: skill.skill.clone(),
            device: device.clone(),
            parameters: serde_json::json!({}),
            expected_outcome: ExpectedOutcome {
                predicate: OutcomePredicate::Equals {
                    path: "mode".into(),
                    value: serde_json::json!("stance"),
                },
                freshness_ms: 500,
                stable_window_ms: 0,
                timeout_ms: 5_000,
            },
            confidence: 0.9,
            frame_refs: vec![],
            provenance: PolicyProvenance {
                provider: "default".into(),
                model: "default-v1".into(),
                version: "1.0".into(),
                digest: "sha256:default".into(),
            },
        }])
    }
    async fn health(&self) -> Result<String, String> {
        Ok("ready".into())
    }
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
    promoter: Option<Arc<dyn EpisodePromotionPort>>,
}

impl RobotCognitiveSessionFactory {
    pub fn new(
        deps: RobotHarnessDependencies,
        clock: Arc<dyn Clock>,
        device: DeviceId,
        sim_scene_version: impl Into<String>,
        aletheon_commit: impl Into<String>,
        bridge_protocol_digest: impl Into<String>,
        promoter: Option<Arc<dyn EpisodePromotionPort>>,
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
            promoter,
        })
    }
}

#[async_trait]
impl CognitiveSessionFactory for RobotCognitiveSessionFactory {
    async fn create(
        &self,
        _session: &fabric::SessionRecord,
        _policy: &crate::application::turn_policy::TurnPolicy,
        cancellation: tokio_util::sync::CancellationToken,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        let harness = build_robot_harness(self.deps.clone()).map_err(anyhow::Error::msg)?;
        Ok(Box::new(RobotCognitiveSession::new(
            harness,
            self.clock.clone(),
            cancellation,
            self.device.clone(),
            self.sim_scene_version.clone(),
            self.aletheon_commit.clone(),
            self.bridge_protocol_digest.clone(),
            self.promoter.clone(),
        )))
    }
}

/// Assemble the robot session factory from the daemon's embodied execution port.
/// `HarnessKind::Robot` requires a configured provider; otherwise it fails closed.
/// The policy provider is injected by the caller — production must supply a real
/// `GrpcPolicyProvider` (or an explicitly chosen fallback), never a silent stub.
pub async fn build_robot_session_factory(
    executor: Arc<dyn fabric::types::embodiment::EmbodimentExecutionPort>,
    clock: Arc<dyn Clock>,
    data_dir: &std::path::Path,
    device: DeviceId,
    unsafe_predicates: Vec<OutcomePredicate>,
    policy: Arc<dyn PolicyProviderPort>,
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
    let world = Arc::new(EmbodimentWorldState::new(16, clock.clone()));
    let pump = WorldStatePump::new(
        world.clone(),
        executor.clone(),
        clock.clone(),
        std::time::Duration::from_millis(250),
    );
    Arc::new(pump).spawn(vec![device.clone()]);
    let executor_adapter: Arc<dyn EmbodiedExecutionPort> =
        Arc::new(EmbodiedExecutionAdapter::new(executor));
    let verifier: Arc<dyn OutcomeVerifierPort> =
        Arc::new(DeterministicOutcomeVerifier::new(world.clone(), clock.clone(), unsafe_predicates));
    let episodes: Arc<dyn EpisodeSink> = Arc::new(
        crate::adapters::episode::sqlite_episode_sink::SqliteEpisodeSink::open(
            data_dir.join("robot-episodes.db"),
            clock.clone(),
        )?,
    );
    let deps = RobotHarnessDependencies {
        config: RobotHarnessConfig::default(),
        world_state: world.clone(),
        executor: executor_adapter,
        verifier,
        planner: Arc::new(DefaultPlanPort),
        episodes,
        policy,
        allowed_skills,
    };
    Ok(Arc::new(RobotCognitiveSessionFactory::new(
        deps,
        clock,
        device,
        sim_scene_version,
        aletheon_commit,
        bridge_protocol_digest,
        promoter,
    )?))
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
    use fabric::types::skill_proposal::SkillProposal;
    use fabric::types::world_state::{WorldSnapshot, WorldStatePort};
    use fabric::MonoDeadline;

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
            _e: &str, _a: u32, _ai: &str, _o: Option<&fabric::OperationId>, _x: &ExpectedOutcome,
            _b: Option<&WorldSnapshot>, _af: Option<&WorldSnapshot>,
            _r: Option<&SkillResult>, _v: Option<&VerificationReport>,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn close_episode(&self, _e: &str, _o: &str) -> Result<(), String> {
            Ok(())
        }
        async fn update_verification(
            &self,
            _e: &str,
            _ai: &str,
            _v: &VerificationReport,
        ) -> Result<(), String> {
            Ok(())
        }
        async fn load_attempts(
            &self,
            _e: &str,
        ) -> Result<Vec<fabric::types::episode_report::AttemptRecord>, String> {
            Ok(vec![])
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
