use aletheon_kernel::supervision::{RestartDecision, RestartPolicy};
use aletheon_kernel::KernelRuntime;
use fabric::ipc::envelope_v2::Target;
use fabric::ipc::mailbox::{InProcessMailbox, MailboxService};
use fabric::types::admission::RiskLevel;
use fabric::{
    AdmissionRequest, BudgetRequest, CapabilityId, CapabilityScope, ExitReason, LeaseRequest,
    OperationKind, OperationRequest, PrincipalId, ProcessSignal, SandboxRequirement, SpawnSpec,
};
use std::sync::Arc;

async fn running_process_with_operation(
    runtime: &KernelRuntime,
) -> (fabric::ProcessHandle, fabric::OperationHandle) {
    let process = runtime.spawn_process(SpawnSpec::default()).await.unwrap();
    runtime
        .signal_process(process.id, ProcessSignal::Start)
        .await
        .unwrap();
    let operation = runtime
        .submit_operation(OperationRequest {
            owner: process.id,
            parent: None,
            kind: OperationKind::Turn,
            deadline: None,
        })
        .await
        .unwrap();
    runtime.start_operation(operation.id).await.unwrap();
    (process, operation)
}

#[tokio::test]
async fn terminal_transaction_cleans_all_owned_resources_before_publishing_exit() {
    let runtime = KernelRuntime::new();
    let (process, operation) = running_process_with_operation(&runtime).await;
    let space = runtime.inspect_process(process.id).await.unwrap().space;
    runtime
        .register_process_mailbox(
            process.id,
            Target::from("agent:test"),
            Arc::new(InProcessMailbox::with_capacity(4)),
        )
        .await
        .unwrap();
    let permit = runtime
        .admission()
        .admit(AdmissionRequest {
            operation_id: operation.id,
            process_id: process.id,
            principal: PrincipalId("owner".into()),
            capability: CapabilityId("tool".into()),
            action: "execute".into(),
            input_summary: String::new(),
            risk: RiskLevel::ReadOnly,
            requested_scope: CapabilityScope::default(),
            budget: Some(BudgetRequest {
                max_tokens: Some(50),
                max_cost_micro: Some(100),
            }),
            lease: Some(LeaseRequest {
                resource: "terminal-test".into(),
                duration_ms: 60_000,
            }),
            sandbox: SandboxRequirement::NotRequired,
        })
        .await
        .unwrap();
    assert_ne!(permit.id.0, uuid::Uuid::nil());

    let outcome = runtime
        .terminate_process(process.id, ExitReason::Failed("boom".into()))
        .await
        .unwrap();
    assert_eq!(outcome.restart_decision, RestartDecision::DoNotRestart);
    assert!(runtime
        .inspect_process(process.id)
        .await
        .unwrap()
        .state
        .is_terminal());
    assert!(runtime
        .inspect_operation(operation.id)
        .await
        .unwrap()
        .state
        .is_terminal());
    assert!(runtime.inspect_space(space).is_none());
    assert_eq!(runtime.mailbox_service().len().await, 0);
    assert_eq!(
        runtime
            .lease_manager()
            .active_count(runtime.clock().mono_now().0)
            .await,
        0
    );
    assert_eq!(
        runtime.budget_controller().active_reservation_count().await,
        0
    );

    let retried = runtime
        .terminate_process(
            process.id,
            ExitReason::Failed("different retry payload".into()),
        )
        .await
        .unwrap();
    assert_eq!(retried, outcome);
}

#[tokio::test]
async fn supervision_restarts_once_inside_the_terminal_transaction() {
    let runtime = KernelRuntime::new();
    let (process, _) = running_process_with_operation(&runtime).await;
    runtime
        .supervise(
            process.id,
            RestartPolicy::RestartOnFailure { max_restarts: 1 },
        )
        .await;

    let outcome = runtime
        .terminate_process(process.id, ExitReason::Panic("crash".into()))
        .await
        .unwrap();
    assert_eq!(
        outcome.restart_decision,
        RestartDecision::Restart { attempt: 1 }
    );
    assert_eq!(outcome.restarted.len(), 1);
    assert_ne!(outcome.restarted[0].id, process.id);
    assert!(!runtime
        .inspect_process(outcome.restarted[0].id)
        .await
        .unwrap()
        .state
        .is_terminal());

    let retried = runtime
        .terminate_process(process.id, ExitReason::Panic("retry".into()))
        .await
        .unwrap();
    assert_eq!(retried.restarted, outcome.restarted);
}

#[tokio::test]
async fn retry_after_restart_spawn_failure_does_not_repeat_cleanup_or_decision() {
    let runtime = KernelRuntime::new();
    let parent = runtime.spawn_process(SpawnSpec::default()).await.unwrap();
    let child = runtime
        .spawn_process(SpawnSpec {
            parent: Some(parent.id),
            ..SpawnSpec::default()
        })
        .await
        .unwrap();
    runtime
        .supervise(
            child.id,
            RestartPolicy::RestartOnFailure { max_restarts: 1 },
        )
        .await;
    runtime
        .terminate_process(parent.id, ExitReason::Completed)
        .await
        .unwrap();

    let first = runtime
        .terminate_process(child.id, ExitReason::Failed("child crash".into()))
        .await
        .unwrap_err()
        .to_string();
    let reservations_after_first = runtime.budget_controller().active_reservation_count().await;
    let second = runtime
        .terminate_process(child.id, ExitReason::Failed("retry".into()))
        .await
        .unwrap_err()
        .to_string();
    assert_eq!(first, second);
    assert_eq!(
        runtime.budget_controller().active_reservation_count().await,
        reservations_after_first
    );
    assert!(runtime
        .inspect_process(child.id)
        .await
        .unwrap()
        .state
        .is_terminal());
}
