use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;

use fabric::MonoTime;

/// Helper: compute elapsed Duration between two MonoTime values.
fn mono_elapsed(now: MonoTime, earlier: MonoTime) -> Duration {
    Duration::from_millis(now.0.saturating_sub(earlier.0))
}

/// Sliding-window counter for a single event source.
struct SlidingWindow {
    timestamps: Vec<MonoTime>,
    window: Duration,
    max_events: u32,
}

impl SlidingWindow {
    fn new(window: Duration, max_events: u32) -> Self {
        Self {
            timestamps: Vec::new(),
            window,
            max_events,
        }
    }

    fn evict(&mut self, now: MonoTime) {
        self.timestamps
            .retain(|&t| mono_elapsed(now, t) <= self.window);
    }

    fn record_and_check(&mut self, now: MonoTime) -> bool {
        self.evict(now);
        if self.timestamps.len() as u32 >= self.max_events {
            return false; // flooded
        }
        self.timestamps.push(now);
        true
    }

    fn count(&mut self, now: MonoTime) -> u32 {
        self.evict(now);
        self.timestamps.len() as u32
    }
}

/// Result of a flood check.
#[derive(Debug, Clone, PartialEq)]
pub enum FloodResult {
    /// Event accepted.
    Accept,
    /// Source is flooding — event should be dropped.
    Flooded {
        source: String,
        count: u32,
        limit: u32,
    },
}

/// Detects and suppresses floods of events from any registered source.
///
/// Each source (e.g. a tool name, topic, or log category) gets an independent
/// sliding window. When a source exceeds its per-window event limit, subsequent
/// events are classified as flooded until the window drains.
pub struct EventFloodProtector {
    per_source: HashMap<String, SlidingWindow>,
    default_window: Duration,
    default_max_events: u32,
    clock: Arc<dyn fabric::Clock>,
}

impl EventFloodProtector {
    /// Create a protector with a default window and max-events-per-window.
    pub fn new(window: Duration, max_events: u32, clock: Arc<dyn fabric::Clock>) -> Self {
        Self {
            per_source: HashMap::new(),
            default_window: window,
            default_max_events: max_events,
            clock,
        }
    }

    /// Register a source with custom limits (overrides defaults).
    pub fn set_source_limit(&mut self, source: &str, window: Duration, max_events: u32) {
        self.per_source
            .insert(source.to_string(), SlidingWindow::new(window, max_events));
    }

    /// Record an event from `source` and return whether it was accepted.
    pub fn record(&mut self, source: &str) -> FloodResult {
        let now = self.clock.mono_now();
        let window = self
            .per_source
            .entry(source.to_string())
            .or_insert_with(|| SlidingWindow::new(self.default_window, self.default_max_events));

        let limit = window.max_events;
        if window.record_and_check(now) {
            FloodResult::Accept
        } else {
            FloodResult::Flooded {
                source: source.to_string(),
                count: window.count(now),
                limit,
            }
        }
    }

    /// Query the current event count for a source (for diagnostics).
    pub fn source_count(&mut self, source: &str) -> u32 {
        let now = self.clock.mono_now();
        self.per_source
            .get_mut(source)
            .map(|w| w.count(now))
            .unwrap_or(0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(TestClock::default())
    }

    fn test_protector(window: Duration, max_events: u32) -> EventFloodProtector {
        EventFloodProtector::new(window, max_events, test_clock())
    }

    #[test]
    fn accepts_events_under_limit() {
        let mut fp = test_protector(Duration::from_secs(1), 5);
        for _ in 0..5 {
            assert_eq!(fp.record("topic_a"), FloodResult::Accept);
        }
    }

    #[test]
    fn detects_flood_after_limit() {
        let mut fp = test_protector(Duration::from_secs(1), 3);
        assert_eq!(fp.record("spam"), FloodResult::Accept);
        assert_eq!(fp.record("spam"), FloodResult::Accept);
        assert_eq!(fp.record("spam"), FloodResult::Accept);
        match fp.record("spam") {
            FloodResult::Flooded { source, limit, .. } => {
                assert_eq!(source, "spam");
                assert_eq!(limit, 3);
            }
            other => panic!("expected Flooded, got {:?}", other),
        }
    }

    #[test]
    fn independent_sources() {
        let mut fp = test_protector(Duration::from_secs(1), 2);
        assert_eq!(fp.record("a"), FloodResult::Accept);
        assert_eq!(fp.record("a"), FloodResult::Accept);
        // a is now at limit, but b should still work.
        assert_eq!(fp.record("b"), FloodResult::Accept);
    }

    #[test]
    fn custom_source_limits() {
        let mut fp = test_protector(Duration::from_secs(60), 1000);
        fp.set_source_limit("critical", Duration::from_secs(1), 1);
        assert_eq!(fp.record("critical"), FloodResult::Accept);
        match fp.record("critical") {
            FloodResult::Flooded { .. } => {}
            other => panic!("expected Flooded, got {:?}", other),
        }
        // Other source still uses default.
        assert_eq!(fp.record("other"), FloodResult::Accept);
    }
}
