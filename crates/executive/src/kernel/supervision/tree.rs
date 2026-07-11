use fabric::{ExitReason, ProcessId};
use std::collections::HashMap;

// ---------------------------------------------------------------------------
// Individual restart policy
// ---------------------------------------------------------------------------

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RestartPolicy {
    Never,
    RestartOnFailure { max_restarts: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RestartDecision {
    DoNotRestart,
    Restart {
        attempt: usize,
    },
    FailedLimitReached,
    /// Group-strategy restarts: the failed process AND additional siblings
    /// must restart according to the group's strategy.
    RestartGroup {
        attempt: usize,
        /// Additional sibling processes to restart alongside the failed one.
        /// The caller should restart the `failed_id` plus each of these.
        siblings: Vec<ProcessId>,
    },
}

// ---------------------------------------------------------------------------
// Group strategy
// ---------------------------------------------------------------------------

/// How the supervisor handles a member failure within a group.
///
/// These are the standard OTP-style restart strategies:
///
/// - [`OneForOne`]: Only the failed process is restarted (default).
/// - [`OneForAll`]: All members of the group are restarted.
/// - [`RestForOne`]: The failed process and all members started *after* it
///   are restarted (in start order).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GroupStrategy {
    /// Restart only the failed child (default).
    OneForOne,
    /// Restart all children in the group.
    OneForAll,
    /// Restart the failed child and all children started after it.
    RestForOne,
}

#[derive(Debug, Clone)]
struct SupervisorGroup {
    strategy: GroupStrategy,
    /// Members in start order (oldest first).
    members: Vec<ProcessId>,
}

// ---------------------------------------------------------------------------
// SupervisorTree
// ---------------------------------------------------------------------------

#[derive(Debug, Default)]
pub struct SupervisorTree {
    policies: HashMap<ProcessId, RestartPolicy>,
    restarts: HashMap<ProcessId, usize>,
    /// Process → group name.
    process_groups: HashMap<ProcessId, String>,
    /// Group name → group.
    groups: HashMap<String, SupervisorGroup>,
}

impl SupervisorTree {
    pub fn new() -> Self {
        Self::default()
    }

    /// Register a process under an individual restart policy.
    pub fn supervise(&mut self, id: ProcessId, policy: RestartPolicy) {
        self.policies.insert(id, policy);
    }

    /// Create a supervised group.
    ///
    /// Group strategies control how failures propagate across sibling
    /// processes. See [`GroupStrategy`].
    pub fn create_group(&mut self, name: &str, strategy: GroupStrategy) {
        self.groups.insert(
            name.to_string(),
            SupervisorGroup {
                strategy,
                members: Vec::new(),
            },
        );
    }

    /// Add a process to a supervised group.
    ///
    /// Processes within a group are tracked in start order for
    /// [`GroupStrategy::RestForOne`].
    pub fn add_to_group(&mut self, name: &str, id: ProcessId) {
        self.process_groups.insert(id, name.to_string());
        if let Some(group) = self.groups.get_mut(name) {
            group.members.push(id);
        }
    }

    /// Return which group (if any) a process belongs to.
    pub fn group_of(&self, id: ProcessId) -> Option<&str> {
        self.process_groups.get(&id).map(|s| s.as_str())
    }

    /// Record a process exit and decide whether/how to restart.
    ///
    /// If the process belongs to a supervised group with a strategy other
    /// than [`OneForOne`], the decision includes sibling restarts.
    pub fn record_exit(&mut self, id: ProcessId, reason: &ExitReason) -> RestartDecision {
        let is_failure = matches!(
            reason,
            ExitReason::Failed(_) | ExitReason::Panic(_) | ExitReason::DeadlineExceeded
        );
        if !is_failure {
            return RestartDecision::DoNotRestart;
        }

        let policy = self
            .policies
            .get(&id)
            .copied()
            .unwrap_or(RestartPolicy::Never);

        let (attempt, can_restart) = match policy {
            RestartPolicy::Never => return RestartDecision::DoNotRestart,
            RestartPolicy::RestartOnFailure { max_restarts } => {
                let next = self.restarts.get(&id).copied().unwrap_or(0) + 1;
                if next > max_restarts {
                    (next, false)
                } else {
                    self.restarts.insert(id, next);
                    (next, true)
                }
            }
        };

        if !can_restart {
            return RestartDecision::FailedLimitReached;
        }

        // Check group strategy for sibling restarts.
        let group_name = self.process_groups.get(&id).cloned();
        if let Some(name) = group_name {
            if let Some(group) = self.groups.get(&name) {
                let siblings = match group.strategy {
                    GroupStrategy::OneForOne => Vec::new(),
                    GroupStrategy::OneForAll => group
                        .members
                        .iter()
                        .copied()
                        .filter(|&pid| pid != id)
                        .collect(),
                    GroupStrategy::RestForOne => {
                        // Find the failed process's position and take
                        // everything after it.
                        let pos = group.members.iter().position(|&pid| pid == id);
                        match pos {
                            Some(idx) => group.members[idx + 1..].to_vec(),
                            None => Vec::new(),
                        }
                    }
                };

                if !siblings.is_empty() {
                    // Reset restart counters for sibling restarts.
                    for &sibling in &siblings {
                        self.restarts.remove(&sibling);
                    }
                    return RestartDecision::RestartGroup { attempt, siblings };
                }
            }
        }

        RestartDecision::Restart { attempt }
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    fn pid(id: u64) -> ProcessId {
        ProcessId(uuid::Uuid::from_u128(id as u128))
    }

    fn failed() -> ExitReason {
        ExitReason::Failed("test failure".into())
    }

    fn success() -> ExitReason {
        ExitReason::Completed
    }

    // -- Individual restart tests (existing behaviour) ----------------------

    #[test]
    fn success_exit_is_not_restarted() {
        let mut tree = SupervisorTree::new();
        tree.supervise(pid(1), RestartPolicy::RestartOnFailure { max_restarts: 3 });
        let d = tree.record_exit(pid(1), &success());
        assert_eq!(d, RestartDecision::DoNotRestart);
    }

    #[test]
    fn failure_with_never_policy_does_not_restart() {
        let mut tree = SupervisorTree::new();
        tree.supervise(pid(1), RestartPolicy::Never);
        let d = tree.record_exit(pid(1), &failed());
        assert_eq!(d, RestartDecision::DoNotRestart);
    }

    #[test]
    fn failure_with_restart_policy_restarts() {
        let mut tree = SupervisorTree::new();
        tree.supervise(pid(1), RestartPolicy::RestartOnFailure { max_restarts: 3 });
        let d = tree.record_exit(pid(1), &failed());
        assert_eq!(d, RestartDecision::Restart { attempt: 1 });
    }

    #[test]
    fn max_restarts_exceeded_returns_failed_limit() {
        let mut tree = SupervisorTree::new();
        tree.supervise(pid(1), RestartPolicy::RestartOnFailure { max_restarts: 1 });
        let d1 = tree.record_exit(pid(1), &failed());
        assert_eq!(d1, RestartDecision::Restart { attempt: 1 });

        let d2 = tree.record_exit(pid(1), &failed());
        assert_eq!(d2, RestartDecision::FailedLimitReached);
    }

    // -- Group strategy tests ------------------------------------------------

    #[test]
    fn one_for_one_restarts_only_failed() {
        let mut tree = SupervisorTree::new();
        tree.create_group("workers", GroupStrategy::OneForOne);
        tree.add_to_group("workers", pid(1));
        tree.add_to_group("workers", pid(2));
        tree.supervise(pid(1), RestartPolicy::RestartOnFailure { max_restarts: 3 });
        tree.supervise(pid(2), RestartPolicy::RestartOnFailure { max_restarts: 3 });

        let d = tree.record_exit(pid(1), &failed());
        assert_eq!(d, RestartDecision::Restart { attempt: 1 });
    }

    #[test]
    fn one_for_all_restarts_all_siblings() {
        let mut tree = SupervisorTree::new();
        tree.create_group("workers", GroupStrategy::OneForAll);
        tree.add_to_group("workers", pid(1));
        tree.add_to_group("workers", pid(2));
        tree.add_to_group("workers", pid(3));
        tree.supervise(pid(1), RestartPolicy::RestartOnFailure { max_restarts: 3 });
        tree.supervise(pid(2), RestartPolicy::RestartOnFailure { max_restarts: 3 });
        tree.supervise(pid(3), RestartPolicy::RestartOnFailure { max_restarts: 3 });

        let d = tree.record_exit(pid(1), &failed());
        match d {
            RestartDecision::RestartGroup { attempt, siblings } => {
                assert_eq!(attempt, 1);
                // pid(2) and pid(3) should restart alongside pid(1).
                assert_eq!(siblings.len(), 2);
                assert!(siblings.contains(&pid(2)));
                assert!(siblings.contains(&pid(3)));
            }
            other => panic!("expected RestartGroup, got {other:?}"),
        }
    }

    #[test]
    fn rest_for_one_restarts_failed_and_later_siblings() {
        let mut tree = SupervisorTree::new();
        tree.create_group("pipeline", GroupStrategy::RestForOne);
        // Start order: A → B → C → D
        tree.add_to_group("pipeline", pid(10)); // A (first)
        tree.add_to_group("pipeline", pid(20)); // B
        tree.add_to_group("pipeline", pid(30)); // C
        tree.add_to_group("pipeline", pid(40)); // D (last)
        for i in [10, 20, 30, 40] {
            tree.supervise(pid(i), RestartPolicy::RestartOnFailure { max_restarts: 3 });
        }

        // B fails → B, C, D restart. A (before B) is NOT restarted.
        let d = tree.record_exit(pid(20), &failed());
        match d {
            RestartDecision::RestartGroup { attempt, siblings } => {
                assert_eq!(attempt, 1);
                // pid(30) and pid(40) are after pid(20).
                assert_eq!(siblings.len(), 2);
                assert!(siblings.contains(&pid(30)));
                assert!(siblings.contains(&pid(40)));
                assert!(!siblings.contains(&pid(10))); // A is before B
            }
            other => panic!("expected RestartGroup, got {other:?}"),
        }
    }

    #[test]
    fn rest_for_one_last_member_restarts_only_self() {
        let mut tree = SupervisorTree::new();
        tree.create_group("pipeline", GroupStrategy::RestForOne);
        tree.add_to_group("pipeline", pid(10));
        tree.add_to_group("pipeline", pid(20));
        tree.supervise(pid(10), RestartPolicy::RestartOnFailure { max_restarts: 3 });
        tree.supervise(pid(20), RestartPolicy::RestartOnFailure { max_restarts: 3 });

        // Last member fails → no siblings to restart.
        let d = tree.record_exit(pid(20), &failed());
        assert_eq!(d, RestartDecision::Restart { attempt: 1 });
    }

    #[test]
    fn group_restart_resets_sibling_counters() {
        let mut tree = SupervisorTree::new();
        tree.create_group("workers", GroupStrategy::OneForAll);
        tree.add_to_group("workers", pid(1));
        tree.add_to_group("workers", pid(2));
        tree.supervise(pid(1), RestartPolicy::RestartOnFailure { max_restarts: 3 });
        tree.supervise(pid(2), RestartPolicy::RestartOnFailure { max_restarts: 3 });

        // pid(2) had a previous restart attempt.
        assert_eq!(
            tree.record_exit(pid(2), &failed()),
            RestartDecision::RestartGroup {
                attempt: 1,
                siblings: vec![pid(1)]
            }
        );

        // pid(1) fails → OneForAll triggers. pid(2)'s counter should reset.
        let d = tree.record_exit(pid(1), &failed());
        assert!(matches!(d, RestartDecision::RestartGroup { .. }));

        // pid(2) fails again → counter was reset, so attempt starts at 1
        // (still RestartGroup because pid(2) is in an OneForAll group).
        let d2 = tree.record_exit(pid(2), &failed());
        match d2 {
            RestartDecision::RestartGroup { attempt, .. } => {
                assert_eq!(attempt, 1, "counter should have been reset to 1");
            }
            other => panic!("expected RestartGroup, got {other:?}"),
        }
    }

    #[test]
    fn non_failure_does_not_trigger_group_restart() {
        let mut tree = SupervisorTree::new();
        tree.create_group("workers", GroupStrategy::OneForAll);
        tree.add_to_group("workers", pid(1));
        tree.add_to_group("workers", pid(2));
        tree.supervise(pid(1), RestartPolicy::RestartOnFailure { max_restarts: 3 });
        tree.supervise(pid(2), RestartPolicy::RestartOnFailure { max_restarts: 3 });

        // Successful exit does NOT trigger group restart.
        let d = tree.record_exit(pid(1), &success());
        assert_eq!(d, RestartDecision::DoNotRestart);
    }

    #[test]
    fn group_of_returns_correct_group() {
        let mut tree = SupervisorTree::new();
        tree.create_group("a", GroupStrategy::OneForOne);
        tree.add_to_group("a", pid(1));

        assert_eq!(tree.group_of(pid(1)), Some("a"));
        assert_eq!(tree.group_of(pid(99)), None);
    }
}
