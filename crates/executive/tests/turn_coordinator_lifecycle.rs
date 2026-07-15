use std::sync::Arc;

use aletheon_kernel::service::ServicePorts;
use executive::r#impl::session::canonical_store::CanonicalSessionStore;
use executive::service::turn_coordinator::{TurnCoordinator, TurnExecution};
use executive::service::turn_policy::*;
use fabric::{
    ItemPayload, OperationState, SessionAppendStore, SessionId, TurnMetrics, TurnRequest,
    TurnResult, TurnStop,
};

fn request(session: &str) -> TurnRequest {
    TurnRequest {
        operation_id: fabric::OperationId::default(),
        process_id: fabric::ProcessId::new(),
        session_id: session.into(),
        input: "hello".into(),
        working_dir: std::env::temp_dir(),
        model_policy: None,
        deadline: None,
    }
}

#[test]
fn policy_contains_all_mode_differences() {
    let daemon = TurnPolicy::daemon();
    let exec = TurnPolicy::exec();
    assert_eq!(daemon.persistence, PersistenceMode::Durable);
    assert_eq!(exec.persistence, PersistenceMode::Durable);
    assert_ne!(daemon.reviewer, exec.reviewer);
    assert_ne!(daemon.memory_eligible, exec.memory_eligible);
    assert_ne!(daemon.agora_available, exec.agora_available);
    assert_ne!(daemon.event_delivery, exec.event_delivery);
    assert_ne!(daemon.environment, exec.environment);
}

#[tokio::test]
async fn coordinator_owns_turn_operation_and_ordered_canonical_items() {
    let ports = ServicePorts::new();
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let coordinator = TurnCoordinator::new(&ports, store.clone());
    let captured = Arc::new(tokio::sync::Mutex::new(None));
    let capture = captured.clone();
    let result = coordinator
        .submit_with(
            request("success"),
            &TurnPolicy::daemon(),
            move |request, _| async move {
                *capture.lock().await = Some(request.operation_id);
                Ok(TurnExecution {
                    result: TurnResult {
                        output: "answer".into(),
                        stop: TurnStop::Completed,
                        metrics: TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![
                        ItemPayload::ToolCall {
                            call_id: "c".into(),
                            name: "tool".into(),
                            input: serde_json::json!({}),
                        },
                        ItemPayload::ToolResult {
                            call_id: "c".into(),
                            content: "ok".into(),
                            is_error: false,
                            permit_id: None,
                            audit_id: None,
                        },
                    ],
                })
            },
        )
        .await
        .unwrap();
    assert_eq!(result.output, "answer");
    let operation = ports
        .operation_table
        .inspect(captured.lock().await.unwrap())
        .await
        .unwrap();
    assert_eq!(operation.kind, fabric::OperationKind::Turn);
    assert_eq!(operation.state, OperationState::Succeeded);
    let items = store
        .load_items(&SessionId("success".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 4);
    assert!(matches!(items[0].payload, ItemPayload::UserMessage { .. }));
    assert!(matches!(items[1].payload, ItemPayload::ToolCall { .. }));
    assert!(matches!(items[2].payload, ItemPayload::ToolResult { .. }));
    assert!(matches!(
        items[3].payload,
        ItemPayload::AssistantMessage { .. }
    ));
}

#[tokio::test]
async fn failure_is_terminal_and_remains_replayable() {
    let ports = ServicePorts::new();
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let coordinator = TurnCoordinator::new(&ports, store.clone());
    let result = coordinator
        .submit_with(
            request("failed"),
            &TurnPolicy::exec(),
            |_request, _| async { anyhow::bail!("model unavailable") },
        )
        .await;
    assert!(result.is_err());
    let items = store
        .load_items(&SessionId("failed".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 2);
    assert!(matches!(items[0].payload, ItemPayload::UserMessage { .. }));
    assert!(matches!(items[1].payload, ItemPayload::SystemNotice { .. }));
}
