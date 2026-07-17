use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::KernelRuntime;
use fabric::{ExitReason, OperationKind, OperationRequest, ProcessId, ProcessSignal, SpawnSpec};
use std::sync::Arc;

#[tokio::test]
async fn runtime_rejects_unknown_and_terminal_operation_owners() {
    let runtime = KernelRuntime::with_clock(Arc::new(TestClock::default()));
    let request = |owner| OperationRequest {
        owner,
        parent: None,
        kind: OperationKind::Turn,
        deadline: None,
    };
    assert!(runtime
        .submit_operation(request(ProcessId::new()))
        .await
        .is_err());
    let owner = runtime.spawn_process(SpawnSpec::default()).await.unwrap();
    runtime
        .signal_process(owner.id, ProcessSignal::Kill)
        .await
        .unwrap();
    assert!(runtime.submit_operation(request(owner.id)).await.is_err());
}

#[tokio::test]
async fn runtime_owns_exact_operation_lifecycle_and_typed_views() {
    let runtime = KernelRuntime::with_clock(Arc::new(TestClock::default()));
    let owner = runtime.spawn_process(SpawnSpec::default()).await.unwrap();
    let operation = runtime
        .submit_operation(OperationRequest {
            owner: owner.id,
            parent: None,
            kind: OperationKind::Turn,
            deadline: None,
        })
        .await
        .unwrap();
    runtime.start_operation(operation.id).await.unwrap();
    runtime.succeed_operation(operation.id).await.unwrap();
    let result = runtime.wait_operation(operation.id).await.unwrap();
    assert_eq!(result.state, fabric::OperationState::Succeeded);
}

#[tokio::test]
async fn runtime_terminal_process_releases_child_space() {
    let runtime = KernelRuntime::with_clock(Arc::new(TestClock::default()));
    let parent = runtime.spawn_process(SpawnSpec::default()).await.unwrap();
    let child_spec = SpawnSpec {
        parent: Some(parent.id),
        ..SpawnSpec::default()
    };
    let child = runtime.spawn_process(child_spec).await.unwrap();
    let child_space = runtime.inspect_process(child.id).await.unwrap().space;
    assert!(runtime.inspect_space(child_space).is_some());
    runtime
        .exit_process(child.id, ExitReason::Panic("fixture".into()))
        .await
        .unwrap();
    assert!(runtime.inspect_space(child_space).is_none());
}
