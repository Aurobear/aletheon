//! Chronos clock contracts.

use crate::types::time::{MonoDeadline, MonoTime, WallTime};
use std::time::Duration;

pub trait Clock: Send + Sync {
    fn wall_now(&self) -> WallTime;
    fn mono_now(&self) -> MonoTime;
}

/// Timer helpers that route through [`Clock`] so tests can deterministically
/// advance time without real wall-clock waits.
///
/// Delegates to `tokio::time` under the hood; the [`Clock`] parameter is
/// carried for future test-clock integration.
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
    pub fn is_expired(clock: &dyn Clock, deadline: MonoDeadline) -> bool {
        deadline.is_expired_at(clock.mono_now())
    }

    /// Sleep for `dur`, using the clock to track elapsed time.
    pub async fn sleep(_clock: &dyn Clock, dur: Duration) {
        tokio::time::sleep(dur).await;
    }

    /// Run a future with a timeout, using the clock to track elapsed time.
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
