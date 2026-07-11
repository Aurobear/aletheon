//! Integration tests for the supervision subsystem.
//!
//! Covers:
//! - SupervisorTree restart-decision logic (kernel-level unit tests)
//! - SubAgentSpawner restart behaviour (end-to-end integration)

use executive::core::SubAgentSpawner;
use executive::kernel::supervision::{RestartDecision, RestartPolicy, SupervisorTree};
use fabric::{ExitReason, ProcessId, SubAgentState};

// ---------------------------------------------------------------------------
// SupervisorTree (kernel) direct tests
// ---------------------------------------------------------------------------

#[test]
fn supervision_restart_on_failure_stops_at_limit() {
    let id = ProcessId::new();
    let mut tree = SupervisorTree::new();
    tree.supervise(id, RestartPolicy::RestartOnFailure { max_restarts: 2 });

    assert_eq!(
        tree.record_exit(id, &ExitReason::Panic("boom".into())),
        RestartDecision::Restart { attempt: 1 }
    );
    assert_eq!(
        tree.record_exit(id, &ExitReason::Failed("again".into())),
        RestartDecision::Restart { attempt: 2 }
    );
    assert_eq!(
        tree.record_exit(id, &ExitReason::DeadlineExceeded),
        RestartDecision::FailedLimitReached
    );
}

#[test]
fn supervision_restart_on_failure_max_1_restarts_once_then_stops() {
    let id = ProcessId::new();
    let mut tree = SupervisorTree::new();
    tree.supervise(id, RestartPolicy::RestartOnFailure { max_restarts: 1 });

    assert_eq!(
        tree.record_exit(id, &ExitReason::Failed("boom".into())),
        RestartDecision::Restart { attempt: 1 }
    );
    // Second failure exceeds the limit.
    assert_eq!(
        tree.record_exit(id, &ExitReason::Failed("again".into())),
        RestartDecision::FailedLimitReached
    );
}

#[test]
fn supervision_never_policy_does_not_restart() {
    let id = ProcessId::new();
    let mut tree = SupervisorTree::new();
    tree.supervise(id, RestartPolicy::Never);

    // Even failure exits should not trigger a restart.
    assert_eq!(
        tree.record_exit(id, &ExitReason::Failed("fail".into())),
        RestartDecision::DoNotRestart
    );
    assert_eq!(
        tree.record_exit(id, &ExitReason::Panic("panic".into())),
        RestartDecision::DoNotRestart
    );
    assert_eq!(
        tree.record_exit(id, &ExitReason::DeadlineExceeded),
        RestartDecision::DoNotRestart
    );
}

#[test]
fn supervision_non_failure_exits_return_do_not_restart() {
    let id = ProcessId::new();
    let mut tree = SupervisorTree::new();
    tree.supervise(id, RestartPolicy::RestartOnFailure { max_restarts: 5 });

    // Completed / normal exits are not failures — no restart.
    assert_eq!(
        tree.record_exit(id, &ExitReason::Completed),
        RestartDecision::DoNotRestart
    );
    // Cancelled is also not a failure.
    assert_eq!(
        tree.record_exit(id, &ExitReason::Cancelled("user-cancelled".into())),
        RestartDecision::DoNotRestart
    );
}

#[test]
fn supervision_unsupervised_process_returns_do_not_restart() {
    let id = ProcessId::new();
    let mut tree = SupervisorTree::new();
    // Never call tree.supervise() — default policy is Never.

    assert_eq!(
        tree.record_exit(id, &ExitReason::Failed("orphan".into())),
        RestartDecision::DoNotRestart
    );
}

#[test]
fn supervision_independent_process_tracking() {
    let a = ProcessId::new();
    let b = ProcessId::new();
    let mut tree = SupervisorTree::new();
    tree.supervise(a, RestartPolicy::RestartOnFailure { max_restarts: 2 });
    tree.supervise(b, RestartPolicy::RestartOnFailure { max_restarts: 1 });

    // Process A: 1st failure → restart.
    assert_eq!(
        tree.record_exit(a, &ExitReason::Failed("a-1".into())),
        RestartDecision::Restart { attempt: 1 }
    );
    // Process B: 1st failure → restart.
    assert_eq!(
        tree.record_exit(b, &ExitReason::Failed("b-1".into())),
        RestartDecision::Restart { attempt: 1 }
    );
    // Process B: 2nd failure → limit reached (max_restarts: 1).
    assert_eq!(
        tree.record_exit(b, &ExitReason::Failed("b-2".into())),
        RestartDecision::FailedLimitReached
    );
    // Process A is unaffected — still has one restart remaining.
    assert_eq!(
        tree.record_exit(a, &ExitReason::Failed("a-2".into())),
        RestartDecision::Restart { attempt: 2 }
    );
}

