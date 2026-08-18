//! Connection state tracking.
//!
//! Manages the active connection counter for daemon health reporting
//! and graceful shutdown coordination.

use super::RequestHandler;
use std::sync::atomic::{AtomicUsize, Ordering};

impl RequestHandler {
    /// Atomically reserve one connection slot without overshooting the limit.
    pub fn try_increment_connections(&self) -> bool {
        try_reserve_connection(&self.active_connections, self.max_connections)
    }

    /// Decrement the active connection counter.
    pub fn decrement_connections(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }
}

fn try_reserve_connection(active: &AtomicUsize, limit: Option<usize>) -> bool {
    active
        .fetch_update(Ordering::AcqRel, Ordering::Acquire, |current| {
            if limit.is_some_and(|limit| current >= limit) {
                None
            } else {
                Some(current.saturating_add(1))
            }
        })
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn connection_reservation_never_overshoots_limit() {
        let active = AtomicUsize::new(0);
        assert!(try_reserve_connection(&active, Some(2)));
        assert!(try_reserve_connection(&active, Some(2)));
        assert!(!try_reserve_connection(&active, Some(2)));
        assert_eq!(active.load(Ordering::Acquire), 2);
    }
}
