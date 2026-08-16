//! Cache-relevant request prefix identity.
//!
//! `InferencePrefixShape` is a *diagnostic identity* of the stable request
//! prefix for one turn. It explains whether the host kept every input it
//! controls stable across turns (so a provider prefix cache could be reused);
//! it never gates inference and it is not a claim about whether the provider
//! actually hit cache. Shape changes are surfaced as typed `LocalMissReason`s.

use ::contracts::llm_types::{tool_schema_digest, ToolDefinition};
use serde::Serialize;
use sha2::{Digest, Sha256};
use std::collections::{HashMap, VecDeque};

pub use crate::prefix_cache_observability::{
    prefix_shape_metrics, record_prefix_shape_miss, LocalMissReason,
};

/// Version of the prefix-shape identity. Bump when digest inputs change.
pub const INFERENCE_PREFIX_SHAPE_VERSION: u16 = 1;

/// Domain separator for the full shape digest.
const SHAPE_DOMAIN: &[u8] = b"aletheon.inference-prefix-shape.v1\0";
/// Domain separator for the system-prefix digest.
const SYSTEM_PREFIX_DOMAIN: &[u8] = b"aletheon.system-prefix.v1\0";

/// Stable host-owned facts that define the cache-relevant request prefix.
///
/// Never contains a secret, the raw system prefix, or any user content — only
/// digests and stable identity.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct InferencePrefixShape {
    pub version: u16,
    pub provider_id: String,
    pub model_id: String,
    pub transport: String,
    pub system_prefix_digest: String,
    pub tool_schema_digest: String,
    pub agent_profile_digest: String,
    pub rewrite_version: u64,
}

impl InferencePrefixShape {
    /// Compute a shape from host-owned turn facts. Tool definitions are
    /// canonicalized by `::contracts::tool_schema_digest` (order and object-key
    /// independent). `system_prefix` must be the stable system prefix only —
    /// memory, goal, Dasein, tool results and the current input never enter it.
    pub fn compute(
        provider_id: &str,
        model_id: &str,
        transport: &str,
        system_prefix: &str,
        tools: &[ToolDefinition],
        agent_profile_digest: &str,
        rewrite_version: u64,
    ) -> anyhow::Result<Self> {
        let system_prefix_digest =
            sha256_domain_hex(SYSTEM_PREFIX_DOMAIN, system_prefix.as_bytes());
        let tools_digest = tool_schema_digest(tools)?;
        Ok(Self {
            version: INFERENCE_PREFIX_SHAPE_VERSION,
            provider_id: provider_id.to_owned(),
            model_id: model_id.to_owned(),
            transport: transport.to_owned(),
            system_prefix_digest,
            tool_schema_digest: tools_digest,
            agent_profile_digest: agent_profile_digest.to_owned(),
            rewrite_version,
        })
    }

    /// Full deterministic SHA-256 digest of the shape (diagnostic identity).
    pub fn digest(&self) -> String {
        let encoded = serde_json::to_vec(self).expect("shape serializes");
        sha256_domain_hex(SHAPE_DOMAIN, &encoded)
    }

    /// Compare against a previous shape and report the first host-controlled
    /// difference, or `None` when identical. When the shape is identical but
    /// the provider still reports a miss, record
    /// `LocalMissReason::ProviderMissOrEviction` — never claim a local cause.
    pub fn compare(&self, previous: &InferencePrefixShape) -> Option<LocalMissReason> {
        if previous.provider_id != self.provider_id || previous.model_id != self.model_id {
            return Some(LocalMissReason::ProviderOrModelChanged);
        }
        if previous.transport != self.transport {
            return Some(LocalMissReason::TransportChanged);
        }
        if previous.system_prefix_digest != self.system_prefix_digest {
            return Some(LocalMissReason::SystemChanged);
        }
        if previous.tool_schema_digest != self.tool_schema_digest {
            return Some(LocalMissReason::ToolSchemaChanged);
        }
        if previous.agent_profile_digest != self.agent_profile_digest {
            return Some(LocalMissReason::ProfileChanged);
        }
        if previous.rewrite_version != self.rewrite_version {
            return Some(LocalMissReason::CompactionOrRewrite);
        }
        None
    }
}

/// Deterministic bootstrap identity for an agent profile. Uses the active
/// profile name so switching profiles changes the shape and produces a
/// `ProfileChanged` local miss reason; the authoritative per-turn profile
/// digest can replace this once a profile registry exposes system-prompt
/// content to the daemon host.
pub fn agent_profile_digest(profile_name: &str) -> String {
    sha256_domain_hex(b"aletheon.agent-profile.v1\0", profile_name.as_bytes())
}

