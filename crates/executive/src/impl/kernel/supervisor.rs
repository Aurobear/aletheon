//! AgentSupervisor — lifecycle management with exponential backoff and fast-fail detection.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use fabric::agent::Pid;
use fabric::Clock;
use fabric::MonoTime;
use tokio::sync::RwLock;

use crate::r#impl::agent::process::AgentProcessConfig;

// ---------------------------------------------------------------------------
// RestartPolicy
// ---------------------------------------------------------------------------

/// Configuration for restart behavior including exponential backoff and fast-fail detection.
#[derive(Debug, Clone)]
pub struct RestartPolicy {
    /// Initial delay before the first restart attempt.
    pub initial_delay: Duration,
    /// Maximum delay between restart attempts.
    pub max_delay: Duration,
    /// Multiplier applied to the delay on each successive restart.
    pub backoff_multiplier: f64,
    /// Time window in which repeated crashes trigger fast-fail parking.
    pub fast_fail_window: Duration,
    /// Number of crashes within the fast-fail window that triggers parking.
    pub fast_fail_threshold: u32,
    /// Exit codes that indicate a permanent (non-retryable) failure.
    pub permanent_exit_codes: Vec<i32>,
}

impl Default for RestartPolicy {
    fn default() -> Self {
        Self {
            initial_delay: Duration::from_secs(2),
            max_delay: Duration::from_secs(120),
            backoff_multiplier: 2.0,
            fast_fail_window: Duration::from_secs(10),
            fast_fail_threshold: 5,
            permanent_exit_codes: vec![78],
        }
    }
}

// ---------------------------------------------------------------------------
// SupervisedState / RestartDecision
// ---------------------------------------------------------------------------

/// State of a supervised process.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SupervisedState {
    Running,
    Suspended,
    Restarting,
    Parked,
    Terminated,
}

/// Decision made by the supervisor after a crash.
#[derive(Debug, Clone)]
pub enum RestartDecision {
    /// Restart the process after a delay.
    Restart {
        delay: Duration,
        task: String,
        config: AgentProcessConfig,
        parent: Option<Pid>,
    },
    /// Park the process — no further restarts.
    Park,
    /// Ignore the crash (process is not supervised).
    Ignore,
}

// ---------------------------------------------------------------------------
// SupervisedProcess (private)
// ---------------------------------------------------------------------------

struct SupervisedProcess {
    config: AgentProcessConfig,
    task: String,
    restart_count: u32,
    last_heartbeat: MonoTime,
    crash_times: Vec<MonoTime>,
    state: SupervisedState,
    parent: Option<Pid>,
}

// ---------------------------------------------------------------------------
// AgentSupervisor
// ---------------------------------------------------------------------------

/// Manages supervised agent processes with automatic restart, exponential
/// backoff, and fast-fail detection.
pub struct AgentSupervisor {
    supervised: RwLock<HashMap<Pid, SupervisedProcess>>,
    policy: RestartPolicy,
    clock: Arc<dyn Clock>,
}

impl AgentSupervisor {
    /// Create a new supervisor with the given restart policy.
    pub fn new(policy: RestartPolicy, clock: Arc<dyn Clock>) -> Self {
        Self {
            supervised: RwLock::new(HashMap::new()),
            policy,
            clock,
        }
    }

    /// Register a process for supervision.
    pub async fn supervise(
        &self,
        pid: Pid,
        task: String,
        config: AgentProcessConfig,
        parent: Option<Pid>,
    ) {
        let entry = SupervisedProcess {
            config,
            task,
            restart_count: 0,
            last_heartbeat: self.clock.mono_now(),
            crash_times: Vec::new(),
            state: SupervisedState::Running,
            parent,
        };
        self.supervised.write().await.insert(pid, entry);
    }

