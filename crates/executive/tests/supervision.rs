//! Integration tests for the supervision subsystem.
//!
//! Covers:
//! - SupervisorTree restart-decision logic (kernel-level unit tests)

use fabric::{ExitReason, ProcessId};
use kernel::supervision::{RestartDecision, RestartPolicy, SupervisorTree};

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
