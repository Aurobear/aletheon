use std::sync::Arc;

use ::contracts::types::embodiment::{
    DeviceId, EmbodimentExecutionPort, SkillId, SkillOutcome, SkillRequest,
};
use aletheon::host::embodiment::build_embodiment_invoker;
use aletheon::host::embodiment::EmbodimentService;
use hardware::progress_projection::RecordingEmbodimentProgress;
use hardware::{Broker, ManualClock, ProviderRegistry, SimulatedEmbodiment};
use kernel::chronos::TestClock;

#[tokio::test]
async fn skill_is_admitted_executed_correlated_and_settled() {
    let kernel = Arc::new(kernel::KernelRuntime::with_clock(Arc::new(TestClock::new(
        100, 0,
    ))));
    let hardware_clock = Arc::new(ManualClock::new(100));
    let progress = Arc::new(RecordingEmbodimentProgress::default());
    let mut registry = ProviderRegistry::new();
    registry.register(
        DeviceId("bot".into()),
        Arc::new(SimulatedEmbodiment::mobile_robot(
            "bot",
            hardware_clock.clone(),
        )),
    );
    let broker = Arc::new(Broker::new(Arc::new(registry), hardware_clock));
    let (invoker, active) =
        build_embodiment_invoker(kernel.admission(), broker.clone(), progress.clone());
    let service = EmbodimentService::new(
        broker,
        invoker,
        active,
        ::contracts::ProcessId::new(),
        ::contracts::PrincipalId("operator".into()),
        ::contracts::WorkspacePolicy::from_resolved_roots(
            std::path::PathBuf::from("/tmp/embodiment-production"),
            vec![],
        )
        .unwrap(),
    );

    let result = service
        .execute_skill(SkillRequest {
            skill: SkillId("navigate".into()),
            device: DeviceId("bot".into()),
            parameters: serde_json::json!({"x": 2.0, "y": 3.0}),
        })
        .await
        .unwrap();

    assert_eq!(result.outcome, SkillOutcome::Succeeded);
    assert_eq!(result.device, DeviceId("bot".into()));
    let updates = progress.updates().await;
    assert_eq!(updates.len(), 1);
    assert_eq!(updates[0].operation_id, result.operation_id);
}