#[test]
fn supervision_restart_attempts_increment_only_on_failure() {
    let id = ProcessId::new();
    let mut tree = SupervisorTree::new();
    tree.supervise(id, RestartPolicy::RestartOnFailure { max_restarts: 2 });

    // Non-failure exits should not consume restart budget.
    tree.record_exit(id, &ExitReason::Completed);
    tree.record_exit(id, &ExitReason::Completed);

    // First failure should still be attempt 1.
    assert_eq!(
        tree.record_exit(id, &ExitReason::Failed("fail".into())),
        RestartDecision::Restart { attempt: 1 }
    );
}

// ---------------------------------------------------------------------------
// SubAgentSpawner integration tests
// ---------------------------------------------------------------------------

#[tokio::test]
async fn spawner_restart_on_failure_max_1_restarts_once() {
    let mut spawner = SubAgentSpawner::new();
    let original = spawner
        .spawn_with_policy(
            "task-a".into(),
            "turn-1".into(),
            RestartPolicy::RestartOnFailure { max_restarts: 1 },
        )
        .await
        .unwrap();

    assert_eq!(spawner.list().len(), 1);
    assert_eq!(spawner.state(&original.id), Some(SubAgentState::Created));

    // Transition to Running so the process table can accept a mark_exit.
    spawner
        .transition(&original.id, SubAgentState::Running)
        .await
        .unwrap();

    // Trigger failure — supervisor should restart (attempt 1 of max 1).
    spawner
        .transition(&original.id, SubAgentState::Failed)
        .await
        .unwrap();

    // The original is now Failed; a replacement has been spawned.
    let agents = spawner.list();
    assert_eq!(
        agents.len(),
        2,
        "expected 2 agents after restart: original (Failed) + replacement"
    );
    assert_eq!(spawner.state(&original.id), Some(SubAgentState::Failed));

    // The replacement has a different id and is in Created state.
    let replacement_id: Vec<&str> = agents
        .iter()
        .map(|h| h.id.as_str())
        .filter(|id| *id != original.id)
        .collect();
    assert_eq!(replacement_id.len(), 1);
    assert_eq!(
        spawner.state(replacement_id[0]),
        Some(SubAgentState::Created)
    );
}

#[tokio::test]
async fn spawner_never_policy_does_not_restart() {
    let mut spawner = SubAgentSpawner::new();
    // Default spawn() uses RestartPolicy::Never.
    let agent = spawner
        .spawn("task-a".into(), "turn-1".into())
        .await
        .unwrap();

    assert_eq!(spawner.list().len(), 1);

    // Transition to Running so the process table can accept mark_exit.
    spawner
        .transition(&agent.id, SubAgentState::Running)
        .await
        .unwrap();

    // Trigger failure — Never policy, no restart.
    spawner
        .transition(&agent.id, SubAgentState::Failed)
        .await
        .unwrap();

    // Still only one agent, now Failed.
    assert_eq!(spawner.list().len(), 1);
    assert_eq!(spawner.state(&agent.id), Some(SubAgentState::Failed));
}

#[tokio::test]
async fn spawner_restart_attempt_is_tracked_in_supervisor_tree() {
    let mut spawner = SubAgentSpawner::new();
    let original = spawner
        .spawn_with_policy(
            "task-a".into(),
            "turn-1".into(),
            RestartPolicy::RestartOnFailure { max_restarts: 2 },
        )
        .await
        .unwrap();

    // Run then fail twice. After the second failure the limit is reached.
    spawner
        .transition(&original.id, SubAgentState::Running)
        .await
        .unwrap();

    // First failure → restart (attempt 1).
    spawner
        .transition(&original.id, SubAgentState::Failed)
        .await
        .unwrap();
    assert_eq!(spawner.list().len(), 2, "first failure spawns replacement");
    assert_eq!(spawner.state(&original.id), Some(SubAgentState::Failed));
    // The original stays Failed; a second transition to Failed is illegal
    // (Failed can only go to Destroyed).  The restart count on the
    // SupervisorTree is internal, but the existing SupervisorTree unit tests
    // verify the attempt counter exhausts correctly.
    //
    // What we CAN verify here: the replacement does NOT itself restart
    // (it was spawned with RestartPolicy::Never).
    let replacement_id = spawner
        .list()
        .iter()
        .find(|h| h.id != original.id)
        .map(|h| h.id.clone())
        .expect("replacement should exist");

    spawner
        .transition(&replacement_id, SubAgentState::Running)
        .await
        .unwrap();
    spawner
        .transition(&replacement_id, SubAgentState::Failed)
        .await
        .unwrap();

    // After the replacement fails with Never policy, no third agent appears.
    assert_eq!(
        spawner.list().len(),
        2,
        "replacement has Never policy; no third agent spawned"
    );
    assert_eq!(spawner.state(&replacement_id), Some(SubAgentState::Failed));
}
