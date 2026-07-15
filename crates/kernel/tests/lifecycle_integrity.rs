use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::KernelRuntime;
use fabric::{
    AgentId, AgentProfileId, CancelReason, NamespaceId, OperationKind, OperationRequest,
    OperationState, ProcessId, ProcessSignal, SpawnSpec,
};
use std::sync::Arc;

fn child_spec(parent: ProcessId) -> SpawnSpec {
    SpawnSpec {
        agent_id: AgentId::new(),
        parent: Some(parent),
        profile: AgentProfileId("child".into()),
        namespace: NamespaceId("test".into()),
        initial_operation: None,
        deadline: None,
    }
}

#[tokio::test]
async fn process_spawn_rejects_orphan_and_terminal_parent() {
    let runtime = KernelRuntime::with_clock(Arc::new(TestClock::default()));
    assert!(runtime
        .spawn_process(child_spec(ProcessId::new()))
        .await
        .is_err());
    let parent = runtime.spawn_process(SpawnSpec::default()).await.unwrap();
    runtime
        .signal_process(parent.id, ProcessSignal::Kill)
        .await
        .unwrap();
    assert!(runtime.spawn_process(child_spec(parent.id)).await.is_err());
}

#[tokio::test]
async fn operation_parent_must_exist_be_live_and_share_owner() {
    let runtime = KernelRuntime::with_clock(Arc::new(TestClock::default()));
    let owner = runtime
        .spawn_process(SpawnSpec::default())
        .await
        .unwrap()
        .id;
    let other = runtime
        .spawn_process(SpawnSpec::default())
        .await
        .unwrap()
        .id;
    let request = |parent| OperationRequest {
        owner,
        parent,
        kind: OperationKind::Turn,
        deadline: None,
    };
    assert!(runtime
        .submit_operation(request(Some(fabric::OperationId::new())))
        .await
        .is_err());
    let parent = runtime.submit_operation(request(None)).await.unwrap();
    let wrong_owner = runtime
        .submit_operation(OperationRequest {
            owner: other,
            parent: Some(parent.id),
            kind: OperationKind::CapabilityCall,
            deadline: None,
        })
        .await;
    assert!(wrong_owner.is_err());
    runtime.start_operation(parent.id).await.unwrap();
    runtime.succeed_operation(parent.id).await.unwrap();
    assert!(runtime
        .submit_operation(request(Some(parent.id)))
        .await
        .is_err());
}

#[tokio::test]
async fn operation_table_rejects_skipped_and_repeated_transitions() {
    let runtime = KernelRuntime::with_clock(Arc::new(TestClock::default()));
    let process = runtime.spawn_process(SpawnSpec::default()).await.unwrap();
    let operation = runtime
        .submit_operation(OperationRequest {
            owner: process.id,
            parent: None,
            kind: OperationKind::Turn,
            deadline: None,
        })
        .await
        .unwrap();
    assert!(runtime.succeed_operation(operation.id).await.is_err());
    runtime.start_operation(operation.id).await.unwrap();
    assert!(runtime.start_operation(operation.id).await.is_err());
    runtime
        .cancel_operation(operation.id, CancelReason::User)
        .await
        .unwrap();
    assert_eq!(
        runtime.inspect_operation(operation.id).await.unwrap().state,
        OperationState::Cancelled
    );
    assert!(runtime.succeed_operation(operation.id).await.is_err());
}
