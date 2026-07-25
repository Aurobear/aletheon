//! Connection state tracking.
//!
//! Manages the active connection counter for daemon health reporting
//! and graceful shutdown coordination.

use super::RequestHandler;
use std::sync::atomic::Ordering;

impl RequestHandler {
    /// Increment the active connection counter.
    pub fn increment_connections(&self) {
        self.active_connections.fetch_add(1, Ordering::Relaxed);
    }

    /// Decrement the active connection counter.
    pub fn decrement_connections(&self) {
        self.active_connections.fetch_sub(1, Ordering::Relaxed);
    }
}
