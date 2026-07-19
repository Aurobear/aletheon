use async_trait::async_trait;
use fabric::types::admission::RiskLevel;
use fabric::{
    BudgetRequest, CapabilityAuthority, CapabilityCall, CapabilityInvoker, CapabilityRequest,
    CapabilityResult, CapabilityScope, ExecutionPermit, ExitReason, InvocationControl,
    LeaseRequest, OperationKind, OperationRequest, PrincipalId, ProcessSignal, RevokeReason,
    SandboxRequirement, SpawnSpec, UsageReport,
};
use kernel::capability::{DefaultCapabilityInvoker, ToolExecutor};
use kernel::chronos::TestClock;
use kernel::KernelRuntime;
use std::sync::Arc;
use tokio::sync::Notify;
use tokio_util::sync::CancellationToken;

struct ResultExecutor {
    fail: bool,
}

#[async_trait]
impl ToolExecutor for ResultExecutor {
    async fn execute_with_permit(
        &self,
        request: &CapabilityRequest,
        permit: &ExecutionPermit,
    ) -> CapabilityResult {
        CapabilityResult {
            call_id: request.call.call_id.clone(),
            output: if self.fail { "tool failed" } else { "ok" }.into(),
            is_error: self.fail,
            usage: UsageReport {
                permit_id: permit.id,
                tokens_used: 3,
                ..Default::default()
            },
            audit_id: Some(fabric::AuditEventId::new()),
            patch_delta: None,
        }
    }
}

struct BlockingExecutor {
    started: Arc<Notify>,
}

#[async_trait]
impl ToolExecutor for BlockingExecutor {
    async fn execute_with_permit(
        &self,
        _request: &CapabilityRequest,
        _permit: &ExecutionPermit,
    ) -> CapabilityResult {
        self.started.notify_one();
        std::future::pending().await
    }
}

struct Scenario {
    kernel: Arc<KernelRuntime>,
    process: fabric::ProcessHandle,
    operation: fabric::OperationHandle,
}

impl Scenario {
    async fn running() -> Self {
        let kernel = Arc::new(KernelRuntime::with_clock(Arc::new(TestClock::default())));
        let process = kernel.spawn_process(SpawnSpec::default()).await.unwrap();
        kernel
            .signal_process(process.id, ProcessSignal::Start)
            .await
            .unwrap();
        let operation = kernel
            .submit_operation(OperationRequest {
                owner: process.id,
                parent: None,
                kind: OperationKind::Turn,
                deadline: None,
            })
            .await
            .unwrap();
        kernel.start_operation(operation.id).await.unwrap();
        Self {
            kernel,
            process,
            operation,
        }
    }

    fn request(&self, cancel: CancellationToken) -> CapabilityRequest {
        CapabilityRequest {
            call: CapabilityCall {
                operation_id: self.operation.id,
                process_id: self.process.id,
                name: "scenario_tool".into(),
                input: serde_json::json!({}),
                call_id: "call-1".into(),
                deadline: None,
            },
            authority: CapabilityAuthority {
                agent: None,
                principal: PrincipalId("scenario".into()),
                action: "execute".into(),
                requested_scope: CapabilityScope::default(),
                risk: RiskLevel::ReadOnly,
                budget: Some(BudgetRequest {
                    max_tokens: Some(10),
                    max_cost_micro: None,
                }),
                lease: Some(LeaseRequest {
                    resource: "scenario-resource".into(),
                    duration_ms: 1_000,
                }),
                sandbox: SandboxRequirement::NotRequired,
                connection_id: fabric::ConnectionId::new(),
                thread_id: fabric::ThreadId("scenario".into()),
                turn_id: fabric::TurnId::new(),
                workspace: fabric::WorkspacePolicy::from_resolved_roots(
                    std::env::temp_dir(),
                    vec![],
                )
                .unwrap(),
                session_id: "scenario".into(),
                working_dir: std::env::temp_dir(),
            },
            control: InvocationControl {
                cancel,
                turn_event_sender: None,
            },
        }
    }

    async fn assert_no_capability_residue(&self, baseline_budget: usize) {
        assert_eq!(
            self.kernel
                .active_permits_for_process(self.process.id)
                .await,
            0
        );
        assert_eq!(
            self.kernel
                .lease_manager()
                .active_count(self.kernel.clock().mono_now().0)
                .await,
            0
        );
        assert_eq!(
            self.kernel
                .budget_controller()
                .active_reservation_count()
                .await,
            baseline_budget
        );
        assert_eq!(self.kernel.mailbox_service().len().await, 0);
    }
}

