use std::sync::Arc;

use executive::application::evaluation::{
    EvaluationReceiptStore, EvaluationService, TurnEvaluationArtifacts,
};
use executive::application::turn_coordinator::{TurnCoordinator, TurnExecution};
use executive::application::turn_diff_tracker::TurnFileDeltaSnapshot;
use executive::application::turn_policy::TurnPolicy;
use executive::runtime::evaluation::SqliteEvaluationStore;
use executive::runtime::session::canonical_store::CanonicalSessionStore;
use fabric::{
    CapabilityRetryDisposition, CapabilityTerminalReceipt, CapabilityTerminalStatus,
    EvaluationDecision, EvaluationMode, ItemPayload, MonoTime, OperationId, SessionAppendStore,
    SessionId, TaskKind, TurnMetrics, TurnRequest, TurnResult, TurnStop,
};
use kernel::KernelRuntime;

struct Fixture {
    kernel: Arc<KernelRuntime>,
    store: Arc<dyn SessionAppendStore>,
    evaluations: Arc<SqliteEvaluationStore>,
    coordinator: TurnCoordinator,
    process_id: fabric::ProcessId,
    thread: String,
}

impl Fixture {
    async fn new(mode: EvaluationMode, thread: &str) -> Self {
        let kernel = Arc::new(KernelRuntime::new());
        let evaluations = Arc::new(SqliteEvaluationStore::in_memory().unwrap());
        let settings = executive::composition::config::EvaluationSettings {
            enabled: true,
            default_mode: mode,
            ..Default::default()
        };
        let evaluation = Arc::new(
            EvaluationService::new(kernel.clone(), settings, evaluations.clone()).unwrap(),
        );
        let coordinator = executive::testing::turn_coordinator::compose_in_memory_turn_coordinator(
            kernel.clone(),
            Arc::new(CanonicalSessionStore::open(":memory:").unwrap()),
        )
        .with_evaluation_service(evaluation);
        let store = coordinator.store();
        let process_id = kernel
            .spawn_process(fabric::SpawnSpec::default())
            .await
            .unwrap()
            .id;
        Self {
            kernel,
            store,
            evaluations,
            coordinator,
            process_id,
            thread: thread.into(),
        }
    }

    fn request(&self) -> TurnRequest {
        TurnRequest {
            operation_id: OperationId::default(),
            process_id: self.process_id,
            context: turn_request_support::context(&self.thread, std::env::temp_dir()),
            input: "make the typed code change".into(),
            execution_target: fabric::ExecutionTargetSelection::default(),
            model_policy: None,
            deadline: None,
            requirements: vec![],
            requested_task_kind: Some(TaskKind::Coding),
            evaluation_contract: None,
        }
    }

    async fn submit(&self, validation_status: CapabilityTerminalStatus) -> TurnResult {
        self.coordinator
            .submit_with(
                self.request(),
                &TurnPolicy::daemon(),
                move |request, _| async move {
                    assert!(request.evaluation_contract.is_some());
                    let receipt = CapabilityTerminalReceipt {
                        invocation_id: "validation-1".into(),
                        operation_id: request.operation_id,
                        process_id: request.process_id,
                        capability: "validation_run".into(),
                        status: validation_status,
                        started_at: MonoTime(1),
                        finished_at: MonoTime(2),
                        exit_code: Some(
                            if validation_status == CapabilityTerminalStatus::Succeeded {
                                0
                            } else {
                                1
                            },
                        ),
                        error_class: None,
                        artifact_ids: vec![],
                        evidence_ids: vec![],
                        output_ref: None,
                        truncated: false,
                        retry_disposition: CapabilityRetryDisposition::Never,
                        audit_id: None,
                    };
                    Ok(TurnExecution {
                        result: TurnResult {
                            output: "runner output".into(),
                            stop: TurnStop::Completed,
                            failure: None,
                            usage: Default::default(),
                            metrics: TurnMetrics {
                                completed_normally: true,
                                ..Default::default()
                            },
                        },
                        items: vec![],
                        projection: None,
                        context_projection: None,
                        evaluation_artifacts: TurnEvaluationArtifacts {
                            session_id: request.context.thread_id.0.clone(),
                            runtime_id: "test-runtime".into(),
                            effective_model_id: "test-provider/test-model".into(),
                            model_display_name: "test-model".into(),
                            workspace: Some(request.context.workspace.clone()),
                            profile_name: "code-agent".into(),
                            capability_receipts: vec![receipt],
                            file_deltas: vec![TurnFileDeltaSnapshot {
                                path: "src/lib.rs".into(),
                                edits: 1,
                                hunks_applied: 1,
                                bytes_before: 1,
                                bytes_after: 2,
                            }],
                            runtime_faults: vec![],
                            supplemental_evidence: vec![],
                            projection_metrics: Default::default(),
                        },
                    })
                },
            )
            .await
            .unwrap()
    }

    async fn receipt(&self) -> fabric::EvaluationReceipt {
        let items = self
            .store
            .load_items(&SessionId(self.thread.clone()), None)
            .await
            .unwrap();
        let reference = items
            .iter()
            .find_map(|item| match &item.payload {
                ItemPayload::EvaluationReceiptRef { receipt } => Some(receipt.clone()),
                _ => None,
            })
            .expect("session stores the receipt reference");
        self.evaluations
            .get_receipt(&reference.receipt_id)
            .await
            .unwrap()
            .expect("full receipt is durable before its Session reference")
    }
}

#[tokio::test]
async fn shadow_failure_persists_receipt_before_completed_turn() {
    let fixture = Fixture::new(EvaluationMode::Shadow, "evaluation-shadow").await;
    let result = fixture.submit(CapabilityTerminalStatus::Failed).await;
    assert_eq!(result.stop, TurnStop::Completed);
    let receipt = fixture.receipt().await;
    assert_eq!(receipt.decision, EvaluationDecision::ObservedFail);
    assert_eq!(receipt.execution.runtime_id, "test-runtime");
    assert_eq!(
        receipt.execution.effective_model_id,
        "test-provider/test-model"
    );
    assert_eq!(receipt.execution.agent_profile, "code-agent");
    assert_eq!(receipt.execution.workspace_boundary_sha256.len(), 64);
    assert_eq!(receipt.execution.verification_selection_sha256.len(), 64);
    assert_eq!(
        fixture
            .kernel
            .inspect_operation(receipt.evaluation_operation_id)
            .await
            .unwrap()
            .state,
        fabric::OperationState::Succeeded
    );
}

#[tokio::test]
async fn enforce_failure_cannot_report_completed() {
    let fixture = Fixture::new(EvaluationMode::Enforce, "evaluation-enforce").await;
    let result = fixture.submit(CapabilityTerminalStatus::Failed).await;
    assert_eq!(result.stop, TurnStop::Blocked);
    assert_eq!(
        fixture.receipt().await.decision,
        EvaluationDecision::Rejected
    );
}

#[tokio::test]
async fn successful_coding_evaluation_accepts_enforce_turn() {
    let fixture = Fixture::new(EvaluationMode::Enforce, "evaluation-pass").await;
    let result = fixture.submit(CapabilityTerminalStatus::Succeeded).await;
    assert_eq!(result.stop, TurnStop::Completed);
    assert_eq!(
        fixture.receipt().await.decision,
        EvaluationDecision::Accepted
    );
}

mod turn_request_support;
