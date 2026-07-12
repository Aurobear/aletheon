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

/// Convert a [`WallTime`] (milliseconds since epoch) to a [`chrono::DateTime<Utc>`].
///
/// This is the canonical bridge between kernel Clock timestamps and code that
/// needs chrono types (serialization, formatting, arithmetic).
/// Returns `DateTime::UNIX_EPOCH` if the millis value is out of chrono's range.
pub fn wall_to_datetime(wt: WallTime) -> chrono::DateTime<chrono::Utc> {
    chrono::DateTime::from_timestamp_millis(wt.0)
        .unwrap_or(chrono::DateTime::UNIX_EPOCH)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn wall_to_datetime_roundtrip() {
        let now = WallTime(1700000000000);
        let dt = wall_to_datetime(now);
        assert_eq!(dt.timestamp_millis(), 1700000000000);
    }

    #[test]
    fn wall_to_datetime_zero_returns_epoch() {
        let dt = wall_to_datetime(WallTime(0));
        assert_eq!(dt.timestamp_millis(), 0);
    }
}
