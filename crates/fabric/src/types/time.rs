//! Explicit wall/monotonic time contracts for kernel scheduling.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct WallTime(pub i64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MonoTime(pub u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
pub struct MonoDeadline(pub MonoTime);

impl MonoDeadline {
    pub fn after(base: MonoTime, millis: u64) -> Self {
        Self(MonoTime(base.0.saturating_add(millis)))
    }

    pub fn is_expired_at(&self, now: MonoTime) -> bool {
        now >= self.0
    }
}
