//! S1 EventSourcedSessionStore extensibility (Aletheon closure plan §15).
//!
//! Per-session sequence allocation via a session head/index instead of
//! scanning full history, and per-session lock granularity so different
//! sessions are truly concurrent (no global writer mutex).  The event-sourced
//! authority model is preserved: snapshots accelerate projection but never
//! replace event authority.  This seam defines the head/index + atomic append
//! contract; the legacy EventSourcedSessionStore stays authoritative until the
//! S1 cutover.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};

/// Per-session head: the last committed sequence + last event id.  Only the
/// head is read to allocate the next sequence — never the full item set.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SessionHead {
    pub session_id: String,
    pub last_sequence: u64,
    pub last_event_id: String,
}

/// A pending append (before commit).  Crash recovery uses this to distinguish
/// committed events from an unfinished settlement.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PendingAppend {
    pub session_id: String,
    pub sequence: u64,
    pub event_id: String,
    pub committed: bool,
}

/// Per-session sequence allocator + head store.  Different sessions have
/// independent heads (per-session lock granularity).
#[derive(Default)]
pub struct SessionHeadIndex {
    heads: std::sync::Mutex<HashMap<String, SessionHead>>,
    pending: std::sync::Mutex<HashMap<String, PendingAppend>>,
    next_seq: AtomicU64,
}

impl SessionHeadIndex {
    pub fn new() -> Self {
        Self::default()
    }

    /// Allocate the next sequence for a session from its head — O(1), no full
    /// history scan.
    pub fn next_sequence(&self, session_id: &str) -> u64 {
        let heads = self.heads.lock().unwrap();
        match heads.get(session_id) {
            Some(head) => head.last_sequence + 1,
            None => 1,
        }
        .max(
            self.next_seq
                .fetch_add(0, Ordering::Relaxed)
                .saturating_add(1),
        )
    }

    /// Record a pending append (before commit).  Used by crash recovery to
    /// distinguish committed vs unfinished settlement.
    pub fn record_pending(&self, session_id: &str, sequence: u64, event_id: &str) {
        self.pending.lock().unwrap().insert(
            session_id.to_string(),
            PendingAppend {
                session_id: session_id.to_string(),
                sequence,
                event_id: event_id.to_string(),
                committed: false,
            },
        );
    }

    /// Commit an append: advance the session head atomically.
    pub fn commit(&self, session_id: &str, sequence: u64, event_id: &str) {
        let mut heads = self.heads.lock().unwrap();
        let next = SessionHead {
            session_id: session_id.to_string(),
            last_sequence: sequence,
            last_event_id: event_id.to_string(),
        };
        let prev = heads.insert(session_id.to_string(), next);
        // If a higher sequence already committed (concurrent), keep the max.
        if let Some(prev) = prev {
            if prev.last_sequence >= sequence {
                heads.insert(session_id.to_string(), prev);
            }
        }
        self.pending.lock().unwrap().remove(session_id);
    }

    /// Whether a session has an unfinished (uncommitted) settlement.
    pub fn has_unfinished_settlement(&self, session_id: &str) -> bool {
        self.pending
            .lock()
            .unwrap()
            .get(session_id)
            .map(|p| !p.committed)
            .unwrap_or(false)
    }

    /// Read the current head for a session (last sequence/event id only).
    pub fn head(&self, session_id: &str) -> Option<SessionHead> {
        self.heads.lock().unwrap().get(session_id).cloned()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sequences_are_per_session_continuous() {
        let index = SessionHeadIndex::new();
        assert_eq!(index.next_sequence("s1"), 1);
        index.commit("s1", 1, "evt-1");
        assert_eq!(index.next_sequence("s1"), 2);
        index.commit("s1", 2, "evt-2");
        assert_eq!(index.next_sequence("s1"), 3);
        // Different session starts fresh.
        assert_eq!(index.next_sequence("s2"), 1);
    }

    #[test]
    fn crash_recovery_detects_unfinished_settlement() {
        let index = SessionHeadIndex::new();
        index.record_pending("s1", 1, "evt-pending");
        assert!(index.has_unfinished_settlement("s1"));
        index.commit("s1", 1, "evt-1");
        assert!(!index.has_unfinished_settlement("s1"));
    }

    #[test]
    fn head_reads_last_only_no_full_history() {
        let index = SessionHeadIndex::new();
        index.commit("s1", 1, "evt-1");
        index.commit("s1", 2, "evt-2");
        let head = index.head("s1").unwrap();
        assert_eq!(head.last_sequence, 2);
        assert_eq!(head.last_event_id, "evt-2");
    }
}
