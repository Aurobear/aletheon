use aletheon_kernel::chronos::TestClock;
use aletheon_kernel::operation::{OperationScope, OperationTable};
use fabric::{
    CancelReason, OperationExitReason, OperationKind, OperationManager, OperationRequest,
    OperationState, ProcessId,
};
use std::sync::Arc;
use std::time::Duration;

#[tokio::test]
async fn operation_parent_cancel_propagates_to_children() {
    let table = OperationTable::new(Arc::new(TestClock::default()));
    let owner = ProcessId::new();
    let parent = table
        .submit(OperationRequest {
            owner,
            parent: None,
            kind: OperationKind::Turn,
            deadline: None,
        })
        .await
        .unwrap();
    let child = table
        .submit(OperationRequest {
            owner,
            parent: Some(parent.id),
            kind: OperationKind::ModelCall,
            deadline: None,
        })
        .await
        .unwrap();

    table.cancel(parent.id, CancelReason::User).await.unwrap();
    let parent_result = table.wait(parent.id).await.unwrap();
    let child_result = table.wait(child.id).await.unwrap();
    assert_eq!(parent_result.state, OperationState::Cancelled);
    assert_eq!(child_result.state, OperationState::Cancelled);
    assert!(matches!(
        child_result.exit,
        Some(OperationExitReason::Cancelled(CancelReason::User))
    ));
}

#[tokio::test]
async fn operation_scope_records_task_panic_as_structured_exit() {
    let mut scope = OperationScope::new(fabric::OperationId::new());
    scope.tasks.spawn(async { panic!("boom") });
    let exit = scope.join_next().await.unwrap();
    assert!(matches!(exit.reason, OperationExitReason::Panic(_)));
}

#[tokio::test]
async fn operation_scope_cancel_and_drain_aborts_or_records_tasks() {
    let mut scope = OperationScope::new(fabric::OperationId::new());
    let token = scope.token();
    scope.spawn("worker", async move {
        token.cancelled().await;
        OperationExitReason::Cancelled(CancelReason::User)
    });
    let exits = scope.cancel_and_drain(Duration::from_millis(50)).await;
    assert_eq!(exits.len(), 1);
    assert!(matches!(
        exits[0].reason,
        OperationExitReason::Cancelled(CancelReason::User)
    ));
}
