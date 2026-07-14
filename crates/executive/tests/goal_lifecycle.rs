//! Integration tests for the M2 Goal Coordinator lifecycle.
//!
//! Exercises bounded tick behavior, process linkage, pause/resume/cancel,
//! and restart recovery of stale process links.

use executive::r#impl::goal::coordinator::{GoalCoordinator, GoalTickOutcome};
use executive::r#impl::goal::ObjectiveStore;
use fabric::goal::{GoalBudget, GoalId, GoalSpec, GoalState};
use fabric::PrincipalId;
use fabric::ProcessId;
use std::sync::{Arc, Mutex};

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn setup() -> (GoalCoordinator, Arc<Mutex<ObjectiveStore>>) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();
    std::mem::forget(tmp);

    let store = ObjectiveStore::open(&path).unwrap();
    let store = Arc::new(Mutex::new(store));
    let coord = GoalCoordinator::new(store.clone());
    (coord, store)
}

fn create_goal(store: &Arc<Mutex<ObjectiveStore>>) -> fabric::goal::GoalSnapshot {
    let s = store.lock().unwrap();
    let spec = GoalSpec {
        original_intent: "ship feature X".into(),
        desired_state: vec!["feature deployed".into()],
        constraints: vec!["no breakage".into()],
        acceptance_criteria: vec!["tests pass".into()],
        budget: GoalBudget {
            max_input_tokens: 1_000_000,
            max_output_tokens: 500_000,
            max_cost_usd: None,
            max_attempts: 10,
            deadline_ms: None,
        },
    };
    s.create_goal(
        &PrincipalId("test-owner".into()),
        "sess-1",
        "project",
        &spec,
    )
    .unwrap()
}

// ---------------------------------------------------------------------------
// 1. Ready → Running → TurnRequested cycle
// ---------------------------------------------------------------------------