/// Tracks the last observed prefix shape across turns.
#[derive(Debug, Default, Clone)]
pub struct PrefixShapeTracker {
    last: Option<InferencePrefixShape>,
}

/// Bounded per-thread tracker store for a long-running daemon. Tracking is
/// diagnostic, so evicting the least-recently-used inactive thread only loses
/// one comparison; it must never allow unbounded thread IDs to grow host
/// memory.
#[derive(Debug)]
pub struct PrefixShapeTrackerStore {
    trackers: HashMap<String, PrefixShapeTracker>,
    order: VecDeque<String>,
    capacity: usize,
}

impl Default for PrefixShapeTrackerStore {
    fn default() -> Self {
        Self::new(4_096)
    }
}

impl PrefixShapeTrackerStore {
    pub fn new(capacity: usize) -> Self {
        Self {
            trackers: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
        }
    }

    pub fn track(
        &mut self,
        thread_id: &str,
        shape: &InferencePrefixShape,
    ) -> (bool, Option<LocalMissReason>) {
        let had_previous = self.trackers.contains_key(thread_id);
        if !had_previous {
            while self.trackers.len() >= self.capacity {
                if let Some(oldest) = self.order.pop_front() {
                    self.trackers.remove(&oldest);
                }
            }
        }
        self.order.retain(|candidate| candidate != thread_id);
        self.order.push_back(thread_id.to_owned());
        let reason = self
            .trackers
            .entry(thread_id.to_owned())
            .or_default()
            .track(shape);
        (had_previous, reason)
    }

    #[cfg(test)]
    fn contains(&self, thread_id: &str) -> bool {
        self.trackers.contains_key(thread_id)
    }
}

impl PrefixShapeTracker {
    /// Record the current shape. Returns the reason the shape changed versus
    /// the previously recorded shape, or `None` when it is identical (or on
    /// first observation).
    pub fn track(&mut self, shape: &InferencePrefixShape) -> Option<LocalMissReason> {
        let reason = self
            .last
            .as_ref()
            .and_then(|previous| shape.compare(previous));
        self.last = Some(shape.clone());
        reason
    }

    /// The most recently recorded shape.
    pub fn last(&self) -> Option<&InferencePrefixShape> {
        self.last.as_ref()
    }
}

fn sha256_domain_hex(domain: &[u8], value: &[u8]) -> String {
    let mut hasher = Sha256::new();
    hasher.update(domain);
    hasher.update(value);
    format!("sha256:{:x}", hasher.finalize())
}

/// Tracks session-wide cache statistics.
#[derive(Debug, Default)]
pub struct CacheStats {
    pub total_hit_tokens: u64,
    pub total_miss_tokens: u64,
}

impl CacheStats {
    /// Record cache hit/miss from an LLM response.
    pub fn record(&mut self, hit_tokens: u64, miss_tokens: u64) {
        self.total_hit_tokens += hit_tokens;
        self.total_miss_tokens += miss_tokens;
    }

    /// Cache hit rate as a percentage (0.0 - 1.0).
    pub fn hit_rate(&self) -> f64 {
        let total = self.total_hit_tokens + self.total_miss_tokens;
        if total == 0 {
            return 0.0;
        }
        self.total_hit_tokens as f64 / total as f64
    }

