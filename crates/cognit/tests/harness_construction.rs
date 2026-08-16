use async_trait::async_trait;
use cognit::harness::{
    CognitiveSession, CognitiveSessionFactory, ExecutionTargetRoutingError, HarnessConfig,
    RobotSessionCapability, TargetRoutedCognitiveSessionFactory,
};
use contracts::turn_policy::TurnPolicy;
use contracts::{
    ExecutionTargetSelection, ExecutionTargetSource, SessionRecord, SESSION_SCHEMA_VERSION,
};
use tokio_util::sync::CancellationToken;

struct MarkerFactory(&'static str);

#[async_trait]
impl CognitiveSessionFactory for MarkerFactory {
    async fn create(
        &self,
        _session: &SessionRecord,
        _policy: &TurnPolicy,
        _cancellation: CancellationToken,
    ) -> anyhow::Result<Box<dyn CognitiveSession>> {
        anyhow::bail!(self.0)
    }
}

fn session_record() -> SessionRecord {
    SessionRecord {
        schema_version: SESSION_SCHEMA_VERSION,
        id: contracts::SessionId("harness-factory".into()),
        parent: None,
        created_at_ms: 0,
        status: contracts::SessionStatus::Active,
    }
}

#[tokio::test]
async fn one_factory_contract_routes_general_and_robot_sessions() {
    let router = TargetRoutedCognitiveSessionFactory::new(
        std::sync::Arc::new(MarkerFactory("linear selected")),
        Some(RobotSessionCapability::new(
            std::sync::Arc::new(MarkerFactory("robot selected")),
            contracts::types::embodiment::DeviceId("robot-1".into()),
            contracts::types::embodiment::ExecutionEnvironment::Simulation,
        )),
    );
    let session = session_record();

    let general = router
        .create_configured_for_target(
            &session,
            &TurnPolicy::daemon(),
            &ExecutionTargetSelection::default(),
            HarnessConfig::default(),
            CancellationToken::new(),
            None,
        )
        .await
        .err()
        .expect("marker factory must fail");
    assert_eq!(general.to_string(), "linear selected");

    let robot = ExecutionTargetSelection::robot(
        "robot-1",
        contracts::types::embodiment::ExecutionEnvironment::Simulation,
        ExecutionTargetSource::UserCommand,
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
    assert_eq!(selected.to_string(), "robot selected");
}

#[test]
fn target_validation_is_typed_before_session_construction() {
    let router = TargetRoutedCognitiveSessionFactory::new(
        std::sync::Arc::new(MarkerFactory("linear selected")),
        None,
    );
    let robot = ExecutionTargetSelection::robot(
        "robot-1",
        contracts::types::embodiment::ExecutionEnvironment::Simulation,
        ExecutionTargetSource::TrustedClient,
    )
    .unwrap();
    assert!(matches!(
        router.validate_target(&robot),
        Err(ExecutionTargetRoutingError::Unavailable(_))
    ));
}
