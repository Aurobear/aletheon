use std::sync::Arc;

use aletheon_kernel::service::ServicePorts;
use executive::r#impl::session::canonical_store::CanonicalSessionStore;
use executive::service::session_service::{InterruptOutcome, SessionService};
use executive::service::turn_coordinator::{cancelled_result, TurnCoordinator, TurnExecution};
use executive::service::turn_policy::TurnPolicy;
use fabric::{SessionAppendStore, SessionId, TurnRequest};

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

#[tokio::test]
async fn resume_fork_replay_and_interrupt_share_canonical_state() {
    let ports = Arc::new(ServicePorts::new());
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let coordinator = Arc::new(TurnCoordinator::new(ports.as_ref(), store.clone()));
    coordinator
        .submit_with(
            request("base"),
            &TurnPolicy::daemon(),
            |_request, _| async {
                Ok(TurnExecution {
                    result: fabric::TurnResult {
                        output: "answer".into(),
                        stop: fabric::TurnStop::Completed,
                        metrics: fabric::TurnMetrics {
                            completed_normally: true,
                            ..Default::default()
                        },
                    },
                    items: vec![],
                })
            },
        )
        .await
        .unwrap();
    let service = SessionService::new(
        store.clone(),
        ports.operation_table.clone(),
        coordinator.active_index(),
    );
    let resumed = service.resume(&SessionId("base".into())).await.unwrap();
    assert_eq!(resumed.next_sequence, 3);
    assert_eq!(resumed.messages.len(), 2);
    let replay_a = serde_json::to_vec(
        &service
            .replay(&SessionId("base".into()), None)
            .await
            .unwrap(),
    )
    .unwrap();
    let replay_b = serde_json::to_vec(
        &service
            .replay(&SessionId("base".into()), None)
            .await
            .unwrap(),
    )
    .unwrap();
    assert_eq!(replay_a, replay_b);
    let child = service.fork(&SessionId("base".into()), 1).await.unwrap();
    assert_eq!(child.parent.as_ref().unwrap().through_sequence, 1);
    assert_eq!(store.load_items(&child.id, None).await.unwrap().len(), 1);

    let running = coordinator.clone();
    let task = tokio::spawn(async move {
        running
            .submit_with(
                request("active"),
                &TurnPolicy::daemon(),
                |_request, cancel| async move {
                    cancel.cancelled().await;
                    Ok(TurnExecution {
                        result: cancelled_result(),
                        items: vec![],
                    })
                },
            )
            .await
    });
    for _ in 0..100 {
        if coordinator
            .active_index()
            .lock()
            .await
            .contains_key("active")
        {
            break;
        }
        tokio::task::yield_now().await;
    }
    assert_eq!(
        service
            .interrupt(&SessionId("active".into()))
            .await
            .unwrap(),
        InterruptOutcome::Interrupted
    );
    assert!(task.await.unwrap().is_ok());
    assert_eq!(
        service
            .interrupt(&SessionId("active".into()))
            .await
            .unwrap(),
        InterruptOutcome::AlreadyTerminal
    );
    let items = store
        .load_items(&SessionId("active".into()), None)
        .await
        .unwrap();
    assert_eq!(items.len(), 2);
}
