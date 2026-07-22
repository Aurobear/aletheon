//! Agent integration baseline — regression tests that must pass before
//! any channel-based integration replaces existing runtime behavior.
//!
//! These tests lock the invariant contracts that the channel integration
//! must preserve: (1) ProcessState transition rules, and (2) ObjectiveStore
//! persistence across re-opens.

use executive::goal::ObjectiveStore;
use fabric::ProcessState;

// ── Step 1.1: ProcessState transition contract ────────────────────────────

#[test]
fn generic_process_state_contract_is_unchanged() {
    assert!(ProcessState::Created.can_transition_to(ProcessState::Ready));
    assert!(ProcessState::Ready.can_transition_to(ProcessState::Running));
    assert!(ProcessState::Running.can_transition_to(ProcessState::Waiting));
    assert!(ProcessState::Waiting.can_transition_to(ProcessState::Running));
    assert!(ProcessState::Running.can_transition_to(ProcessState::Stopping));
    assert!(ProcessState::Stopping.can_transition_to(ProcessState::Exited));
    assert!(!ProcessState::Created.can_transition_to(ProcessState::Running));
}

// ── Step 1.2: ObjectiveStore restart regression ──────────────────────────

#[test]
fn objective_store_reopens_and_recovers_active_objective() {
    // Use a temp dir so that the WAL and index files live alongside the DB file
    // without colliding between concurrent test runs.
    let tmp_dir = tempfile::tempdir().expect("tempdir for objective store");
    let db_path = tmp_dir.path().join("objectives.db");

    // Open, create an objective, then drop the store.
    let description = "rebuild the hyperdrive after hyperspace incident";
    {
        let store = ObjectiveStore::open(&db_path).expect("open objective store for first write");
        let id = store
            .create(description, None, "session-reopen", "project")
            .expect("create objective");
        assert!(id > 0, "new objective should have a positive id");
    } // store dropped, connection closed

    // Re-open from the same file — must recover the objective with the same
    // description.
    {
        let store = ObjectiveStore::open(&db_path).expect("re-open objective store for recovery");
        let active = store
            .active()
            .expect("active query should succeed")
            .expect("active objective should exist after reopen");
        assert_eq!(
            active.description, description,
            "recovered objective should have the same description"
        );
        assert_eq!(active.session_id, "session-reopen");
        assert_eq!(active.scope, "project");
    }
}