    /// Record a heartbeat for the given process.
    pub async fn heartbeat(&self, pid: Pid) {
        if let Some(proc) = self.supervised.write().await.get_mut(&pid) {
            proc.last_heartbeat = self.clock.mono_now();
        }
    }

    /// Handle a crash for the given process. Returns the restart decision.
    pub async fn on_crash(&self, pid: Pid, exit_code: Option<i32>) -> RestartDecision {
        let mut supervised = self.supervised.write().await;
        let proc = match supervised.get_mut(&pid) {
            Some(p) => p,
            None => return RestartDecision::Ignore,
        };

        // 1. Check permanent exit code.
        if let Some(code) = exit_code {
            if self.policy.permanent_exit_codes.contains(&code) {
                proc.state = SupervisedState::Parked;
                return RestartDecision::Park;
            }
        }

        // 2. Record crash time and prune old ones outside the window.
        let now = self.clock.mono_now();
        proc.crash_times.push(now);
        let window = self.policy.fast_fail_window;
        proc.crash_times
            .retain(|t| Duration::from_millis(now.0.saturating_sub(t.0)) <= window);

        // 3. Check fast-fail threshold.
        if proc.crash_times.len() as u32 >= self.policy.fast_fail_threshold {
            proc.state = SupervisedState::Parked;
            return RestartDecision::Park;
        }

        // 4. Increment restart count and calculate delay.
        proc.restart_count += 1;
        let delay = self.calculate_delay(proc.restart_count);
        proc.state = SupervisedState::Restarting;

        RestartDecision::Restart {
            delay,
            task: proc.task.clone(),
            config: proc.config.clone(),
            parent: proc.parent,
        }
    }

    /// Mark a supervised process as stable (resets restart counter and crash history).
    pub async fn mark_stable(&self, pid: Pid) {
        if let Some(proc) = self.supervised.write().await.get_mut(&pid) {
            proc.restart_count = 0;
            proc.crash_times.clear();
            proc.state = SupervisedState::Running;
        }
    }

    /// Number of currently supervised processes.
    pub async fn supervised_count(&self) -> usize {
        self.supervised.read().await.len()
    }

    /// Get the current state of a supervised process.
    pub async fn state_of(&self, pid: Pid) -> Option<SupervisedState> {
        self.supervised
            .read()
            .await
            .get(&pid)
            .map(|p| p.state.clone())
    }

