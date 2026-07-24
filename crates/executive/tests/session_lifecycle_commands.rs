use std::sync::Arc;

use executive::application::session_service::{InterruptOutcome, SessionService};
use executive::application::turn_coordinator::{cancelled_result, ActiveTurnKey, TurnExecution};
use executive::application::turn_policy::TurnPolicy;
use executive::runtime::events::{EventReadFilter, SqliteEventSpine};
use executive::runtime::session::canonical_store::CanonicalSessionStore;
use fabric::{SessionAppendStore, SessionId, TurnRequest};
use kernel::KernelRuntime;

fn request(session: &str, process_id: fabric::ProcessId) -> TurnRequest {
    TurnRequest {
        operation_id: fabric::OperationId::default(),
        process_id,
        context: turn_request_support::context(session, std::env::temp_dir()),
        input: "hello".into(),
        model_policy: None,
        deadline: None,
    }
}

#[tokio::test]
async fn resume_fork_replay_and_interrupt_share_canonical_state() {
    let kernel = Arc::new(KernelRuntime::new());
    let process = kernel
        .spawn_process(fabric::SpawnSpec::default())
        .await
        .unwrap();
    let store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(":memory:").unwrap());
    let event_spine = Arc::new(SqliteEventSpine::open(":memory:").unwrap());
    let coordinator = Arc::new(
        executive::testing::turn_coordinator::compose_with_event_spine(
            kernel,
            store.clone(),
            event_spine.clone(),
            executive::composition::config::GrokHardeningConfig::default(),
        ),
    );
    coordinator
        .submit_with(
            request("base", process.id),
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
                    projection: None,
                    context_projection: None,
                })
            },
        )
        .await
        .unwrap();
    let service = SessionService::new(coordinator.store(), coordinator.active_index());
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
    let fork_events = event_spine
        .read_tree(
            fabric::EventTreeId::for_root_session(&child.id.0),
            EventReadFilter {
                limit: 10,
                ..Default::default()
            },
        )
        .unwrap();
    assert_eq!(fork_events.len(), 1);
    assert_eq!(
        fork_events[0].schema.0,
        fabric::SchemaId::EVENT_SESSION_FORKED_V1
    );

    let running = coordinator.clone();
    let active_request = request("active", process.id);
    let active_key = ActiveTurnKey::from_context(&active_request.context);
    let task = tokio::spawn(async move {
        running
            .submit_with(
                active_request,
                &TurnPolicy::daemon(),
                |_request, cancel| async move {
                    cancel.cancelled().await;
                    Ok(TurnExecution {
                        result: cancelled_result(),
                        items: vec![],
                        projection: None,
                        context_projection: None,
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
            .contains_key(&active_key)
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
mod turn_request_support;
