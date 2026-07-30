//! Process-local counters for guarded context compaction outcomes.

use std::sync::atomic::{AtomicU64, Ordering};

static COMPACTION_DEGENERATE_TOTAL: AtomicU64 = AtomicU64::new(0);
static COMPACTION_SAMPLER_ERROR_TOTAL: AtomicU64 = AtomicU64::new(0);
static COMPACTION_EVICTED_MESSAGES_TOTAL: AtomicU64 = AtomicU64::new(0);

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct CompactionMetrics {
    pub degenerate_total: u64,
    pub sampler_error_total: u64,
    pub evicted_messages_total: u64,
}

pub fn compaction_metrics() -> CompactionMetrics {
    CompactionMetrics {
        degenerate_total: COMPACTION_DEGENERATE_TOTAL.load(Ordering::Relaxed),
        sampler_error_total: COMPACTION_SAMPLER_ERROR_TOTAL.load(Ordering::Relaxed),
        evicted_messages_total: COMPACTION_EVICTED_MESSAGES_TOTAL.load(Ordering::Relaxed),
    }
}

pub(super) fn record_degenerate() {
    COMPACTION_DEGENERATE_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub(super) fn record_sampler_error() {
    COMPACTION_SAMPLER_ERROR_TOTAL.fetch_add(1, Ordering::Relaxed);
}

pub(super) fn record_evicted(message_count: usize) {
    COMPACTION_EVICTED_MESSAGES_TOTAL.fetch_add(message_count as u64, Ordering::Relaxed);
}
