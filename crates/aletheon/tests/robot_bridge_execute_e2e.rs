//! Real-bridge skill-execution E2E (gated).
//!
//! Exercises the FULL production path — Kernel admission → Broker → provider →
//! bridge → MuJoCo sim — against a live `aletheon-kuavo-bridge`. Requires the
//! bridge on 127.0.0.1:50051 (an SSH tunnel to the `software` host works).
//!
//! ```text
//! bash scripts/cargo-agent.sh test -p executive --test robot_bridge_execute_e2e -- --ignored
//! ```

use std::sync::Arc;

use ::contracts::types::embodiment::{
    DeviceId, EmbodimentExecutionPort, SkillId, SkillOutcome, SkillRequest,
};
use aletheon::host::embodiment::build_embodiment_invoker;
use aletheon::host::embodiment::EmbodimentService;
use hardware::grpc::provider::{GrpcEmbodimentProvider, GrpcProviderConfig};
use hardware::progress_projection::RecordingEmbodimentProgress;
use hardware::{Broker, ProviderRegistry};
use kernel::chronos::TestClock;

const DEVICE_ID: &str = "kuavo-mujoco-01";

async fn connect_service() -> EmbodimentService {
    let kernel = Arc::new(kernel::KernelRuntime::with_clock(Arc::new(TestClock::new(
        0, 0,
    ))));
    let clock = Arc::new(hardware::ManualClock::new(0));
    let provider = GrpcEmbodimentProvider::connect_with_clock(
        GrpcProviderConfig {
            endpoint: "http://127.0.0.1:50051".into(),
            required_device_id: Some(DEVICE_ID.into()),
            required_observation_schemas: vec![
                hardware::ObservationSchemaRequirement {
                    schema: "base_pose".into(),
                    schema_version: 1,
                },
                hardware::ObservationSchemaRequirement {
                    schema: "base_twist".into(),
                    schema_version: 1,
                },
            ],
            ..Default::default()
        },
        clock.clone(),
    )
    .await
    .expect("bridge must be reachable (tunnel or local)");
    let mut registry = ProviderRegistry::new();
    registry.register(DeviceId(DEVICE_ID.into()), Arc::new(provider));
    let broker = Arc::new(Broker::new(Arc::new(registry), clock));
    let progress = Arc::new(RecordingEmbodimentProgress::default());
    let (invoker, active) =
        build_embodiment_invoker(kernel.admission(), broker.clone(), progress.clone());
    let workspace = ::contracts::WorkspacePolicy::from_resolved_roots(
        std::path::PathBuf::from("/tmp/embodiment-bridge-e2e"),
        vec![],
    )
    .unwrap();
    EmbodimentService::new(
        broker,
        invoker,
        active,
        ::contracts::ProcessId::new(),
        ::contracts::PrincipalId("operator".into()),
        workspace,
    )
}

#[ignore]
#[tokio::test]
async fn production_path_lists_and_executes_stop_against_bridge() {
    let service = connect_service().await;
    let device = DeviceId(DEVICE_ID.into());

    let skills = service
        .list_skills(&device)
        .await
        .expect("list_skills over bridge");
    assert!(
        skills.iter().any(|s| s.skill.0 == "kuavo.stop"),
        "bridge must expose kuavo.stop, got: {:?}",
        skills
            .iter()
            .map(|s| s.skill.0.as_str())
            .collect::<Vec<_>>()
    );

    // Production-path execution: Kernel admission -> Broker -> Grpc -> bridge.
    let result = service
        .execute_skill(SkillRequest {
            skill: SkillId("kuavo.stop".into()),
            device: device.clone(),
            parameters: serde_json::json!({}),
        })
        .await
        .expect("execute_skill over bridge");
    assert_eq!(
        result.outcome,
        SkillOutcome::Succeeded,
        "kuavo.stop must succeed over the real bridge"
    );

    let observations = service.observe(&device).await.expect("observe over bridge");
    assert!(
        !observations.is_empty(),
        "bridge must return observations after execution"
    );
}
