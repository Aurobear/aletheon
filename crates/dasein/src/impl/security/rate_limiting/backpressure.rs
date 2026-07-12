use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use fabric::MonoTime;

/// Helper: compute elapsed Duration between two MonoTime values.
fn mono_elapsed(now: MonoTime, earlier: MonoTime) -> Duration {
    Duration::from_millis(now.0.saturating_sub(earlier.0))
}

/// Signals the system can emit to throttle upstream producers.
#[derive(Debug, Clone, PartialEq)]
pub enum BackpressureSignal {
    /// Producer should reduce its rate.
    SlowDown,
    /// System is shedding load — drop low-priority work.
    DropLowPriority,
    /// System is critically overloaded — pause the source entirely.
    PauseSource,
}

/// A single tracked source's pressure state.
struct SourceState {
    /// Sliding window of event timestamps.
    events: Vec<MonoTime>,
    /// How many consecutive over-threshold readings.
    over_count: u32,
}

/// Central backpressure manager that monitors multiple event sources and
/// emits escalating signals when one source exceeds its configured thresholds.
pub struct BackpressureManager {
    /// Per-source state.
    sources: HashMap<String, SourceState>,
    /// Sliding window duration.
    window: Duration,
    /// Thresholds for escalating signals.
    slow_down_threshold: u32,
    drop_threshold: u32,
    pause_threshold: u32,
    clock: Arc<dyn fabric::Clock>,
}

impl BackpressureManager {
    pub fn new(
        window: Duration,
        slow_down_threshold: u32,
        drop_threshold: u32,
        pause_threshold: u32,
        clock: Arc<dyn fabric::Clock>,
    ) -> Self {
        assert!(
            slow_down_threshold <= drop_threshold && drop_threshold <= pause_threshold,
            "thresholds must be non-decreasing"
        );
        Self {
            sources: HashMap::new(),
            window,
            slow_down_threshold,
            drop_threshold,
            pause_threshold,
            clock,
        }
    }

    fn evict(state: &mut SourceState, now: MonoTime, window: Duration) {
        state.events.retain(|&t| mono_elapsed(now, t) <= window);
    }

    /// Record an event from `source` and return a signal if pressure is high.
    pub fn record(&mut self, source: &str) -> Option<BackpressureSignal> {
        let now = self.clock.mono_now();
        let state = self
            .sources
            .entry(source.to_string())
            .or_insert(SourceState {
                events: Vec::new(),
                over_count: 0,
            });

        Self::evict(state, now, self.window);
        state.events.push(now);
        let count = state.events.len() as u32;

        if count >= self.pause_threshold {
            state.over_count += 1;
            Some(BackpressureSignal::PauseSource)
        } else if count >= self.drop_threshold {
            state.over_count += 1;
            Some(BackpressureSignal::DropLowPriority)
        } else if count >= self.slow_down_threshold {
            state.over_count += 1;
            Some(BackpressureSignal::SlowDown)
        } else {
            state.over_count = 0;
            None
        }
    }

    /// Query the current event count for a source.
    pub fn source_count(&mut self, source: &str) -> u32 {
        let now = self.clock.mono_now();
        self.sources
            .get_mut(source)
            .map(|s| {
                Self::evict(s, now, self.window);
                s.events.len() as u32
            })
            .unwrap_or(0)
    }

    /// Reset all state for a source (e.g. after the producer has been paused and resumed).
    pub fn reset_source(&mut self, source: &str) {
        self.sources.remove(source);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use aletheon_kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(TestClock::default())
    }

    fn test_mgr(window: Duration, slow: u32, drop: u32, pause: u32) -> BackpressureManager {
        BackpressureManager::new(window, slow, drop, pause, test_clock())
    }

    #[test]
    fn no_signal_below_slow_down() {
        let mut mgr = test_mgr(Duration::from_secs(1), 5, 10, 20);
        for _ in 0..4 {
            assert_eq!(mgr.record("src"), None);
        }
    }

    #[test]
    fn slow_down_signal() {
        let mut mgr = test_mgr(Duration::from_secs(1), 3, 10, 20);
        mgr.record("src");
        mgr.record("src");
        assert_eq!(mgr.record("src"), Some(BackpressureSignal::SlowDown));
    }

    #[test]
    fn drop_low_priority_signal() {
        let mut mgr = test_mgr(Duration::from_secs(1), 3, 5, 20);
        for _ in 0..4 {
            mgr.record("src");
        }
        assert_eq!(mgr.record("src"), Some(BackpressureSignal::DropLowPriority));
    }

    #[test]
    fn pause_source_signal() {
        let mut mgr = test_mgr(Duration::from_secs(1), 3, 5, 7);
        for _ in 0..6 {
            mgr.record("src");
        }
        assert_eq!(mgr.record("src"), Some(BackpressureSignal::PauseSource));
    }

    #[test]
    fn reset_source_clears_pressure() {
        let mut mgr = test_mgr(Duration::from_secs(1), 2, 5, 10);
        mgr.record("src");
        mgr.record("src");
        assert!(mgr.record("src").is_some());
        mgr.reset_source("src");
        assert_eq!(mgr.source_count("src"), 0);
        assert_eq!(mgr.record("src"), None);
    }
}
