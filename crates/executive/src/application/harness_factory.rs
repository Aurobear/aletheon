use async_trait::async_trait;
use cognit::harness::config::HarnessConfig;
use fabric::SessionRecord;
use mnemosyne::runtime::AdvancedCompressor;
use tokio_util::sync::CancellationToken;

use crate::application::turn_policy::TurnPolicy;
use crate::composition::config::ExecutiveConfig;

#[derive(Debug, thiserror::Error)]
pub enum ExecutionTargetRoutingError {
    #[error("execution_target_invalid: {0}")]
    Invalid(String),
    #[error("execution_target_unavailable: {0}")]
    Unavailable(String),
    #[error("execution_target_mismatch: {0}")]
    Mismatch(String),
}

/// Stable operator-facing label for the typed harness selected at bootstrap.
pub fn selected_harness_kind(kind: cognit::harness::HarnessKind) -> &'static str {
    match kind {
        cognit::harness::HarnessKind::Linear => "linear",
        cognit::harness::HarnessKind::Robot => "robot",
    }
}

#[async_trait]
pub trait CognitiveSessionFactory: Send + Sync {
    fn validate_target(
        &self,
        target: &fabric::ExecutionTargetSelection,
    ) -> Result<(), ExecutionTargetRoutingError> {
        target
            .validate()
            .map_err(ExecutionTargetRoutingError::Invalid)?;
        match &target.target {
            fabric::ExecutionTarget::General => Ok(()),
            fabric::ExecutionTarget::Robot { .. } => Err(ExecutionTargetRoutingError::Unavailable(
                "robot capability is not configured".into(),
            )),
        }
    }

