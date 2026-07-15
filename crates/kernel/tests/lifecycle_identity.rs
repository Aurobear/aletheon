use aletheon_kernel::supervision::RestartPolicy;
use aletheon_kernel::KernelRuntime;
use fabric::{ExitReason, OperationKind, OsProcessId, SpawnSpec};

#[tokio::test]
async fn one_agent_maps_to_one_live_process_generation_and_rejects_stale_pid_binding() {
    let runtime = KernelRuntime::new();
    let spec = SpawnSpec::default();
    let agent = spec.agent_id;
    let first = runtime.spawn_process(spec.clone()).await.unwrap();
    assert!(runtime.spawn_process(spec.clone()).await.is_err());
    let first_identity = runtime.identity_for_agent(agent).await.unwrap();
    assert_eq!(first_identity.process_id, first.id);
    assert_eq!(first_identity.generation, 1);
    assert_eq!(
        runtime
            .bind_os_process_id(first.id, OsProcessId(42))
            .await
            .unwrap()
            .os_pid,
        Some(OsProcessId(42))
    );

    runtime
        .supervise(
            first.id,
            RestartPolicy::RestartOnFailure { max_restarts: 1 },
        )
        .await;
    let outcome = runtime
        .terminate_process(first.id, ExitReason::Failed("crash".into()))
        .await
        .unwrap();
    let replacement = outcome.restarted[0];
    let current = runtime.identity_for_agent(agent).await.unwrap();
    assert_eq!(current.process_id, replacement.id);
    assert_eq!(current.generation, 2);
    assert_eq!(current.os_pid, None);
    assert!(runtime
        .bind_os_process_id(first.id, OsProcessId(99))
        .await
        .is_err());

    let encoded = serde_json::to_string(&current).unwrap();
    assert_eq!(
        serde_json::from_str::<fabric::ProcessIdentity>(&encoded).unwrap(),
        current
    );
}

#[test]
fn operation_kinds_are_closed_versionable_discriminants() {
    let kinds = [
        OperationKind::Turn,
        OperationKind::ModelCall,
        OperationKind::CapabilityCall,
        OperationKind::MemoryConsolidation,
        OperationKind::SubAgent,
        OperationKind::ApprovedApply,
    ];
    for kind in kinds {
        let encoded = serde_json::to_string(&kind).unwrap();
        assert_eq!(
            serde_json::from_str::<OperationKind>(&encoded).unwrap(),
            kind
        );
    }
    assert!(serde_json::from_str::<OperationKind>(r#""invented_kind""#).is_err());
    assert!(serde_json::from_str::<OperationKind>(r#"{"Other":"ad_hoc"}"#).is_err());
}
