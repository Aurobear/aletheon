use std::sync::Arc;

use executive::service::embodiment_authority::build_embodiment_invoker;
use executive::service::embodiment_progress::RecordingEmbodimentProgress;
use executive::service::embodiment_service::EmbodimentService;
use fabric::types::embodiment::{
    DeviceId, EmbodimentExecutionPort, SkillId, SkillOutcome, SkillRequest,
};
use hardware::{Broker, ManualClock, ProviderRegistry, SimulatedEmbodiment};
use kernel::chronos::TestClock;

#[tokio::test]
async fn service_queries_executes_and_correlates_progress() {
    let kernel = Arc::new(kernel::KernelRuntime::with_clock(Arc::new(TestClock::new(
        0, 0,
    ))));
    let clock = Arc::new(ManualClock::new(0));
    let mut registry = ProviderRegistry::new();
    registry.register(
        DeviceId("bot".into()),
        Arc::new(SimulatedEmbodiment::mobile_robot("bot", clock.clone())),
    );
    let broker = Arc::new(Broker::new(Arc::new(registry), clock));
    let progress = Arc::new(RecordingEmbodimentProgress::default());
    let (invoker, active) =
        build_embodiment_invoker(kernel.admission(), broker.clone(), progress.clone());
    let workspace = fabric::WorkspacePolicy::from_resolved_roots(
        std::path::PathBuf::from("/tmp/embodiment-test"),
        vec![],
    )
    .unwrap();
    let service = EmbodimentService::new(
        broker,
        invoker,
        active,
        fabric::ProcessId::new(),
        fabric::PrincipalId("operator".into()),
        workspace,
    );

    assert_eq!(
        service
            .observe(&DeviceId("bot".into()))
            .await
            .unwrap()
            .len(),
        1
    );
    assert!(service
        .get_state(&DeviceId("bot".into()))
        .await
        .unwrap()
        .is_some());
    assert_eq!(
        service.list_skills(&DeviceId("bot".into())).await.unwrap()[0].skill,
        SkillId("navigate".into())
    );
    let result = service
        .execute_skill(SkillRequest {
            skill: SkillId("navigate".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({"x": 1.0, "y": 1.0}),
        })
        .await
        .unwrap();
    assert_eq!(result.outcome, SkillOutcome::Succeeded);
    let updates = progress.updates().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].operation_id, result.operation_id);
    service.safe_stop(&DeviceId("bot".into())).await.unwrap();
}
