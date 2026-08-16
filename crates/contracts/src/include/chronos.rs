//! Chronos clock and timer contracts.
//!
//! The [`Clock`] trait provides wall-clock and monotonic time sources.
//! The [`Timer`] trait provides async sleep / timeout operations.
//!
//! Implementations live in `kernel::chronos` (see
//! `SystemClock`, `SystemTimer`, `TestClock`, `TestTimer`).

use std::future::Future;
use std::time::Duration;

use crate::types::time::{MonoDeadline, MonoTime, WallTime};

// ============================================================================
// Clock
// ============================================================================

pub trait Clock: Send + Sync {
    fn wall_now(&self) -> WallTime;
    fn mono_now(&self) -> MonoTime;
}

// ============================================================================
// Timer
// ============================================================================

/// Error returned when a [`Timer::timeout`] expires.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Elapsed;

impl std::fmt::Display for Elapsed {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("timer elapsed")
    }
}

impl std::error::Error for Elapsed {}

/// Async timer abstraction.
///
/// Implementations in `kernel::chronos`:
/// - `SystemTimer` — delegates to `tokio::time` (production).
/// - `TestTimer`  — uses a Notify priority-queue driven by clock
///   advancement for deterministic tests.
pub trait Timer: Send + Sync {
    /// Check whether `deadline` has expired at the given monotonic time.
    fn is_expired(&self, now: MonoTime, deadline: MonoDeadline) -> bool {
        deadline.is_expired_at(now)
    }

    /// Sleep for `dur`.
    fn sleep(&self, dur: Duration) -> impl Future<Output = ()> + Send;

    /// Run `fut` with a timeout.  Returns [`Elapsed`] if `dur` passes
    /// before the future completes.
    fn timeout<F>(
        &self,
        dur: Duration,
        fut: F,
    ) -> impl Future<Output = Result<F::Output, Elapsed>> + Send
    where
        F: Future + Send,
        F::Output: Send;
}
