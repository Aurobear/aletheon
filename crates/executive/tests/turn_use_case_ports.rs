use std::sync::Arc;

use async_trait::async_trait;
use executive::runtime::session::canonical_store::CanonicalSessionStore;
use executive::service::post_turn_projection::{
    PostTurnDispatch, PostTurnOutcome, PostTurnProjection,
};
use executive::service::turn_coordinator::{TurnCoordinator, TurnExecution};
use executive::service::turn_policy::TurnPolicy;
use fabric::{
    ItemPayload, OperationId, OperationState, SessionAppendStore, SessionId, TurnMetrics,
    TurnRequest, TurnResult, TurnStop,
};
use kernel::KernelRuntime;
use tokio::sync::{oneshot, Mutex};

struct FailingProjection {
    kernel: Arc<KernelRuntime>,
    store: Arc<dyn SessionAppendStore>,
    operation: Arc<Mutex<Option<OperationId>>>,
    observed: Mutex<Option<oneshot::Sender<(OperationState, usize, bool)>>>,
}

#[async_trait]
impl PostTurnProjection for FailingProjection {
    async fn project(&self, outcome: PostTurnOutcome) -> anyhow::Result<()> {
        let operation = self.operation.lock().await.expect("operation captured");
        let state = self.kernel.inspect_operation(operation).await?.state;
        let items = self
            .store
            .load_items(&SessionId(outcome.session_id), None)
            .await?;
        let terminal_is_assistant = items
            .last()
            .is_some_and(|item| matches!(item.payload, ItemPayload::AssistantMessage { .. }));
        if let Some(sender) = self.observed.lock().await.take() {
            let _ = sender.send((state, items.len(), terminal_is_assistant));
        }
        anyhow::bail!("projection backend unavailable")
    }
}

#[tokio::test]
async fn projection_runs_after_terminal_settlement_and_cannot_fail_the_turn() {
    let kernel = Arc::new(KernelRuntime::new());
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let coordinator = TurnCoordinator::new(kernel.clone(), store.clone());
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let operation = Arc::new(Mutex::new(None));
    let (observed_tx, observed_rx) = oneshot::channel();
    let projector = Arc::new(FailingProjection {
        kernel: kernel.clone(),
        store: store.clone(),
        operation: operation.clone(),
        observed: Mutex::new(Some(observed_tx)),
    });
    let request = TurnRequest {
        operation_id: OperationId::default(),
        process_id: process.id,
        context: turn_request_support::context("projection-order", std::env::temp_dir()),
        input: "hello".into(),
        model_policy: None,
        deadline: None,
    };

    let result = coordinator
        .submit_with(request, &TurnPolicy::daemon(), move |request, _| {
            let operation = operation.clone();
            let projector = projector.clone();
            async move {
                *operation.lock().await = Some(request.operation_id);
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "answer".into(),
                        stop: TurnStop::Completed,
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![],
                    projection: Some(PostTurnDispatch {
                        projector,
                        outcome: PostTurnOutcome {
                            session_id: request.context.thread_id.0,
                            input: request.input,
                            output: "answer".into(),
                            turn: 1,
                            succeeded: true,
                            tool_calls_made: 0,
                            tool_errors: 0,
                            elapsed_ms: 1,
                            iterations: 1,
                            completed_normally: true,
                            agora_start_version: 0,
                        },
                    }),
                    context_projection: None,
                })
            }
        })
        .await
        .expect("projection outage must not fail a settled turn");
    assert_eq!(result.output, "answer");

    let (state, item_count, terminal_is_assistant) =
        tokio::time::timeout(std::time::Duration::from_secs(1), observed_rx)
            .await
            .expect("projection should run")
            .expect("projection observation channel");
    assert_eq!(state, OperationState::Succeeded);
    assert_eq!(item_count, 2);
    assert!(terminal_is_assistant);
}

#[test]
fn turn_pipeline_has_no_direct_post_turn_domain_writes() {
    let pipeline = include_str!("../src/application/turn_pipeline.rs");
    for forbidden in [
        "extract_auto_memory(",
        "record_turn_reflection(",
        "run_post_evolution(",
        "commit_agora_snapshot(",
        "store_reflection(",
        "store_evolution_log(",
        "agora_commit persist",
    ] {
        assert!(
            !pipeline.contains(forbidden),
            "pipeline contains {forbidden}"
        );
    }
    assert!(pipeline.contains("post_turn_projection"));
    for forbidden in [
        "self.subsystems",
        "sm_arc.lock()",
        "storm_breaker.lock()",
        "hook_registry.lock()",
        "approval_rx.lock()",
        "pending_approvals.lock()",
    ] {
        assert!(
            !pipeline.contains(forbidden),
            "pipeline bypasses runtime port with {forbidden}"
        );
    }

    let context = pipeline.find(".assemble(&context_request").unwrap();
    let model = pipeline.find(".models.select").unwrap();
    let capability = pipeline.find(".capabilities").unwrap();
    assert!(context < model && model < capability);

    let coordinator = include_str!("../src/application/turn_coordinator.rs");
    let settlement = coordinator.find("terminal?;").unwrap();
    let projection = coordinator.find("dispatch.projector.project").unwrap();
    assert!(settlement < projection);

    let post_turn = include_str!("../src/application/post_turn_projection.rs");
    for forbidden in [
        "MemoryService",
        "AutoMemory",
        "RecallMemory",
        "analyze_and_store",
        "store_reflection",
        "store_evolution_log",
        "record_assistant_message",
        "persist_agora_commits",
    ] {
        assert!(
            !post_turn.contains(forbidden),
            "post-turn handler bypasses the event-driven projection path with {forbidden}"
        );
    }
}
mod turn_request_support;
