use fabric::{Clock, MonoTime, WallTime};
use std::sync::atomic::{AtomicI64, AtomicU64, Ordering};
use std::time::{Instant, SystemTime, UNIX_EPOCH};

/// Production clock: wall time from `SystemTime`, monotonic time from process-local `Instant`.
#[derive(Debug)]
pub struct SystemClock {
    mono_base: Instant,
}

impl SystemClock {
    pub fn new() -> Self {
        Self {
            mono_base: Instant::now(),
        }
    }
}

impl Default for SystemClock {
    fn default() -> Self {
        Self::new()
    }
}

impl Clock for SystemClock {
    fn wall_now(&self) -> WallTime {
        let millis = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as i64;
        WallTime(millis)
    }

    fn mono_now(&self) -> MonoTime {
        MonoTime(self.mono_base.elapsed().as_millis() as u64)
    }
}

/// Explicit virtual clock for deterministic timeout/deadline tests.
#[derive(Debug)]
pub struct TestClock {
    wall_ms: AtomicI64,
    mono_ms: AtomicU64,
}

impl TestClock {
    pub fn new(wall_ms: i64, mono_ms: u64) -> Self {
        Self {
            wall_ms: AtomicI64::new(wall_ms),
            mono_ms: AtomicU64::new(mono_ms),
        }
    }

    pub fn advance(&self, millis: u64) {
        self.mono_ms.fetch_add(millis, Ordering::SeqCst);
        self.wall_ms.fetch_add(millis as i64, Ordering::SeqCst);
    }
}

impl Default for TestClock {
    fn default() -> Self {
        Self::new(0, 0)
    }
}

impl Clock for TestClock {
    fn wall_now(&self) -> WallTime {
        WallTime(self.wall_ms.load(Ordering::SeqCst))
    }

    fn mono_now(&self) -> MonoTime {
        MonoTime(self.mono_ms.load(Ordering::SeqCst))
    }
}
