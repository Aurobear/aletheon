//! Timer implementations: [`SystemTimer`] (production) and [`TestTimer`]
//! (deterministic tests), both implementing the [`fabric::Timer`] trait.

use std::collections::BTreeMap;
use std::future::Future;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use fabric::{Clock, Timer};
use tokio::sync::Notify;

use crate::chronos::TestClock;

// ============================================================================
// SystemTimer
// ============================================================================

/// Production timer — delegates to `tokio::time`.
#[derive(Debug, Default, Clone, Copy)]
pub struct SystemTimer;

#[allow(clippy::manual_async_fn)]
impl Timer for SystemTimer {
    fn sleep(&self, dur: Duration) -> impl Future<Output = ()> + Send {
        async move { tokio::time::sleep(dur).await }
    }

    fn timeout<F>(
        &self,
        dur: Duration,
        fut: F,
    ) -> impl Future<Output = Result<F::Output, fabric::Elapsed>> + Send
    where
        F: Future + Send,
        F::Output: Send,
    {
        async move {
            match tokio::time::timeout(dur, fut).await {
                Ok(output) => Ok(output),
                Err(_elapsed) => Err(fabric::Elapsed),
            }
        }
    }
}

// ============================================================================
// TestTimer
// ============================================================================

/// Deterministic timer for tests.
///
/// Maintains a priority queue of (mono-deadline-millis, Notify) entries.
/// [`TestTimer::advance(ms)`] advances the associated [`TestClock`] and
/// wakes every sleeper whose deadline has been reached.
///
/// # Example
///
/// ```ignore
/// let clock = Arc::new(TestClock::new(0, 0));
/// let timer = TestTimer::new(clock.clone());
///
/// let handle = tokio::spawn(async move {
///     timer.sleep(Duration::from_secs(5)).await;
///     42
/// });
///
/// timer.advance(6_000); // +6 seconds
/// assert_eq!(handle.await.unwrap(), 42);
/// ```
pub struct TestTimer {
    /// Sleepers keyed by monotonic deadline (milliseconds).
    sleepers: Mutex<BTreeMap<u64, Vec<Arc<Notify>>>>,
    /// Shared TestClock; mono and wall are advanced together.
    clock: Arc<TestClock>,
}

impl TestTimer {
    /// Create a new `TestTimer` backed by `clock`.
    pub fn new(clock: Arc<TestClock>) -> Self {
        Self {
            sleepers: Mutex::new(BTreeMap::new()),
            clock,
        }
    }

    /// Advance both this timer and the underlying clock by `millis`
    /// milliseconds.  All sleepers whose deadline ≤ the new monotonic time
    /// are woken before this method returns.
    pub fn advance(&self, millis: u64) {
        // 1. Advance the clock's internal counters.
        self.clock.advance(millis);

        // 2. Read the new monotonic time.
        let now = self.clock.mono_now().0;

        // 3. Drain every sleeper with deadline ≤ now.
        let expired: Vec<Arc<Notify>> = {
            let mut sleepers = self.sleepers.lock().unwrap_or_else(|e| e.into_inner());
            let mut to_wake = Vec::new();
            while let Some(entry) = sleepers.first_entry() {
                if *entry.key() > now {
                    break;
                }
                to_wake.extend(entry.remove());
            }
            to_wake
        };

        // 4. Wake all expired sleepers (outside the lock).
        for n in expired {
            n.notify_waiters();
        }
    }
}

#[allow(clippy::manual_async_fn)]
impl Timer for TestTimer {
    fn sleep(&self, dur: Duration) -> impl Future<Output = ()> + Send {
        let deadline = self.clock.mono_now().0 + dur.as_millis() as u64;

        // If already expired, return a no-op future.
        let already_expired = self.clock.mono_now().0 >= deadline;

        let notify = if already_expired {
            None
        } else {
            let n = Arc::new(Notify::new());
            let mut sleepers = self.sleepers.lock().unwrap_or_else(|e| e.into_inner());
            sleepers.entry(deadline).or_default().push(n.clone());
            Some(n)
        };

        async move {
            if let Some(n) = notify {
                n.notified().await;
            }
        }
    }

    fn timeout<F>(
        &self,
        dur: Duration,
        fut: F,
    ) -> impl Future<Output = Result<F::Output, fabric::Elapsed>> + Send
    where
        F: Future + Send,
        F::Output: Send,
    {
        let sleep_fut = Timer::sleep(self, dur);
        async move {
            tokio::select! {
                result = fut => Ok(result),
                () = sleep_fut => Err(fabric::Elapsed),
            }
        }
    }
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{MonoDeadline, MonoTime};

    #[tokio::test]
    async fn system_timer_sleep_short() {
        SystemTimer.sleep(Duration::from_millis(1)).await;
    }

    #[tokio::test]
    async fn system_timer_timeout_ok() {
        let result = SystemTimer
            .timeout(Duration::from_secs(10), async { 42 })
            .await;
        assert_eq!(result.unwrap(), 42);
    }

    #[tokio::test]
    async fn system_timer_timeout_elapsed() {
        let result = SystemTimer
            .timeout(
                Duration::from_millis(1),
                tokio::time::sleep(Duration::from_secs(10)),
            )
            .await;
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn test_timer_sleep_wakes_after_advance() {
        let clock = Arc::new(TestClock::new(0, 0));
        let timer = Arc::new(TestTimer::new(clock.clone()));

        let t = timer.clone();
        let handle = tokio::spawn(async move { t.sleep(Duration::from_secs(5)).await });

        tokio::time::sleep(Duration::from_millis(10)).await;
        timer.advance(6_000);

        tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap();
    }

    #[tokio::test]
    async fn test_timer_timeout_elapsed_after_advance() {
        let clock = Arc::new(TestClock::new(0, 0));
        let timer = Arc::new(TestTimer::new(clock.clone()));

        let t = timer.clone();
        let handle = tokio::spawn(async move {
            t.timeout(Duration::from_secs(5), std::future::pending::<()>())
                .await
        });

        tokio::time::sleep(Duration::from_millis(10)).await;
        timer.advance(6_000);

        let result = tokio::time::timeout(Duration::from_secs(1), handle)
            .await
            .unwrap()
            .unwrap();
        assert!(result.is_err());
    }

    #[tokio::test]
    async fn is_expired_works_correctly() {
        let clock = TestClock::new(0, 10);
        let timer = SystemTimer;
        let deadline = MonoDeadline::after(MonoTime(10), 25); // deadline at mono=35

        assert!(!timer.is_expired(MonoTime(10), deadline));
        clock.advance(24);
        assert!(!timer.is_expired(MonoTime(34), deadline));
        clock.advance(1);
        assert!(timer.is_expired(MonoTime(35), deadline));
    }
}
