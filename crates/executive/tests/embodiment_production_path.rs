use std::sync::Arc;

use executive::service::embodiment_authority::KernelEmbodimentAuthority;
use executive::service::embodiment_progress::RecordingEmbodimentProgress;
use executive::service::embodiment_service::EmbodimentService;
use fabric::{
    DeviceId, EmbodimentExecutionPort, PrincipalId, ProcessId, SkillId, SkillOutcome, SkillRequest,
};
use hardware::{Broker, ManualClock, ProviderRegistry, SimulatedEmbodiment};
use kernel::chronos::TestClock;

#[tokio::test]
async fn skill_is_admitted_executed_correlated_and_settled() {
    let kernel = Arc::new(kernel::KernelRuntime::with_clock(Arc::new(TestClock::new(
        100, 0,
    ))));
    let hardware_clock = Arc::new(ManualClock::new(100));
    let authority = Arc::new(KernelEmbodimentAuthority::new(
        kernel.admission(),
        hardware_clock.clone(),
        ProcessId::new(),
        PrincipalId("operator".into()),
    ));
    let progress = Arc::new(RecordingEmbodimentProgress::default());
    let mut registry = ProviderRegistry::new();
    registry.register(
        DeviceId("bot".into()),
        Arc::new(SimulatedEmbodiment::mobile_robot(
            "bot",
            hardware_clock.clone(),
        )),
    );
    let service = EmbodimentService::new(
        Arc::new(Broker::new(Arc::new(registry), hardware_clock)),
        authority,
        progress.clone(),
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
