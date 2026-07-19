use crate::MonotonicInstant;
use std::sync::atomic::{AtomicU64, Ordering};

pub trait MonotonicClock: Send + Sync {
    fn now(&self) -> MonotonicInstant;
}
#[derive(Default)]
pub struct ManualClock(AtomicU64);
impl ManualClock {
    pub fn new(now: u64) -> Self {
        Self(AtomicU64::new(now))
    }
    pub fn advance_to(&self, next: u64) -> Result<(), String> {
        self.0
            .fetch_update(Ordering::SeqCst, Ordering::SeqCst, |current| {
                (next >= current).then_some(next)
            })
            .map(|_| ())
            .map_err(|_| "monotonic clock cannot move backwards".into())
    }
    pub fn advance_by(&self, delta: u64) {
        self.0.fetch_add(delta, Ordering::SeqCst);
    }
}
impl MonotonicClock for ManualClock {
    fn now(&self) -> MonotonicInstant {
        MonotonicInstant(self.0.load(Ordering::SeqCst))
    }
}