    /// Total tokens processed.
    pub fn total_tokens(&self) -> u64 {
        self.total_hit_tokens + self.total_miss_tokens
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    fn tool(name: &str, description: &str) -> ToolDefinition {
        ToolDefinition {
            name: name.into(),
            description: description.into(),
            input_schema: json!({ "type": "object" }),
        }
    }

    fn shape(system: &str, tools: &[ToolDefinition], profile: &str) -> InferencePrefixShape {
        InferencePrefixShape::compute(
            "lejurobot_deepseek",
            "deepseek/deepseek-v4-flash[1m]",
            "provider-a",
            system,
            tools,
            profile,
            0,
        )
        .unwrap()
    }

    #[test]
    fn compute_is_deterministic_and_sha256() {
        let a = shape(
            "prefix",
            &[tool("alpha", "one"), tool("zeta", "two")],
            "profile-a",
        );
        let b = shape(
            "prefix",
            &[tool("alpha", "one"), tool("zeta", "two")],
            "profile-a",
        );
        assert_eq!(a.digest(), b.digest());
        assert!(a.digest().starts_with("sha256:"));
        // A digest never embeds the prefix text, a secret, or user content.
        assert!(!a.digest().contains("prefix"));
        assert!(!a.system_prefix_digest.contains("prefix"));
    }

    #[test]
    fn tool_order_and_object_key_independent() {
        let left = shape("sys", &[tool("zeta", "z"), tool("alpha", "a")], "profile-a");
        let right = shape("sys", &[tool("alpha", "a"), tool("zeta", "z")], "profile-a");
        assert_eq!(left.tool_schema_digest, right.tool_schema_digest);
        assert_eq!(left.digest(), right.digest());
    }

    #[test]
    fn tool_schema_change_detected() {
        let a = shape("sys", &[tool("alpha", "one")], "profile-a");
        let b = shape("sys", &[tool("alpha", "two")], "profile-a");
        assert_eq!(b.compare(&a), Some(LocalMissReason::ToolSchemaChanged));
    }

    #[test]
    fn system_change_detected() {
        let a = shape("prompt A", &[tool("alpha", "one")], "profile-a");
        let b = shape("prompt B", &[tool("alpha", "one")], "profile-a");
        assert_eq!(b.compare(&a), Some(LocalMissReason::SystemChanged));
    }

    #[test]
    fn memory_and_user_input_never_enter_shape() {
        // Memory, goal, Dasein and current input are not inputs to compute();
        // identical prefix/tools/profile yield an identical shape.
        let a = shape("stable", &[tool("alpha", "one")], "profile-a");
        let b = shape("stable", &[tool("alpha", "one")], "profile-a");
        assert_eq!(a.compare(&b), None);
    }

    #[test]
    fn profile_change_detected() {
        let a = shape("sys", &[tool("alpha", "one")], "profile-a");
        let b = shape("sys", &[tool("alpha", "one")], "profile-b");
        assert_eq!(b.compare(&a), Some(LocalMissReason::ProfileChanged));
    }

    #[test]
    fn provider_or_model_change_detected() {
        let a = shape("sys", &[tool("alpha", "one")], "profile-a");
        let mut b = shape("sys", &[tool("alpha", "one")], "profile-a");
        b.model_id = "deepseek/deepseek-v4-pro[1m]".into();
        assert_eq!(b.compare(&a), Some(LocalMissReason::ProviderOrModelChanged));
    }

    #[test]
    fn transport_change_detected() {
        let a = shape("sys", &[tool("alpha", "one")], "profile-a");
        let mut b = shape("sys", &[tool("alpha", "one")], "profile-a");
        b.transport = "transport-b".into();
        assert_eq!(b.compare(&a), Some(LocalMissReason::TransportChanged));
    }

    #[test]
    fn compaction_detected() {
        let a = shape("sys", &[tool("alpha", "one")], "profile-a");
        let mut b = shape("sys", &[tool("alpha", "one")], "profile-a");
        b.rewrite_version = 1;
        assert_eq!(b.compare(&a), Some(LocalMissReason::CompactionOrRewrite));
    }

    #[test]
    fn tracker_reports_first_change_then_stable() {
        let mut tracker = PrefixShapeTracker::default();
        let a = shape("sys", &[tool("alpha", "one")], "profile-a");
        assert_eq!(tracker.track(&a), None); // first observation
        assert_eq!(tracker.track(&a), None); // stable
        let b = shape("sys", &[tool("alpha", "two")], "profile-a");
        assert_eq!(tracker.track(&b), Some(LocalMissReason::ToolSchemaChanged));
        assert_eq!(tracker.track(&b), None);
        assert_eq!(tracker.last(), Some(&b));
    }

    #[test]
    fn tracker_store_is_bounded_and_refreshes_recency() {
        let mut store = PrefixShapeTrackerStore::new(2);
        let a = shape("a", &[tool("alpha", "one")], "profile-a");
        let b = shape("b", &[tool("alpha", "one")], "profile-a");
        assert_eq!(store.track("thread-a", &a), (false, None));
        assert_eq!(store.track("thread-b", &a), (false, None));
        assert_eq!(store.track("thread-a", &a), (true, None));
        assert_eq!(store.track("thread-c", &b), (false, None));
        assert!(store.contains("thread-a"));
        assert!(!store.contains("thread-b"));
        assert!(store.contains("thread-c"));
    }

    #[test]
    fn stats_hit_rate() {
        let mut stats = CacheStats::default();
        stats.record(1000, 200);
        assert!((stats.hit_rate() - 0.833).abs() < 0.01);
        assert_eq!(stats.total_tokens(), 1200);
    }

    #[test]
    fn stats_empty() {
        let stats = CacheStats::default();
        assert_eq!(stats.hit_rate(), 0.0);
    }
}