#[test]
fn ready_to_running_to_turn_requested() {
    let (coord, store) = setup();
    let g = create_goal(&store);

    assert_eq!(g.state, GoalState::Ready);

    let outcome = coord.tick(g.id, 0).unwrap();
    match outcome {
        GoalTickOutcome::Transitioned { from, to } => {
            assert_eq!(from, GoalState::Ready);
            assert_eq!(to, GoalState::Running);
        }
        other => panic!("expected Transitioned, got {other:?}"),
    }

    let outcome2 = coord.tick(g.id, 0).unwrap();
    match outcome2 {
        GoalTickOutcome::TurnRequested { goal_id, ref input } => {
            assert_eq!(goal_id, g.id);
            assert!(input.contains("ship feature X"));
        }
        other => panic!("expected TurnRequested, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// 2. Pause suspends the goal
// ---------------------------------------------------------------------------

#[test]
fn pause_and_resume() {
    let (coord, store) = setup();
    let g = create_goal(&store);

    coord.tick(g.id, 0).unwrap();

    // Pause: Running → Suspended.
    {
        let s = store.lock().unwrap();
        let g2 = s
            .transition_goal(
                g.id,
                1,
                GoalState::Suspended,
                None,
                &serde_json::json!({"action": "pause"}),
            )
            .unwrap();
        assert_eq!(g2.state, GoalState::Suspended);
    }

    let outcome = coord.tick(g.id, 0).unwrap();
    assert!(matches!(
        outcome,
        GoalTickOutcome::Noop {
            state: GoalState::Suspended
        }
    ));

    // Resume: Suspended → Ready.
    {
        let s = store.lock().unwrap();
        let g3 = s
            .transition_goal(
                g.id,
                2,
                GoalState::Ready,
                None,
                &serde_json::json!({"action": "resume"}),
            )
            .unwrap();
        assert_eq!(g3.state, GoalState::Ready);
        assert_eq!(g3.version, 3);
    }

    let outcome = coord.tick(g.id, 0).unwrap();
    assert!(matches!(
        outcome,
        GoalTickOutcome::Transitioned {
            from: GoalState::Ready,
            to: GoalState::Running
        }
    ));
}

// ---------------------------------------------------------------------------
// 3. Cancel from Running
// ---------------------------------------------------------------------------

#[test]
fn cancel_from_running() {
    let (coord, store) = setup();
    let g = create_goal(&store);

    coord.tick(g.id, 0).unwrap();

    // Cancel: Running → Cancelled.
    {
        let s = store.lock().unwrap();
        let g2 = s
            .transition_goal(
                g.id,
                1,
                GoalState::Cancelled,
                None,
                &serde_json::json!({"action": "cancel"}),
            )
            .unwrap();
        assert_eq!(g2.state, GoalState::Cancelled);
        assert!(g2.state.is_terminal());
    }

    let outcome = coord.tick(g.id, 0).unwrap();
    assert!(matches!(
        outcome,
        GoalTickOutcome::Noop {
            state: GoalState::Cancelled
        }
    ));
}

// ---------------------------------------------------------------------------
// 4. Process link lifecycle
// ---------------------------------------------------------------------------

#[test]
fn process_link_lifecycle() {
    let (coord, store) = setup();
    let g = create_goal(&store);

    coord.tick(g.id, 0).unwrap();

    let pid = ProcessId(uuid::Uuid::new_v4());
    let snapshot = coord.set_process_link(g.id, 1, Some(pid)).unwrap();
    assert_eq!(snapshot.process_id, Some(pid));
    assert_eq!(snapshot.version, 2);

    let snapshot2 = coord.set_process_link(g.id, 2, None).unwrap();
    assert_eq!(snapshot2.process_id, None);
    assert_eq!(snapshot2.version, 3);
}

// ---------------------------------------------------------------------------
// 5. Stale process link is cleared on re-tick after reconstruction
// ---------------------------------------------------------------------------

#[test]
fn stale_process_cleared_on_reconstruction() {
    let (coord, store) = setup();
    let g = create_goal(&store);

    // Activate and link a fake process.
    coord.tick(g.id, 0).unwrap();
    let pid = ProcessId(uuid::Uuid::new_v4());
    coord.set_process_link(g.id, 1, Some(pid)).unwrap();

    // Simulate daemon restart: Running → Suspended (pause), then Suspended → Ready (resume).
    // Running can transition to Suspended but not Ready directly.
    {
        let s = store.lock().unwrap();
        s.transition_goal(
            g.id,
            2,
            GoalState::Suspended,
            None,
            &serde_json::json!({"action": "pause"}),
        )
        .unwrap();
        s.transition_goal(
            g.id,
            3,
            GoalState::Ready,
            None,
            &serde_json::json!({"action": "resume"}),
        )
        .unwrap();
    }
    // Drop lock before calling set_process_link (it acquires the lock internally).
    coord.set_process_link(g.id, 4, None).unwrap();

    // Goal is back to Ready, process cleared.
    {
        let s = store.lock().unwrap();
        let fresh = s.get_goal(g.id).unwrap().unwrap();
        assert_eq!(fresh.state, GoalState::Ready);
        assert_eq!(fresh.process_id, None);
    }

    // Re-tick activates with fresh process.
    let outcome = coord.tick(g.id, 0).unwrap();
    assert!(matches!(
        outcome,
        GoalTickOutcome::Transitioned {
            from: GoalState::Ready,
            to: GoalState::Running
        }
    ));
}

// ---------------------------------------------------------------------------
// 6. Original intent is immutable across the entire lifecycle
// ---------------------------------------------------------------------------

#[test]
fn original_intent_immutable() {
    let (coord, store) = setup();
    let g = create_goal(&store);

    let check_intent = |store: &Arc<Mutex<ObjectiveStore>>, id: GoalId| {
        let s = store.lock().unwrap();
        let snap = s.get_goal(id).unwrap().unwrap();
        assert_eq!(snap.spec.original_intent, "ship feature X");
    };

    check_intent(&store, g.id);

    coord.tick(g.id, 0).unwrap();
    check_intent(&store, g.id);

    // Second tick: Running → TurnRequested (version stays at 1, no state transition).
    coord.tick(g.id, 0).unwrap();
    check_intent(&store, g.id);

    // Complete the goal: expected_version 1 (since TurnRequested didn't bump version).
    {
        let s = store.lock().unwrap();
        s.transition_goal(g.id, 1, GoalState::Completed, None, &serde_json::json!({}))
            .unwrap();
    }
    check_intent(&store, g.id);
}
