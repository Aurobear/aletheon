use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::operation::OperationTable;
use aletheon_kernel::process::ProcessTable;
use fabric::{
    AgentId, AgentProfileId, CancelReason, NamespaceId, OperationKind, OperationManager,
    OperationRequest, OperationState, ProcessId, ProcessManager, ProcessSignal, SpawnSpec,
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
    let table = ProcessTable::new(Arc::new(TestClock::default()));
    assert!(table.spawn(child_spec(ProcessId::new())).await.is_err());
    let parent = table.spawn(SpawnSpec::default()).await.unwrap();
    table.signal(parent.id, ProcessSignal::Kill).await.unwrap();
    assert!(table.spawn(child_spec(parent.id)).await.is_err());
}

#[tokio::test]
async fn operation_parent_must_exist_be_live_and_share_owner() {
    let table = OperationTable::new(Arc::new(TestClock::default()));
    let owner = ProcessId::new();
    let request = |parent| OperationRequest {
        owner,
        parent,
        kind: OperationKind::Turn,
        deadline: None,
    };
    assert!(table
        .submit(request(Some(fabric::OperationId::new())))
        .await
        .is_err());
    let parent = table.submit(request(None)).await.unwrap();
    let wrong_owner = table
        .submit(OperationRequest {
            owner: ProcessId::new(),
            parent: Some(parent.id),
            kind: OperationKind::CapabilityCall,
            deadline: None,
        })
        .await;
    assert!(wrong_owner.is_err());
    table.start(parent.id).await.unwrap();
    table.succeed(parent.id).await.unwrap();
    assert!(table.submit(request(Some(parent.id))).await.is_err());
}

#[tokio::test]
async fn operation_table_rejects_skipped_and_repeated_transitions() {
    let table = OperationTable::new(Arc::new(TestClock::default()));
    let operation = table
        .submit(OperationRequest {
            owner: ProcessId::new(),
            parent: None,
            kind: OperationKind::Turn,
            deadline: None,
        })
        .await
        .unwrap();
    assert!(table.succeed(operation.id).await.is_err());
    table.start(operation.id).await.unwrap();
    assert!(table.start(operation.id).await.is_err());
    table
        .cancel(operation.id, CancelReason::User)
        .await
        .unwrap();
    assert_eq!(
        table.inspect(operation.id).await.unwrap().state,
        OperationState::Cancelled
    );
    assert!(table.succeed(operation.id).await.is_err());
}
