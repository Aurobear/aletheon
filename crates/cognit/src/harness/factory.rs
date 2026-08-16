//! Object-safe construction and execution-target routing for cognitive sessions.

use std::sync::Arc;

use async_trait::async_trait;
use contracts::turn_policy::TurnPolicy;
use contracts::{ExecutionTarget, ExecutionTargetSelection, SessionRecord};
use tokio_util::sync::CancellationToken;

use super::{BatchPlanner, CognitiveSession, HarnessConfig};

#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum ExecutionTargetRoutingError {
    #[error("execution_target_invalid: {0}")]
    Invalid(String),
    #[error("execution_target_unavailable: {0}")]
    Unavailable(String),
    #[error("execution_target_mismatch: {0}")]
    Mismatch(String),
}

/// Host composition port for constructing a stateful cognitive session.
///
/// The contract lives with the harness it creates. Concrete memory, device and
/// provider adapters remain in the outer composition root.
#[async_trait]
pub trait CognitiveSessionFactory: Send + Sync {
    fn validate_target(
        &self,
        target: &ExecutionTargetSelection,
    ) -> Result<(), ExecutionTargetRoutingError> {
        target
            .validate()
            .map_err(ExecutionTargetRoutingError::Invalid)?;
        match &target.target {
            ExecutionTarget::General => Ok(()),
            ExecutionTarget::Robot { .. } => Err(ExecutionTargetRoutingError::Unavailable(
                "robot capability is not configured".into(),
            )),
        }
    }

