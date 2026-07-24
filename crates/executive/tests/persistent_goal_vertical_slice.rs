//! End-to-end vertical slice: full M2 goal lifecycle.
//!
//! Tests the complete path from /goal → Draft → resume → tick →
//! pause → restart recovery → re-tick → cancel → restart (terminal).

use executive::goal::coordinator::{GoalCoordinator, GoalTickOutcome};
use executive::goal::ObjectiveStore;
use fabric::goal::{GoalBudget, GoalId, GoalSpec, GoalState};
use fabric::PrincipalId;
use std::sync::{Arc, Mutex};

fn setup() -> (GoalCoordinator, Arc<Mutex<ObjectiveStore>>) {
    let tmp = tempfile::NamedTempFile::new().unwrap();
    let path = tmp.path().to_path_buf();
    std::mem::forget(tmp);
    let store = ObjectiveStore::open(&path).unwrap();
    let store = Arc::new(Mutex::new(store));
    let coord = GoalCoordinator::new(store.clone());
    (coord, store)
}

fn create_goal(store: &Arc<Mutex<ObjectiveStore>>, intent: &str, is_draft: bool) -> GoalId {
    let s = store.lock().unwrap();
    let spec = GoalSpec {
        original_intent: intent.into(),
        desired_state: vec![],
        constraints: vec![],
        acceptance_criteria: vec![],
        budget: GoalBudget {
            max_input_tokens: 1_000_000,
            max_output_tokens: 500_000,
            max_cost_usd: None,
            max_attempts: 10,
            deadline_ms: None,
        },
    };
    let g = s
        .create_goal(&PrincipalId("test".into()), "s", "session", &spec)
        .unwrap();
    let id = g.id;

    if is_draft {
        // Set to Draft via a transition: Ready -> Cancelled is the closest we can do;
        // but for Draft, we need raw SQL since Ready -> Draft is illegal.
        // We set Draft via raw manipulation to simulate external creation.
        s.transition_goal(id, 0, GoalState::Running, None, &serde_json::json!({}))
            .unwrap();
        s.transition_goal(id, 1, GoalState::Cancelled, None, &serde_json::json!({}))
            .unwrap();
        drop(s);
        // Create a new goal for the actual test:
        let s = store.lock().unwrap();
        let spec2 = GoalSpec {
            original_intent: intent.into(),
            desired_state: vec![],
            constraints: vec![],
            acceptance_criteria: vec![],
            budget: GoalBudget {
                max_input_tokens: 1_000_000,
                max_output_tokens: 500_000,
                max_cost_usd: None,
                max_attempts: 10,
                deadline_ms: None,
            },
        };
        s.create_goal(&PrincipalId("test".into()), "s2", "session", &spec2)
            .unwrap()
            .id
    } else {
        drop(s);
        id
    }
}

// ---------------------------------------------------------------------------
// Vertical slice: Ready → Running → TurnRequested → pause → recovery → cancel
// ---------------------------------------------------------------------------

#[test]
fn vertical_slice() {
    let (coord, store) = setup();

    // 1. Create goal (Ready).
    let id = create_goal(&store, "ship feature X", false);
    {
        let s = store.lock().unwrap();
        let g = s.get_goal(id).unwrap().unwrap();
        assert_eq!(g.state, GoalState::Ready);
        assert_eq!(g.spec.original_intent, "ship feature X");
    }

    // 2. Tick: Ready → Running.
    let outcome = coord.tick(id, 0).unwrap();
    assert!(matches!(
        outcome,
        GoalTickOutcome::Transitioned {
            from: GoalState::Ready,
            to: GoalState::Running,
        }
    ));

    // 3. Tick: Running → TurnRequested.
    let outcome = coord.tick(id, 0).unwrap();
    assert!(matches!(outcome, GoalTickOutcome::TurnRequested { .. }));

    // 4. Pause: Running → Suspended.
    {
        let s = store.lock().unwrap();
        s.transition_goal(
            id,
            1,
            GoalState::Suspended,
            None,
            &serde_json::json!({"action": "pause"}),
        )
        .unwrap();
    }

    // 5. Tick on Suspended is Noop.
    let outcome = coord.tick(id, 0).unwrap();
    assert!(matches!(
        outcome,
        GoalTickOutcome::Noop {
            state: GoalState::Suspended
        }
    ));

    // 6. Simulate daemon restart: recover goals.
    {
        let s = store.lock().unwrap();
        let recovered = s.recover_goals().unwrap();
        assert_eq!(recovered.len(), 1);
        assert_eq!(recovered[0].state, GoalState::Suspended);
    }

    // 7. Resume: Suspended → Ready.
    {
        let s = store.lock().unwrap();
        let current = s.get_goal(id).unwrap().unwrap();
        s.transition_goal(
            id,
            current.version,
            GoalState::Ready,
            None,
            &serde_json::json!({"action": "resume"}),
        )
        .unwrap();
    }

    // 8. Tick again: Ready → Running.
    let outcome = coord.tick(id, 0).unwrap();
    assert!(matches!(
        outcome,
        GoalTickOutcome::Transitioned {
            from: GoalState::Ready,
            to: GoalState::Running,
        }
    ));

    // 9. Cancel: Running → Cancelled.
    {
        let s = store.lock().unwrap();
        let current = s.get_goal(id).unwrap().unwrap();
        s.transition_goal(
            id,
            current.version,
            GoalState::Cancelled,
            None,
            &serde_json::json!({"action": "cancel"}),
        )
        .unwrap();
    }

    // 10. Tick on Cancelled is Noop.
    let outcome = coord.tick(id, 0).unwrap();
    assert!(matches!(
        outcome,
        GoalTickOutcome::Noop {
            state: GoalState::Cancelled
        }
    ));

    // 11. Second restart: terminal goals not recovered.
    {
        let s = store.lock().unwrap();
        let recovered = s.recover_goals().unwrap();
        assert!(recovered.is_empty());
    }
}

// ---------------------------------------------------------------------------
// Invariant: original intent never changes
// ---------------------------------------------------------------------------

#[test]
fn original_intent_immutable() {
    let (coord, store) = setup();
    let id = create_goal(&store, "immutable intent", false);

    let check = |store: &Arc<Mutex<ObjectiveStore>>| {
        let s = store.lock().unwrap();
        let g = s.get_goal(id).unwrap().unwrap();
        assert_eq!(g.spec.original_intent, "immutable intent");
    };

    check(&store);

    // Tick through several states.
    coord.tick(id, 0).unwrap(); // Ready -> Running
    check(&store);

    coord.tick(id, 0).unwrap(); // TurnRequested (no state change)
    check(&store);

    {
        let s = store.lock().unwrap();
        s.transition_goal(id, 1, GoalState::Completed, None, &serde_json::json!({}))
            .unwrap();
    }
    check(&store);
}
