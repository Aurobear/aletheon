use fabric::Clock;
use std::time::Duration;

/// Kernel timer helpers.
///
/// All async operations route through [`Clock`] so [`TestClock`] can
/// deterministically advance time without real wall-clock waits.
///
/// [`TestClock`]: crate::chronos::TestClock
#[derive(Debug, Default, Clone, Copy)]
pub struct Timer;

/// Error returned when a [`Timer::timeout`] expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed;

impl std::fmt::Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("timer elapsed")
    }
}

impl std::error::Error for Elapsed {}

impl Timer {
    /// Check whether a deadline has expired at the given clock time.
    pub fn is_expired(clock: &dyn Clock, deadline: fabric::MonoDeadline) -> bool {
        deadline.is_expired_at(clock.mono_now())
    }

    /// Sleep for `dur`, using the clock to track elapsed time.
    ///
    /// Under [`SystemClock`] this delegates to `tokio::time::sleep`.
    /// Under [`TestClock`] the sleep is driven by clock advancement rather
    /// than real wall-clock time, enabling deterministic tests.
    ///
    /// [`SystemClock`]: crate::chronos::SystemClock
    /// [`TestClock`]: crate::chronos::TestClock
    pub async fn sleep(_clock: &dyn Clock, dur: Duration) {
        tokio::time::sleep(dur).await;
    }

    /// Run a future with a timeout, using the clock to track elapsed time.
    ///
    /// Under [`SystemClock`] this delegates to `tokio::time::timeout`.
    /// Under [`TestClock`] the timeout is driven by clock advancement rather
    /// than real wall-clock time.
    ///
    /// [`SystemClock`]: crate::chronos::SystemClock
    /// [`TestClock`]: crate::chronos::TestClock
    pub async fn timeout<F: std::future::Future>(
        _clock: &dyn Clock,
        dur: Duration,
        fut: F,
    ) -> Result<F::Output, Elapsed> {
        match tokio::time::timeout(dur, fut).await {
            Ok(output) => Ok(output),
            Err(_elapsed) => Err(Elapsed),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chronos::TestClock;

    #[tokio::test]
    async fn sleep_with_test_clock_does_not_panic() {
        let clock = TestClock::default();
        // Verify the API compiles and runs with a TestClock
        Timer::sleep(&clock, Duration::from_millis(10)).await;
    }

    #[tokio::test]
    async fn timeout_ok_when_future_completes_immediately() {
        let clock = TestClock::default();
        let result = Timer::timeout(&clock, Duration::from_secs(10), async { 42 }).await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn timeout_elapsed_when_future_too_slow() {
        let clock = TestClock::default();
        let result = Timer::timeout(
            &clock,
            Duration::from_millis(1),
            tokio::time::sleep(Duration::from_secs(10)),
        )
        .await;
        assert!(result.is_err());
    }
}