    async fn create(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn CognitiveSession>>;

    async fn create_configured(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        _config: HarnessConfig,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn CognitiveSession>> {
        self.create(session, policy, cancellation).await
    }

    async fn create_configured_with_batch_planner(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        config: HarnessConfig,
        cancellation: CancellationToken,
        _batch_planner: Option<Arc<dyn BatchPlanner>>,
    ) -> anyhow::Result<Box<dyn CognitiveSession>> {
        self.create_configured(session, policy, config, cancellation)
            .await
    }

    async fn create_configured_for_target(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        target: &ExecutionTargetSelection,
        config: HarnessConfig,
        cancellation: CancellationToken,
        batch_planner: Option<Arc<dyn BatchPlanner>>,
    ) -> anyhow::Result<Box<dyn CognitiveSession>> {
        self.validate_target(target).map_err(anyhow::Error::new)?;
        match &target.target {
            ExecutionTarget::General => {
                self.create_configured_with_batch_planner(
                    session,
                    policy,
                    config,
                    cancellation,
                    batch_planner,
                )
                .await
            }
            ExecutionTarget::Robot { .. } => {
                anyhow::bail!("execution_target_unavailable: robot capability is not configured")
            }
        }
    }
}

/// Exactly-bound Robot factory capability used by the target router.
pub struct RobotSessionCapability {
    factory: Arc<dyn CognitiveSessionFactory>,
    device_id: contracts::types::embodiment::DeviceId,
    environment: contracts::types::embodiment::ExecutionEnvironment,
}

impl RobotSessionCapability {
    pub fn new(
        factory: Arc<dyn CognitiveSessionFactory>,
        device_id: contracts::types::embodiment::DeviceId,
        environment: contracts::types::embodiment::ExecutionEnvironment,
    ) -> Self {
        Self {
            factory,
            device_id,
            environment,
        }
    }
}

/// Single target-routing point used by the authoritative Turn Engine.
pub struct TargetRoutedCognitiveSessionFactory {
    general: Arc<dyn CognitiveSessionFactory>,
    robot: Option<RobotSessionCapability>,
}

impl TargetRoutedCognitiveSessionFactory {
    pub fn new(
        general: Arc<dyn CognitiveSessionFactory>,
        robot: Option<RobotSessionCapability>,
    ) -> Self {
        Self { general, robot }
    }
}

#[async_trait]
impl CognitiveSessionFactory for TargetRoutedCognitiveSessionFactory {
    fn validate_target(
        &self,
        target: &ExecutionTargetSelection,
    ) -> Result<(), ExecutionTargetRoutingError> {
        target
            .validate()
            .map_err(ExecutionTargetRoutingError::Invalid)?;
        match &target.target {
            ExecutionTarget::General => Ok(()),
            ExecutionTarget::Robot {
                device_id,
                environment,
            } => {
                let capability = self.robot.as_ref().ok_or_else(|| {
                    ExecutionTargetRoutingError::Unavailable(
                        "robot capability is not configured".into(),
                    )
                })?;
                if capability.device_id != *device_id {
                    return Err(ExecutionTargetRoutingError::Mismatch(format!(
                        "robot device '{}' is not configured",
                        device_id.0
                    )));
                }
                if capability.environment != *environment {
                    return Err(ExecutionTargetRoutingError::Mismatch(format!(
                        "robot environment '{}' is not configured for device '{}'",
                        environment.as_str(),
                        device_id.0
                    )));
                }
                Ok(())
            }
        }
    }

    async fn create(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn CognitiveSession>> {
        self.general.create(session, policy, cancellation).await
    }

    async fn create_configured_for_target(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        target: &ExecutionTargetSelection,
        config: HarnessConfig,
        cancellation: CancellationToken,
        batch_planner: Option<Arc<dyn BatchPlanner>>,
    ) -> anyhow::Result<Box<dyn CognitiveSession>> {
        self.validate_target(target).map_err(anyhow::Error::new)?;
        match &target.target {
            ExecutionTarget::General => {
                self.general
                    .create_configured_with_batch_planner(
                        session,
                        policy,
                        config,
                        cancellation,
                        batch_planner,
                    )
                    .await
            }
            ExecutionTarget::Robot { .. } => {
                let capability = self
                    .robot
                    .as_ref()
                    .expect("validated Robot target has a matching capability");
                capability
                    .factory
                    .create_configured_with_batch_planner(
                        session,
                        policy,
                        config,
                        cancellation,
                        batch_planner,
                    )
                    .await
            }
        }
    }
}

/// General production factory: constructs a [`HarnessCognitiveSession`] driven
/// by the minimal model → tools → repeat loop with lifecycle hooks.
///
/// Owned by the harness crate so the daemon composition root no longer depends
/// on the retired `executive` implementation. `LinearCognitiveSessionFactory`
/// is the compatibility alias kept for callers that have not renamed their
/// binding; it constructs the Harness core, never `ReActLoop`.

/// `max_iterations = 0` means "unset": keep the bounded production default so
/// the harness actually runs a turn. A literal zero would make the step
/// budget hook end before the first model dispatch.
fn step_budget_from_config(config: HarnessConfig) -> u32 {
    if config.max_iterations == 0 {
        u32::try_from(HarnessConfig::default().max_iterations).unwrap_or(u32::MAX)
    } else {
        u32::try_from(config.max_iterations).unwrap_or(u32::MAX)
    }
}

pub struct HarnessCognitiveSessionFactory {
    clock: Arc<dyn contracts::Clock>,
    max_steps: u32,
    /// Optional Harness-private observation store. Runtime remains the durable
    /// Session/Turn authority when this is absent or volatile.
    persistence: Option<Arc<dyn crate::harness::session_log::HarnessSessionPersistence>>,
}

impl HarnessCognitiveSessionFactory {
    /// Construct the production Harness without a second persistence authority.
    /// Public history and restart recovery are seeded through Runtime services.
    pub fn new(config: HarnessConfig, clock: Arc<dyn contracts::Clock>) -> Self {
        Self {
            clock,
            max_steps: step_budget_from_config(config),
            persistence: None,
        }
    }

    /// Attach an explicit Harness-private observation store.
    ///
    /// This never replaces Runtime Session/Turn persistence and is intended for
    /// deterministic tests or an outer composition that needs private log
    /// replay in addition to canonical Runtime recovery.
    pub fn with_observation_store(
        mut self,
        persistence: Arc<dyn crate::harness::session_log::HarnessSessionPersistence>,
    ) -> Self {
        self.persistence = Some(persistence);
        self
    }
}

/// Compatibility name for callers that have not yet renamed their factory
/// binding. It constructs the new Harness core, not `ReActLoop`.
pub type LinearCognitiveSessionFactory = HarnessCognitiveSessionFactory;

/// Stable operator-facing label for the typed harness selected at bootstrap.
pub fn selected_harness_kind(kind: crate::harness::HarnessKind) -> &'static str {
    match kind {
        crate::harness::HarnessKind::Linear => "linear",
        crate::harness::HarnessKind::Robot => "robot",
    }
}

#[async_trait]
impl CognitiveSessionFactory for HarnessCognitiveSessionFactory {
    async fn create(
        &self,
        session: &SessionRecord,
        _policy: &TurnPolicy,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn CognitiveSession>> {
        Ok(Box::new(
            crate::harness::HarnessCognitiveSession::with_hooks_and_persistence(
                crate::harness::session_log::HarnessSessionId(session.id.0.clone()),
                self.clock.clone(),
                cancellation,
                Arc::new(crate::harness::lifecycle::StepBudgetHook::new(
                    self.max_steps,
                )),
                self.persistence.clone(),
            )?,
        ))
    }

    async fn create_configured(
        &self,
        session: &SessionRecord,
        _policy: &TurnPolicy,
        config: HarnessConfig,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn CognitiveSession>> {
        Ok(Box::new(
            crate::harness::HarnessCognitiveSession::with_hooks_and_persistence(
                crate::harness::session_log::HarnessSessionId(session.id.0.clone()),
                self.clock.clone(),
                cancellation,
                Arc::new(crate::harness::lifecycle::StepBudgetHook::new(
                    step_budget_from_config(config),
                )),
                self.persistence.clone(),
            )?,
        ))
    }

    async fn create_configured_with_batch_planner(
        &self,
        session: &SessionRecord,
        _policy: &TurnPolicy,
        config: HarnessConfig,
        cancellation: CancellationToken,
        batch_planner: Option<Arc<dyn crate::harness::BatchPlanner>>,
    ) -> anyhow::Result<Box<dyn CognitiveSession>> {
        let session = crate::harness::HarnessCognitiveSession::with_hooks_and_persistence(
            crate::harness::session_log::HarnessSessionId(session.id.0.clone()),
            self.clock.clone(),
            cancellation,
            Arc::new(crate::harness::lifecycle::StepBudgetHook::new(
                step_budget_from_config(config),
            )),
            self.persistence.clone(),
        )?;
        let session = match batch_planner {
            Some(planner) => session.with_batch_planner(planner),
            None => session,
        };
        Ok(Box::new(session))
    }
}

#[cfg(test)]
mod step_budget_tests {
    use super::*;

    struct FixedClock;

    impl contracts::Clock for FixedClock {
        fn wall_now(&self) -> contracts::WallTime {
            contracts::WallTime(0)
        }

        fn mono_now(&self) -> contracts::MonoTime {
            contracts::MonoTime(0)
        }
    }

    #[test]
    fn zero_max_iterations_falls_back_to_production_default() {
        // A literal zero step budget would end the harness before the first
        // model dispatch (pre_step: step > max_steps). The deployed config
        // `max_iterations = 0` historically means "unset".
        let config = HarnessConfig {
            max_iterations: 0,
            ..HarnessConfig::default()
        };
        assert_eq!(
            step_budget_from_config(config),
            u32::try_from(HarnessConfig::default().max_iterations).unwrap()
        );
    }

    #[test]
    fn explicit_max_iterations_is_preserved() {
        let config = HarnessConfig {
            max_iterations: 7,
            ..HarnessConfig::default()
        };
        assert_eq!(step_budget_from_config(config), 7);
    }

    #[test]
    fn production_factory_has_no_second_persistence_authority() {
        let factory =
            HarnessCognitiveSessionFactory::new(HarnessConfig::default(), Arc::new(FixedClock));
        assert!(factory.persistence.is_none());
    }
}
