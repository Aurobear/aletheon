use std::sync::atomic::{AtomicU64, Ordering};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LocalMissReason {
    ProviderOrModelChanged,
    TransportChanged,
    SystemChanged,
    ToolSchemaChanged,
    ProfileChanged,
    CompactionOrRewrite,
    ProviderMissOrEviction,
}

impl LocalMissReason {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::ProviderOrModelChanged => "provider_or_model_changed",
            Self::TransportChanged => "transport_changed",
            Self::SystemChanged => "system_changed",
            Self::ToolSchemaChanged => "tool_schema_changed",
            Self::ProfileChanged => "profile_changed",
            Self::CompactionOrRewrite => "compaction_or_rewrite",
            Self::ProviderMissOrEviction => "provider_miss_or_eviction",
        }
    }
}

static COUNTERS: [AtomicU64; 7] = [const { AtomicU64::new(0) }; 7];

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct PrefixShapeMetricsSnapshot {
    pub provider_or_model_changed_total: u64,
    pub transport_changed_total: u64,
    pub system_changed_total: u64,
    pub tool_schema_changed_total: u64,
    pub profile_changed_total: u64,
    pub compaction_or_rewrite_total: u64,
    pub provider_miss_or_eviction_total: u64,
}

pub fn record_prefix_shape_miss(reason: LocalMissReason) {
    COUNTERS[reason as usize].fetch_add(1, Ordering::Relaxed);
}

pub fn prefix_shape_metrics() -> PrefixShapeMetricsSnapshot {
    let load = |index: usize| COUNTERS[index].load(Ordering::Relaxed);
    PrefixShapeMetricsSnapshot {
        provider_or_model_changed_total: load(0),
        transport_changed_total: load(1),
        system_changed_total: load(2),
        tool_schema_changed_total: load(3),
        profile_changed_total: load(4),
        compaction_or_rewrite_total: load(5),
        provider_miss_or_eviction_total: load(6),
    }
}
