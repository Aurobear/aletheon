//! Agent process identity types.

use serde::{Deserialize, Serialize};
use std::sync::atomic::{AtomicU64, Ordering};

/// Agent process identifier — unique per runtime session.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct Pid(u64);

impl Pid {
    /// Create a new unique PID.
    pub fn new() -> Self {
        static NEXT_PID: AtomicU64 = AtomicU64::new(1);
        Self(NEXT_PID.fetch_add(1, Ordering::Relaxed))
    }

    pub fn as_u64(&self) -> u64 {
        self.0
    }
}

impl Default for Pid {
    fn default() -> Self {
        Self::new()
    }
}

impl std::fmt::Display for Pid {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "pid:{}", self.0)
    }
}
