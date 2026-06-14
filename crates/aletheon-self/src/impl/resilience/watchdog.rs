use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

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
    last_beat: Arc<std::sync::Mutex<Instant>>,
    alive: Arc<AtomicBool>,
}

impl HeartbeatLayer {
    pub fn new(name: &'static str, timeout: Duration) -> Self {
        let (tx, rx) = watch::channel(());
        Self {
            name,
            timeout,
            tx,
            rx,
            last_beat: Arc::new(std::sync::Mutex::new(Instant::now())),
            alive: Arc::new(AtomicBool::new(true)),
        }
    }

    /// Send a heartbeat signal from the monitored task.
    pub fn beat(&self) {
        if let Ok(mut t) = self.last_beat.lock() {
            *t = Instant::now();
        }
        let _ = self.tx.send(());
    }

    /// Returns `true` if the last heartbeat was within the timeout window.
    pub fn is_alive(&self) -> bool {
        let elapsed = self
            .last_beat
            .lock()
            .map(|t| t.elapsed())
            .unwrap_or(Duration::MAX);
        let alive = elapsed <= self.timeout;
        self.alive.store(alive, Ordering::SeqCst);
        alive
    }

    /// Returns the elapsed time since the last heartbeat.
    pub fn elapsed(&self) -> Duration {
        self.last_beat
            .lock()
            .map(|t| t.elapsed())
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
/// - **L3 reasoning loop** — 5 min timeout (application-level)
///
/// Each layer has its own heartbeat sender and timeout checker.
pub struct WatchdogTimer {
    pub l1_systemd: HeartbeatLayer,
    pub l2_runtime: HeartbeatLayer,
    pub l3_reasoning: HeartbeatLayer,
}

impl WatchdogTimer {
    pub fn new() -> Self {
        Self {
            l1_systemd: HeartbeatLayer::new("L1-systemd", Duration::from_secs(30)),
            l2_runtime: HeartbeatLayer::new("L2-runtime", Duration::from_secs(10)),
            l3_reasoning: HeartbeatLayer::new("L3-reasoning", Duration::from_secs(300)),
        }
    }

    /// Returns `true` if **all** layers are alive.
    pub fn all_alive(&self) -> bool {
        self.l1_systemd.is_alive()
            && self.l2_runtime.is_alive()
            && self.l3_reasoning.is_alive()
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
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_heartbeat_basics() {
        let layer = HeartbeatLayer::new("test", Duration::from_secs(1));
        assert!(layer.is_alive());
        assert_eq!(layer.name(), "test");
    }

    #[test]
    fn test_heartbeat_timeout() {
        let layer = HeartbeatLayer::new("test", Duration::from_millis(50));
        // Immediately alive
        assert!(layer.is_alive());
        // Sleep past the timeout
        std::thread::sleep(Duration::from_millis(80));
        assert!(!layer.is_alive());
        // Beat resets the timer
        layer.beat();
        assert!(layer.is_alive());
    }

    #[test]
    fn test_watchdog_all_alive_initially() {
        let wd = WatchdogTimer::new();
        assert!(wd.all_alive());
        assert!(wd.expired_layers().is_empty());
    }

    #[test]
    fn test_watchdog_expired_layers() {
        let wd = WatchdogTimer::new();
        // Artificially expire L2 by setting its last_beat far in the past.
        {
            let mut t = wd.l2_runtime.last_beat.lock().unwrap();
            *t = Instant::now() - Duration::from_secs(60);
        }
        let expired = wd.expired_layers();
        assert!(expired.contains(&"L2-runtime"));
        assert!(!wd.all_alive());
    }

    #[tokio::test]
    async fn test_spawn_checker_no_panic() {
        let wd = Arc::new(WatchdogTimer::new());
        let handle = wd.spawn_checker();
        // Let it tick a couple of times
        tokio::time::sleep(Duration::from_millis(1200)).await;
        handle.abort();
        assert!(handle.await.unwrap_err().is_cancelled());
    }
}