    async fn create(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>>;

    async fn create_configured(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        _config: HarnessConfig,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        self.create(session, policy, cancellation).await
    }

    async fn create_configured_with_batch_planner(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        config: HarnessConfig,
        cancellation: CancellationToken,
        _batch_planner: Option<std::sync::Arc<dyn cognit::harness::BatchPlanner>>,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        self.create_configured(session, policy, config, cancellation)
            .await
    }

    /// Per-turn target routing hook. Ordinary factories accept only General;
    /// the production composite overrides this method to expose an optional,
    /// exactly-bound Robot capability without creating another Turn Engine.
    async fn create_configured_for_target(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        target: &fabric::ExecutionTargetSelection,
        config: HarnessConfig,
        cancellation: CancellationToken,
        batch_planner: Option<std::sync::Arc<dyn cognit::harness::BatchPlanner>>,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        self.validate_target(target).map_err(anyhow::Error::new)?;
        match &target.target {
            fabric::ExecutionTarget::General => {
                self.create_configured_with_batch_planner(
                    session,
                    policy,
                    config,
                    cancellation,
                    batch_planner,
                )
                .await
            }
            fabric::ExecutionTarget::Robot { .. } => {
                anyhow::bail!("execution_target_unavailable: robot capability is not configured")
            }
        }
    }
}

/// Single routing point used by the one authoritative Turn Engine.
pub struct TargetRoutedCognitiveSessionFactory {
    general: std::sync::Arc<dyn CognitiveSessionFactory>,
    robot: Option<RobotSessionCapability>,
}

pub struct RobotSessionCapability {
    factory: std::sync::Arc<dyn CognitiveSessionFactory>,
    device_id: fabric::types::embodiment::DeviceId,
    environment: fabric::types::embodiment::ExecutionEnvironment,
}

impl RobotSessionCapability {
    pub fn new(
        factory: std::sync::Arc<dyn CognitiveSessionFactory>,
        device_id: fabric::types::embodiment::DeviceId,
        environment: fabric::types::embodiment::ExecutionEnvironment,
    ) -> Self {
        Self {
            factory,
            device_id,
            environment,
        }
    }
}

impl TargetRoutedCognitiveSessionFactory {
    pub fn new(
        general: std::sync::Arc<dyn CognitiveSessionFactory>,
        robot: Option<RobotSessionCapability>,
    ) -> Self {
        Self { general, robot }
    }
}

#[async_trait]
impl CognitiveSessionFactory for TargetRoutedCognitiveSessionFactory {
    fn validate_target(
        &self,
        target: &fabric::ExecutionTargetSelection,
    ) -> Result<(), ExecutionTargetRoutingError> {
        target
            .validate()
            .map_err(ExecutionTargetRoutingError::Invalid)?;
        match &target.target {
            fabric::ExecutionTarget::General => Ok(()),
            fabric::ExecutionTarget::Robot {
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
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        self.general.create(session, policy, cancellation).await
    }

    async fn create_configured_for_target(
        &self,
        session: &SessionRecord,
        policy: &TurnPolicy,
        target: &fabric::ExecutionTargetSelection,
        config: HarnessConfig,
        cancellation: CancellationToken,
        batch_planner: Option<std::sync::Arc<dyn cognit::harness::BatchPlanner>>,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        self.validate_target(target).map_err(anyhow::Error::new)?;
        match &target.target {
            fabric::ExecutionTarget::General => {
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
            fabric::ExecutionTarget::Robot { .. } => {
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

pub struct LinearCognitiveSessionFactory {
    config: HarnessConfig,
    clock: std::sync::Arc<dyn fabric::Clock>,
    evicted_memory: Option<std::sync::Arc<tokio::sync::Mutex<mnemosyne::runtime::RecallMemory>>>,
    verifier: Option<std::sync::Arc<dyn fabric::policy::verifier::Verifier>>,
    dasein: Option<std::sync::Arc<dyn fabric::dasein::DaseinOps>>,
}

impl LinearCognitiveSessionFactory {
    pub fn new(config: HarnessConfig, clock: std::sync::Arc<dyn fabric::Clock>) -> Self {
        Self {
            config,
            clock,
            evicted_memory: None,
            verifier: None,
            dasein: None,
        }
    }

    pub fn with_verifier(
        mut self,
        verifier: std::sync::Arc<dyn fabric::policy::verifier::Verifier>,
    ) -> Self {
        self.verifier = Some(verifier);
        self
    }

    pub fn with_evicted_memory(
        mut self,
        memory: std::sync::Arc<tokio::sync::Mutex<mnemosyne::runtime::RecallMemory>>,
    ) -> Self {
        self.evicted_memory = Some(memory);
        self
    }

    pub fn with_grounded_dasein_outcomes(
        mut self,
        dasein: std::sync::Arc<dyn fabric::dasein::DaseinOps>,
    ) -> Self {
        self.dasein = Some(dasein);
        self
    }

    fn grounded_outcome_sink(
        &self,
        session: &SessionRecord,
    ) -> Option<std::sync::Arc<dyn cognit::core::GroundedOutcomeSink>> {
        self.dasein.as_ref().map(|dasein| {
            std::sync::Arc::new(
                crate::application::dasein_workspace_adapter::GroundedDaseinOutcomeSink::new(
                    dasein.clone(),
                    self.clock.clone(),
                    session.id.0.clone(),
                ),
            ) as std::sync::Arc<dyn cognit::core::GroundedOutcomeSink>
        })
    }
}

pub fn production_cognitive_session_factory(
    config: &ExecutiveConfig,
    clock: std::sync::Arc<dyn fabric::Clock>,
    memory: std::sync::Arc<tokio::sync::Mutex<mnemosyne::runtime::RecallMemory>>,
    dasein: std::sync::Arc<dyn fabric::dasein::DaseinOps>,
) -> std::sync::Arc<dyn CognitiveSessionFactory> {
    tracing::info!(
        harness = "linear",
        configured_harness = selected_harness_kind(config.harness_kind),
        "linear cognitive session factory composed"
    );
    std::sync::Arc::new(
        LinearCognitiveSessionFactory::new(harness_config_from_executive(config), clock)
            .with_evicted_memory(memory)
            .with_grounded_dasein_outcomes(dasein),
    )
}

#[async_trait]
impl CognitiveSessionFactory for LinearCognitiveSessionFactory {
    async fn create(
        &self,
        session: &SessionRecord,
        _policy: &TurnPolicy,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        Ok(Box::new(cognit::harness::LinearCognitiveSession::new(
            self.config.clone(),
            cognit::CognitiveSessionDependencies {
                clock: self.clock.clone(),
                cancellation,
                compactor: Some(compactor(&self.config)),
                batch_planner: None,
                evicted_callback: evicted_callback(self.evicted_memory.clone(), session),
                verifier: self.verifier.clone(),
                grounded_outcome_sink: self.grounded_outcome_sink(session),
            },
        )))
    }

    async fn create_configured(
        &self,
        session: &SessionRecord,
        _policy: &TurnPolicy,
        config: HarnessConfig,
        cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        let compactor = compactor(&config);
        Ok(Box::new(cognit::harness::LinearCognitiveSession::new(
            config,
            cognit::CognitiveSessionDependencies {
                clock: self.clock.clone(),
                cancellation,
                compactor: Some(compactor),
                batch_planner: None,
                evicted_callback: evicted_callback(self.evicted_memory.clone(), session),
                verifier: self.verifier.clone(),
                grounded_outcome_sink: self.grounded_outcome_sink(session),
            },
        )))
    }

    async fn create_configured_with_batch_planner(
        &self,
        session: &SessionRecord,
        _policy: &TurnPolicy,
        config: HarnessConfig,
        cancellation: CancellationToken,
        batch_planner: Option<std::sync::Arc<dyn cognit::harness::BatchPlanner>>,
    ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
        let compactor = compactor(&config);
        Ok(Box::new(cognit::harness::LinearCognitiveSession::new(
            config,
            cognit::CognitiveSessionDependencies {
                clock: self.clock.clone(),
                cancellation,
                compactor: Some(compactor),
                batch_planner,
                evicted_callback: evicted_callback(self.evicted_memory.clone(), session),
                verifier: self.verifier.clone(),
                grounded_outcome_sink: self.grounded_outcome_sink(session),
            },
        )))
    }
}

fn evicted_callback(
    memory: Option<std::sync::Arc<tokio::sync::Mutex<mnemosyne::runtime::RecallMemory>>>,
    session: &SessionRecord,
) -> Option<std::sync::Arc<dyn Fn(Vec<fabric::Message>) + Send + Sync>> {
    let memory = memory?;
    let session_id = session.id.0.clone();
    Some(std::sync::Arc::new(move |messages| {
        let memory = memory.clone();
        let session_id = session_id.clone();
        tokio::spawn(async move {
            let metadata = serde_json::json!({
                "scope_key": format!("session:{session_id}"),
                "sensitivity": mnemosyne::MemorySensitivity::Internal,
                "sensitivity_ord": 1,
                "authority": mnemosyne::MemoryAuthority::RawExperience,
                "provenance": "compaction_evicted"
            })
            .to_string();
            let memory = memory.lock().await;
            for message in messages {
                let content = serde_json::to_string(&message)
                    .unwrap_or_else(|_| "[unserializable evicted message]".to_string());
                if let Err(error) =
                    memory.store(&session_id, "compaction_evicted", &content, Some(&metadata))
                {
                    tracing::warn!(%error, %session_id, "failed to capture evicted compaction message");
                }
            }
        });
    }))
}

fn compactor(config: &HarnessConfig) -> Box<dyn fabric::CompactorTrait> {
    let effective_tail = if config.tail_token_budget * 4 < config.context_window_tokens {
        config.context_window_tokens / 8
    } else {
        config.tail_token_budget
    };
    Box::new(
        AdvancedCompressor::new(
            effective_tail,
            config.target_summary_chars,
            config.context_window_tokens,
        )
        .with_threshold_fraction(config.compaction_threshold_percent as f64 / 100.0),
    )
}

pub fn harness_config_from_executive(config: &ExecutiveConfig) -> HarnessConfig {
    HarnessConfig {
        max_iterations: config.max_iterations,
        compaction_enabled: config.compaction_enabled,
        compaction_threshold_percent: config.compaction_threshold_percent,
        compaction_v2: config.compaction_v2,
        tail_token_budget: config.tail_token_budget,
        target_summary_chars: config.target_summary_chars,
        context_window_tokens: config.context_window_tokens,
        max_tool_calls: config.agent_loop.max_tool_calls,
        reflection_interval: config.agent_loop.reflection_interval,
        reflection_tool_call_limit: config.agent_loop.reflection_tool_call_limit,
        circuit_breaker_max_repeats: config.circuit_breaker.max_repeats,
        circuit_breaker_window_size: config.circuit_breaker.window_size,
        learning_enabled: config.learning_enabled,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct MarkerFactory(&'static str);

    #[async_trait]
    impl CognitiveSessionFactory for MarkerFactory {
        async fn create(
            &self,
            _session: &SessionRecord,
            _policy: &TurnPolicy,
            _cancellation: CancellationToken,
        ) -> anyhow::Result<Box<dyn cognit::harness::CognitiveSession>> {
            anyhow::bail!(self.0)
        }
    }

    fn session_record() -> SessionRecord {
        SessionRecord {
            schema_version: fabric::SESSION_SCHEMA_VERSION,
            id: fabric::SessionId("target-routing".into()),
            parent: None,
            created_at_ms: 0,
            status: fabric::SessionStatus::Active,
        }
    }

    #[tokio::test]
    async fn target_router_defaults_to_general_and_uses_robot_only_when_explicit() {
        let router = TargetRoutedCognitiveSessionFactory::new(
            std::sync::Arc::new(MarkerFactory("general factory selected")),
            Some(RobotSessionCapability::new(
                std::sync::Arc::new(MarkerFactory("robot factory selected")),
                fabric::types::embodiment::DeviceId("robot-1".into()),
                fabric::types::embodiment::ExecutionEnvironment::Simulation,
            )),
        );
        let session = session_record();

        let general = router
            .create_configured_for_target(
                &session,
                &TurnPolicy::daemon(),
                &fabric::ExecutionTargetSelection::default(),
                HarnessConfig::default(),
                CancellationToken::new(),
                None,
            )
            .await
            .err()
            .expect("marker factory must fail");
        assert_eq!(general.to_string(), "general factory selected");

        let robot = fabric::ExecutionTargetSelection::robot(
            "robot-1",
            fabric::types::embodiment::ExecutionEnvironment::Simulation,
            fabric::ExecutionTargetSource::UserCommand,
        )
        .unwrap();
        let selected = router
            .create_configured_for_target(
                &session,
                &TurnPolicy::daemon(),
                &robot,
                HarnessConfig::default(),
                CancellationToken::new(),
                None,
            )
            .await
            .err()
            .expect("marker factory must fail");
        assert_eq!(selected.to_string(), "robot factory selected");
    }

    #[tokio::test]
    async fn target_router_rejects_unconfigured_or_mismatched_robot_binding() {
        let session = session_record();
        let target = fabric::ExecutionTargetSelection::robot(
            "other-robot",
            fabric::types::embodiment::ExecutionEnvironment::Simulation,
            fabric::ExecutionTargetSource::TrustedClient,
        )
        .unwrap();
        let unavailable = TargetRoutedCognitiveSessionFactory::new(
            std::sync::Arc::new(MarkerFactory("general factory selected")),
            None,
        )
        .validate_target(&target)
        .unwrap_err();
        assert!(matches!(
            unavailable,
            ExecutionTargetRoutingError::Unavailable(_)
        ));

        let router = TargetRoutedCognitiveSessionFactory::new(
            std::sync::Arc::new(MarkerFactory("general factory selected")),
            Some(RobotSessionCapability::new(
                std::sync::Arc::new(MarkerFactory("robot factory selected")),
                fabric::types::embodiment::DeviceId("robot-1".into()),
                fabric::types::embodiment::ExecutionEnvironment::Simulation,
            )),
        );
        let error = router
            .create_configured_for_target(
                &session,
                &TurnPolicy::daemon(),
                &target,
                HarnessConfig::default(),
                CancellationToken::new(),
                None,
            )
            .await
            .err()
            .expect("mismatched device must fail");
        assert!(error.to_string().contains("device 'other-robot'"));
    }

    #[tokio::test]
    async fn evicted_callback_captures_session_scoped_observed_memory() {
        let dir = tempfile::tempdir().unwrap();
        let memory = std::sync::Arc::new(tokio::sync::Mutex::new(
            mnemosyne::runtime::RecallMemory::new(
                &dir.path().join("recall.db"),
                std::sync::Arc::new(kernel::chronos::SystemClock::new()),
            )
            .unwrap(),
        ));
        let session = fabric::SessionRecord {
            schema_version: fabric::SESSION_SCHEMA_VERSION,
            id: fabric::SessionId("session-c1".into()),
            parent: None,
            created_at_ms: 0,
            status: fabric::SessionStatus::Active,
        };
        let callback = evicted_callback(Some(memory.clone()), &session).unwrap();
        callback(vec![fabric::Message::user("durable compaction marker")]);

        let mut found = Vec::new();
        for _ in 0..20 {
            tokio::task::yield_now().await;
            found = memory
                .lock()
                .await
                .search_in_session("session-c1", "durable compaction marker", 5)
                .unwrap();
            if !found.is_empty() {
                break;
            }
        }
        assert_eq!(found.len(), 1);
        assert_eq!(found[0].entry_type, "compaction_evicted");
        let metadata: serde_json::Value =
            serde_json::from_str(found[0].metadata.as_deref().unwrap()).unwrap();
        assert_eq!(metadata["scope_key"], "session:session-c1");
        assert_eq!(metadata["authority"], "raw_experience");
        assert_eq!(metadata["sensitivity"], "internal");
    }

    #[test]
    fn missing_memory_store_is_explicit_noop_fallback() {
        let session = fabric::SessionRecord {
            schema_version: fabric::SESSION_SCHEMA_VERSION,
            id: fabric::SessionId("session-noop".into()),
            parent: None,
            created_at_ms: 0,
            status: fabric::SessionStatus::Active,
        };
        assert!(evicted_callback(None, &session).is_none());
    }
}
