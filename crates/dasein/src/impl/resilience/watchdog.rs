use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use aletheon_kernel::chronos::Timer;
use fabric::{Clock, MonoTime};
use tokio::sync::watch;
use tracing::{info, warn};

/// A single heartbeat layer with a sender (task side) and a tracked
/// last-beat timestamp (checker side).
pub struct HeartbeatLayer {
    name: &'static str,
    timeout: Duration,
    tx: watch::Sender<()>,
    #[allow(dead_code)]
    rx: watch::Receiver<()>,
    last_beat: Arc<std::sync::Mutex<MonoTime>>,
    alive: Arc<AtomicBool>,
    clock: Arc<dyn Clock>,
}

impl HeartbeatLayer {
    pub fn new(name: &'static str, timeout: Duration, clock: Arc<dyn Clock>) -> Self {
        let (tx, rx) = watch::channel(());
        let now = clock.mono_now();
        Self {
            name,
            timeout,
            tx,
            rx,
            last_beat: Arc::new(std::sync::Mutex::new(now)),
            alive: Arc::new(AtomicBool::new(true)),
            clock,
        }
    }

    /// Send a heartbeat signal from the monitored task.
    pub fn beat(&self) {
        if let Ok(mut t) = self.last_beat.lock() {
            *t = self.clock.mono_now();
        }
        let _ = self.tx.send(());
    }

    /// Returns `true` if the last heartbeat was within the timeout window.
    pub fn is_alive(&self) -> bool {
        let elapsed_ms = self
            .last_beat
            .lock()
            .map(|t| self.clock.mono_now().0.saturating_sub(t.0))
            .unwrap_or(u64::MAX);
        let alive = elapsed_ms <= self.timeout.as_millis() as u64;
        self.alive.store(alive, Ordering::SeqCst);
        alive
    }

    /// Returns the elapsed time since the last heartbeat.
    pub fn elapsed(&self) -> Duration {
        self.last_beat
            .lock()
            .map(|t| Duration::from_millis(self.clock.mono_now().0.saturating_sub(t.0)))
            .unwrap_or(Duration::MAX)
    }

    pub fn name(&self) -> &'static str {
        self.name
    }
}

/// 3-layer hierarchical watchdog timer.
///
/// - **L1 systemd** — 30 s timeout (external, coarse)
/// - **L2 tokio runtime** — 10 s timeout (runtime-level)
/// - **L3 reasoning** — 5 min timeout (application-level)
///
/// Each layer has its own heartbeat sender and timeout checker.
pub struct WatchdogTimer {
    pub l1_systemd: HeartbeatLayer,
    pub l2_runtime: HeartbeatLayer,
    pub l3_reasoning: HeartbeatLayer,
}

impl WatchdogTimer {
    pub fn new(clock: Arc<dyn Clock>) -> Self {
        Self {
            l1_systemd: HeartbeatLayer::new("L1-systemd", Duration::from_secs(30), clock.clone()),
            l2_runtime: HeartbeatLayer::new("L2-runtime", Duration::from_secs(10), clock.clone()),
            l3_reasoning: HeartbeatLayer::new("L3-reasoning", Duration::from_secs(300), clock),
        }
    }

    /// Returns `true` if **all** layers are alive.
    pub fn all_alive(&self) -> bool {
        self.l1_systemd.is_alive() && self.l2_runtime.is_alive() && self.l3_reasoning.is_alive()
    }

    /// Returns the names of layers that have timed out.
    pub fn expired_layers(&self) -> Vec<&'static str> {
        let mut expired = Vec::new();
        if !self.l1_systemd.is_alive() {
            expired.push(self.l1_systemd.name());
        }
        if !self.l2_runtime.is_alive() {
            expired.push(self.l2_runtime.name());
        }
        if !self.l3_reasoning.is_alive() {
            expired.push(self.l3_reasoning.name());
        }
        expired
    }

    /// Spawn a background task that periodically checks all layers and logs
    /// warnings for any that have expired.  Returns a `JoinHandle` that can
    /// be aborted to stop the checker.
    pub fn spawn_checker(self: &Arc<Self>) -> tokio::task::JoinHandle<()> {
        let wd = Arc::clone(self);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(Duration::from_secs(5));
            loop {
                interval.tick().await;
                let expired = wd.expired_layers();
                if !expired.is_empty() {
                    warn!(layers = ?expired, "Watchdog layers expired");
                } else {
                    info!("All watchdog layers alive");
                }
            }
        })
    }
}

impl Default for WatchdogTimer {
    fn default() -> Self {
        Self::new(Arc::new(aletheon_kernel::chronos::SystemClock::new()))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn Clock> {
        // Start at mono=600_000 (10 min) so MonoTime(0) is convincingly expired
        // for all watchdog layers (L2=10s, L3=5min).
        Arc::new(TestClock::new(0, 600_000))
    }

    #[test]
    fn test_heartbeat_basics() {
        let layer = HeartbeatLayer::new("test", Duration::from_secs(1), test_clock());
        assert!(layer.is_alive());
        assert_eq!(layer.name(), "test");
    }

    #[test]
    fn test_heartbeat_timeout() {
        let layer = HeartbeatLayer::new("test", Duration::from_millis(50), test_clock());
        // Immediately alive
        assert!(layer.is_alive());
        // Artificially expire by setting last_beat far in the past
        {
            let mut t = layer.last_beat.lock().unwrap();
            *t = MonoTime(0);
        }
        assert!(!layer.is_alive());
        // Beat resets the timer
        layer.beat();
        assert!(layer.is_alive());
    }

    #[test]
    fn test_watchdog_all_alive_initially() {
        let wd = WatchdogTimer::new(test_clock());
        assert!(wd.all_alive());
        assert!(wd.expired_layers().is_empty());
    }

    #[test]
    fn test_watchdog_expired_layers() {
        let wd = WatchdogTimer::new(test_clock());
        // Artificially expire L2 by setting its last_beat far in the past.
        {
            let mut t = wd.l2_runtime.last_beat.lock().unwrap();
            *t = MonoTime(0);
        }
        let expired = wd.expired_layers();
        assert!(expired.contains(&"L2-runtime"));
        assert!(!wd.all_alive());
    }

    #[tokio::test]
    async fn test_spawn_checker_no_panic() {
        let wd = Arc::new(WatchdogTimer::new(test_clock()));
        let handle = wd.spawn_checker();
        // Let it tick a couple of times
        Timer::sleep(&*test_clock(), Duration::from_millis(1200)).await;
        handle.abort();
        assert!(handle.await.unwrap_err().is_cancelled());
    }
}
