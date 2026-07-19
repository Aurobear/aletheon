use std::collections::VecDeque;
use std::sync::Arc;
use std::time::Duration;

use fabric::MonoTime;

/// Helper: compute elapsed Duration between two MonoTime values.
fn mono_elapsed(now: MonoTime, earlier: MonoTime) -> Duration {
    Duration::from_millis(now.0.saturating_sub(earlier.0))
}

/// Action the limiter recommends when a token request arrives.
#[derive(Debug, Clone, PartialEq)]
pub enum ThrottleAction {
    /// Request is within limits — proceed.
    Allow,
    /// Request would exceed a soft limit — caller should wait this long.
    Delay(Duration),
    /// Request would exceed a hard limit — reject with the given reason.
    Reject(String),
}

/// Rolling window tracker for a single time bucket.
struct WindowCounter {
    timestamps: VecDeque<MonoTime>,
    max_count: u32,
    window: Duration,
}

impl WindowCounter {
    fn new(max_count: u32, window: Duration) -> Self {
        Self {
            timestamps: VecDeque::new(),
            max_count,
            window,
        }
    }

    fn evict(&mut self, now: MonoTime) {
        while let Some(&front) = self.timestamps.front() {
            if mono_elapsed(now, front) > self.window {
                self.timestamps.pop_front();
            } else {
                break;
            }
        }
    }

    fn count(&mut self, now: MonoTime) -> u32 {
        self.evict(now);
        self.timestamps.len() as u32
    }

    fn record(&mut self, now: MonoTime) {
        self.timestamps.push_back(now);
    }

    /// Returns how long until the oldest entry expires, or None if empty.
    fn time_until_slot(&self, now: MonoTime) -> Option<Duration> {
        self.timestamps.front().map(|&oldest| {
            let elapsed = mono_elapsed(now, oldest);
            if elapsed < self.window {
                self.window - elapsed
            } else {
                Duration::ZERO
            }
        })
    }
}

/// Token-level rate limiter with per-turn, per-hour, and per-day windows.
pub struct TokenRateLimiter {
    max_per_turn: u32,
    per_hour: WindowCounter,
    per_day: WindowCounter,
    turn_usage: u32,
    clock: Arc<dyn fabric::Clock>,
}

impl TokenRateLimiter {
    pub fn new(
        max_per_turn: u32,
        max_per_hour: u32,
        max_per_day: u32,
        clock: Arc<dyn fabric::Clock>,
    ) -> Self {
        Self {
            max_per_turn,
            per_hour: WindowCounter::new(max_per_hour, Duration::from_secs(3600)),
            per_day: WindowCounter::new(max_per_day, Duration::from_secs(86400)),
            turn_usage: 0,
            clock,
        }
    }

    /// Begin a new conversation turn — resets per-turn counter.
    pub fn begin_turn(&mut self) {
        self.turn_usage = 0;
    }

    /// Try to consume `count` tokens. Returns the recommended action.
    pub fn consume(&mut self, count: u32) -> ThrottleAction {
        let now = self.clock.mono_now();

        // Per-turn hard limit.
        if self.turn_usage + count > self.max_per_turn {
            return ThrottleAction::Reject(format!(
                "Per-turn limit exceeded: {} + {} > {}",
                self.turn_usage, count, self.max_per_turn
            ));
        }

        // Per-hour soft limit — suggest delay.
        let hour_count = self.per_hour.count(now);
        if hour_count + count > self.per_hour.max_count {
            if let Some(wait) = self.per_hour.time_until_slot(now) {
                return ThrottleAction::Delay(wait);
            }
        }

        // Per-day soft limit — suggest delay.
        let day_count = self.per_day.count(now);
        if day_count + count > self.per_day.max_count {
            if let Some(wait) = self.per_day.time_until_slot(now) {
                return ThrottleAction::Delay(wait);
            }
        }

        // All clear — record and allow.
        self.turn_usage += count;
        // Record individual token as a timestamp entry (batched for perf in production,
        // but per-token here for accuracy of the sliding window).
        for _ in 0..count {
            self.per_hour.record(now);
            self.per_day.record(now);
        }
        ThrottleAction::Allow
    }

    pub fn turn_usage(&self) -> u32 {
        self.turn_usage
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use kernel::chronos::TestClock;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(TestClock::default())
    }

    fn test_limiter(max_per_turn: u32, max_per_hour: u32, max_per_day: u32) -> TokenRateLimiter {
        TokenRateLimiter::new(max_per_turn, max_per_hour, max_per_day, test_clock())
    }

    #[test]
    fn allows_within_per_turn_limit() {
        let mut limiter = test_limiter(100, 10_000, 100_000);
        limiter.begin_turn();
        assert_eq!(limiter.consume(50), ThrottleAction::Allow);
        assert_eq!(limiter.consume(49), ThrottleAction::Allow);
    }

    #[test]
    fn rejects_when_per_turn_exceeded() {
        let mut limiter = test_limiter(10, 10_000, 100_000);
        limiter.begin_turn();
        assert_eq!(limiter.consume(5), ThrottleAction::Allow);
        assert_eq!(limiter.consume(5), ThrottleAction::Allow);
        match limiter.consume(1) {
            ThrottleAction::Reject(_) => {}
            other => panic!("expected Reject, got {:?}", other),
        }
    }

    #[test]
    fn begin_turn_resets_turn_usage() {
        let mut limiter = test_limiter(10, 10_000, 100_000);
        limiter.begin_turn();
        limiter.consume(10);
        limiter.begin_turn();
        assert_eq!(limiter.turn_usage(), 0);
        assert_eq!(limiter.consume(5), ThrottleAction::Allow);
    }

    #[test]
    fn delays_when_hourly_limit_approached() {
        // Very low hourly limit to trigger delay path.
        let mut limiter = test_limiter(1000, 5, 100_000);
        limiter.begin_turn();
        for _ in 0..5 {
            assert_eq!(limiter.consume(1), ThrottleAction::Allow);
        }
        match limiter.consume(1) {
            ThrottleAction::Delay(d) => assert!(d > Duration::ZERO),
            other => panic!("expected Delay, got {:?}", other),
        }
    }
}