    /// Calculate the exponential backoff delay for a given restart count.
    fn calculate_delay(&self, restart_count: u32) -> Duration {
        let delay_secs = self.policy.initial_delay.as_secs_f64()
            * self
                .policy
                .backoff_multiplier
                .powi(restart_count as i32 - 1);
        let capped = delay_secs.min(self.policy.max_delay.as_secs_f64());
        Duration::from_secs_f64(capped)
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn Clock> {
        Arc::new(TestClock::default())
    }

    fn make_config(id: &str) -> AgentProcessConfig {
        AgentProcessConfig {
            id: id.to_string(),
            max_tokens_per_pulse: 1000,
            ..Default::default()
        }
    }

    #[tokio::test]
    async fn test_supervise_and_heartbeat() {
        let supervisor = AgentSupervisor::new(RestartPolicy::default(), test_clock());
        let pid = Pid::new();
        supervisor
            .supervise(pid, "task-1".into(), make_config("a1"), None)
            .await;
        assert_eq!(supervisor.supervised_count().await, 1);
        assert_eq!(
            supervisor.state_of(pid).await,
            Some(SupervisedState::Running)
        );

        supervisor.heartbeat(pid).await;
        // State should still be Running after heartbeat.
        assert_eq!(
            supervisor.state_of(pid).await,
            Some(SupervisedState::Running)
        );
    }

    #[tokio::test]
    async fn test_crash_restart_decision() {
        let supervisor = AgentSupervisor::new(RestartPolicy::default(), test_clock());
        let pid = Pid::new();
        supervisor
            .supervise(pid, "task-1".into(), make_config("a1"), None)
            .await;

        let decision = supervisor.on_crash(pid, Some(1)).await;
        match decision {
            RestartDecision::Restart { delay, .. } => {
                assert_eq!(delay, Duration::from_secs(2));
            }
            other => panic!("expected Restart, got {:?}", other),
        }
    }

    #[tokio::test]
    async fn test_permanent_exit_code_parks() {
        let supervisor = AgentSupervisor::new(RestartPolicy::default(), test_clock());
        let pid = Pid::new();
        supervisor
            .supervise(pid, "task-1".into(), make_config("a1"), None)
            .await;

        let decision = supervisor.on_crash(pid, Some(78)).await;
        assert!(matches!(decision, RestartDecision::Park));
        assert_eq!(
            supervisor.state_of(pid).await,
            Some(SupervisedState::Parked)
        );
    }

    #[tokio::test]
    async fn test_fast_fail_parks() {
        let policy = RestartPolicy {
            fast_fail_window: Duration::from_secs(60),
            fast_fail_threshold: 3,
            ..Default::default()
        };
        let supervisor = AgentSupervisor::new(policy, test_clock());
        let pid = Pid::new();
        supervisor
            .supervise(pid, "task-1".into(), make_config("a1"), None)
            .await;

        // Crash 1 → Restart
        let d1 = supervisor.on_crash(pid, Some(1)).await;
        assert!(matches!(d1, RestartDecision::Restart { .. }));

        // Crash 2 → Restart
        let d2 = supervisor.on_crash(pid, Some(1)).await;
        assert!(matches!(d2, RestartDecision::Restart { .. }));

        // Crash 3 → Park (threshold reached)
        let d3 = supervisor.on_crash(pid, Some(1)).await;
        assert!(matches!(d3, RestartDecision::Park));
        assert_eq!(
            supervisor.state_of(pid).await,
            Some(SupervisedState::Parked)
        );
    }

    #[tokio::test]
    async fn test_exponential_backoff() {
        let supervisor = AgentSupervisor::new(RestartPolicy::default(), test_clock());
        let pid = Pid::new();
        supervisor
            .supervise(pid, "task-1".into(), make_config("a1"), None)
            .await;

        // Crash 1 → 2s
        let d1 = supervisor.on_crash(pid, Some(1)).await;
        if let RestartDecision::Restart { delay, .. } = d1 {
            assert_eq!(delay, Duration::from_secs(2));
        } else {
            panic!("expected Restart");
        }

        // Crash 2 → 4s
        let d2 = supervisor.on_crash(pid, Some(1)).await;
        if let RestartDecision::Restart { delay, .. } = d2 {
            assert_eq!(delay, Duration::from_secs(4));
        } else {
            panic!("expected Restart");
        }

        // Crash 3 → 8s
        let d3 = supervisor.on_crash(pid, Some(1)).await;
        if let RestartDecision::Restart { delay, .. } = d3 {
            assert_eq!(delay, Duration::from_secs(8));
        } else {
            panic!("expected Restart");
        }
    }

    #[tokio::test]
    async fn test_mark_stable_resets_count() {
        let supervisor = AgentSupervisor::new(RestartPolicy::default(), test_clock());
        let pid = Pid::new();
        supervisor
            .supervise(pid, "task-1".into(), make_config("a1"), None)
            .await;

        // Crash twice to bump restart_count.
        supervisor.on_crash(pid, Some(1)).await;
        supervisor.on_crash(pid, Some(1)).await;

        // Mark stable resets the counter.
        supervisor.mark_stable(pid).await;

        // Next crash should use initial_delay (2s) again.
        let decision = supervisor.on_crash(pid, Some(1)).await;
        if let RestartDecision::Restart { delay, .. } = decision {
            assert_eq!(delay, Duration::from_secs(2));
        } else {
            panic!("expected Restart with initial_delay after mark_stable");
        }
    }
}
