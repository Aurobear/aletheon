use std::sync::Arc;

use executive::r#impl::goal::ObjectiveStore;
use executive::service::goal_service::{GoalAction, GoalService, GoalServiceError, GoalUseCases};
use fabric::{GoalSpec, GoalState, PrincipalId};
use tempfile::tempdir;
use tokio::sync::Mutex;

fn setup() -> (GoalService, Arc<Mutex<ObjectiveStore>>) {
    let directory = tempdir().expect("temporary goal directory");
    let path = directory.keep().join("goals.db");
    let store = Arc::new(Mutex::new(
        ObjectiveStore::open(&path).expect("open objective store"),
    ));
    (GoalService::new(store.clone()), store)
}

fn spec(intent: &str) -> GoalSpec {
    GoalSpec {
        original_intent: intent.into(),
        desired_state: vec!["delivered".into()],
        constraints: vec!["preserve compatibility".into()],
        acceptance_criteria: vec!["tests pass".into()],
        budget: Default::default(),
    }
}

#[tokio::test]
async fn legacy_operations_preserve_objective_behavior() {
    let (service, _) = setup();
    let id = service
        .create_legacy(
            "ship the change".into(),
            "session-1".into(),
            "session".into(),
        )
        .await
        .unwrap();

    let detail = service.show_legacy(id).await.unwrap();
    assert_eq!(detail.objective.description, "ship the change");
    assert!(detail.sub_goals.is_empty());
    assert_eq!(service.list_legacy(None).await.unwrap().len(), 1);
    assert!(service
        .set_legacy_status(id, "completed".into())
        .await
        .unwrap());
    assert!(service.resume_legacy().await.unwrap().is_none());
    assert_eq!(
        service.show_legacy(i64::MAX).await.unwrap_err(),
        GoalServiceError::NotFound
    );
}

#[tokio::test]
async fn versioned_create_and_list_keep_original_intent() {
    let (service, _) = setup();
    let created = service
        .create_goal(
            PrincipalId("owner".into()),
            "session-1".into(),
            "project".into(),
            spec("immutable intent"),
        )
        .await
        .unwrap();

    assert_eq!(created.state, GoalState::Ready);
    assert_eq!(created.version, 0);
    let listed = service.list_goals(20).await.unwrap();
    assert_eq!(listed.len(), 1);
    assert_eq!(listed[0].spec.original_intent, "immutable intent");
}

#[tokio::test]
async fn legal_pause_run_and_cancel_transitions_are_versioned() {
    let (service, store) = setup();
    let created = service
        .create_goal(
            PrincipalId("owner".into()),
            "session-1".into(),
            "session".into(),
            spec("run lifecycle"),
        )
        .await
        .unwrap();
    store
        .lock()
        .await
        .transition_goal(
            created.id,
            0,
            GoalState::Running,
            None,
            &serde_json::json!({"test": "start"}),
        )
        .unwrap();

    let suspended = service
        .act(created.id, GoalAction::Pause, None)
        .await
        .unwrap();
    assert_eq!(suspended.state, GoalState::Suspended);
    let ready = service
        .act(created.id, GoalAction::Run, None)
        .await
        .unwrap();
    assert_eq!(ready.state, GoalState::Ready);
    let cancelled = service
        .act(created.id, GoalAction::Cancel, None)
        .await
        .unwrap();
    assert_eq!(cancelled.state, GoalState::Cancelled);
    assert!(cancelled.version > created.version);

    assert!(matches!(
        service.act(created.id, GoalAction::Cancel, None).await,
        Err(GoalServiceError::InvalidTransition(_))
    ));
}

#[tokio::test]
async fn illegal_and_stale_transitions_are_sanitized() {
    let (service, store) = setup();
    let created = service
        .create_goal(
            PrincipalId("owner".into()),
            "session-1".into(),
            "session".into(),
            spec("reject bad transitions"),
        )
        .await
        .unwrap();

    assert!(matches!(
        service.act(created.id, GoalAction::Pause, None).await,
        Err(GoalServiceError::InvalidTransition(_))
    ));
    store
        .lock()
        .await
        .transition_goal(
            created.id,
            0,
            GoalState::Running,
            None,
            &serde_json::json!({"test": "advance version"}),
        )
        .unwrap();
    assert!(matches!(
        service.act(created.id, GoalAction::Pause, Some(0)).await,
        Err(GoalServiceError::Conflict(_))
    ));
}

#[test]
fn goal_rpc_has_no_concrete_store_or_lock_access() {
    let source = include_str!("../src/host/daemon/handler/rpc/rpc_goal.rs");
    assert!(source.contains("self.ports.goals"));
    for forbidden in ["subsystems", "ObjectiveStore", "objective_store", ".lock()"] {
        assert!(
            !source.contains(forbidden),
            "goal RPC must not contain {forbidden}"
        );
    }
}