#[tokio::test]
async fn successful_turn_settles_capability_while_process_stays_healthy() {
    let scenario = Scenario::running().await;
    let baseline = scenario
        .kernel
        .budget_controller()
        .active_reservation_count()
        .await;
    let invoker = DefaultCapabilityInvoker::new(
        scenario.kernel.admission(),
        Arc::new(ResultExecutor { fail: false }),
    );
    let result = invoker
        .invoke(scenario.request(CancellationToken::new()))
        .await;
    assert!(!result.is_error);
    scenario
        .kernel
        .succeed_operation(scenario.operation.id)
        .await
        .unwrap();
    scenario.assert_no_capability_residue(baseline).await;
    assert_eq!(
        scenario
            .kernel
            .active_operation_count(scenario.process.id)
            .await,
        0
    );
    assert!(!scenario
        .kernel
        .inspect_process(scenario.process.id)
        .await
        .unwrap()
        .state
        .is_terminal());
}

#[tokio::test]
async fn tool_failure_settles_once_and_parent_cancel_cancels_descendants() {
    let scenario = Scenario::running().await;
    let child = scenario
        .kernel
        .submit_operation(OperationRequest {
            owner: scenario.process.id,
            parent: Some(scenario.operation.id),
            kind: OperationKind::CapabilityCall,
            deadline: None,
        })
        .await
        .unwrap();
    scenario.kernel.start_operation(child.id).await.unwrap();
    let baseline = scenario
        .kernel
        .budget_controller()
        .active_reservation_count()
        .await;
    let invoker = DefaultCapabilityInvoker::new(
        scenario.kernel.admission(),
        Arc::new(ResultExecutor { fail: true }),
    );
    let result = invoker
        .invoke(scenario.request(CancellationToken::new()))
        .await;
    assert!(result.is_error);
    assert!(scenario
        .kernel
        .admission()
        .settle(result.usage.permit_id, result.usage.clone())
        .await
        .is_err());
    scenario
        .kernel
        .cancel_operation(scenario.operation.id, fabric::CancelReason::User)
        .await
        .unwrap();
    assert!(scenario
        .kernel
        .inspect_operation(child.id)
        .await
        .unwrap()
        .state
        .is_terminal());
    scenario.assert_no_capability_residue(baseline).await;
}

#[tokio::test]
async fn user_cancellation_revokes_one_permit_and_releases_every_hold() {
    let scenario = Scenario::running().await;
    let baseline = scenario
        .kernel
        .budget_controller()
        .active_reservation_count()
        .await;
    let started = Arc::new(Notify::new());
    let invoker = Arc::new(DefaultCapabilityInvoker::new(
        scenario.kernel.admission(),
        Arc::new(BlockingExecutor {
            started: started.clone(),
        }),
    ));
    let cancel = CancellationToken::new();
    let request = scenario.request(cancel.clone());
    let task = tokio::spawn(async move { invoker.invoke(request).await });
    started.notified().await;
    assert_eq!(
        scenario
            .kernel
            .active_permits_for_process(scenario.process.id)
            .await,
        1
    );
    cancel.cancel();
    let result = task.await.unwrap();
    assert!(result.is_error);
    assert!(result.output.contains("cancelled"));
    scenario
        .kernel
        .admission()
        .revoke(result.usage.permit_id, RevokeReason::OperationCancelled)
        .await
        .unwrap();
    scenario.assert_no_capability_residue(baseline).await;
}

#[tokio::test]
async fn reconstructed_kernel_rejects_terminal_operation_and_identity_ids() {
    let original = Scenario::running().await;
    let stale_process = original.process.id;
    let stale_operation = original.operation.id;
    original
        .kernel
        .terminate_process(stale_process, ExitReason::Completed)
        .await
        .unwrap();

    let reconstructed = KernelRuntime::with_clock(Arc::new(TestClock::default()));
    assert!(reconstructed
        .inspect_operation(stale_operation)
        .await
        .is_err());
    assert!(reconstructed
        .submit_operation(OperationRequest {
            owner: stale_process,
            parent: None,
            kind: OperationKind::Turn,
            deadline: None,
        })
        .await
        .is_err());
    assert!(reconstructed
        .bind_os_process_id(stale_process, fabric::OsProcessId(42))
        .await
        .is_err());
}
