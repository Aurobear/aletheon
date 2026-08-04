//! Bounded read-only tool result cache (Phase C7 of
//! docs/plans/deepseek-cache-and-message-optimization-plan.md).
//!
//! The host only consults this cache for tools that (a) explicitly declare a
//! non-`Never` `ToolCachePolicy` **and** (b) are read-only
//! (`permission_level() == L0`). The L0 gate is enforced by the executor, so a
//! mutating tool cannot be misconfigured into caching. A hit still yields an
//! auditable `CapabilityResult` marked `served_from_cache` and never bypasses
//! the permission/approval gate (which runs before the cache is consulted).
//! Cache failures fail open to the real read-only tool.

use std::collections::{BTreeMap, HashMap, VecDeque};
use std::time::{Duration, Instant};

use fabric::tool::{PermissionLevel, ToolCachePolicy};
use serde_json::Value;
use sha2::{Digest, Sha256};

const KEY_DOMAIN: &[u8] = b"aletheon.read-only-tool-cache.v1\0";

/// Host-owned cache key for a read-only tool result.
///
/// Includes the tool name, implementation version, canonical arguments,
/// workspace identity and the exact scope implied by the policy. Per-turn and
/// session policies can never cross their typed boundary; shared policies omit
/// principal identity only when the tool owner explicitly declares that safe.
#[derive(Debug, Clone, Copy)]
pub struct ToolCacheScope<'a> {
    pub workspace: &'a str,
    pub principal: &'a str,
    pub session: &'a str,
    pub turn: &'a str,
}

pub fn read_only_cache_key(
    tool_name: &str,
    impl_version: &str,
    args: &Value,
    scope: ToolCacheScope<'_>,
    policy: ToolCachePolicy,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(KEY_DOMAIN);
    hasher.update(tool_name.as_bytes());
    hasher.update(b"\0");
    hasher.update(impl_version.as_bytes());
    hasher.update(b"\0");
    hash_canonical_json(&mut hasher, args);
    hasher.update(b"\0");
    hasher.update(scope.workspace.as_bytes());
    hasher.update(b"\0");
    match policy {
        ToolCachePolicy::Never => hasher.update(b"never"),
        ToolCachePolicy::PerTurn => {
            hasher.update(b"turn\0");
            hasher.update(scope.principal.as_bytes());
            hasher.update(b"\0");
            hasher.update(scope.session.as_bytes());
            hasher.update(b"\0");
            hasher.update(scope.turn.as_bytes());
        }
        ToolCachePolicy::Session { .. } => {
            hasher.update(b"session\0");
            hasher.update(scope.principal.as_bytes());
            hasher.update(b"\0");
            hasher.update(scope.session.as_bytes());
        }
        ToolCachePolicy::SharedReadOnly {
            vary_by_principal, ..
        } => {
            hasher.update(b"shared\0");
            if vary_by_principal {
                hasher.update(scope.principal.as_bytes());
            }
        }
    }
    hasher.update(b"\0");
    hasher.update(format!("{policy:?}").as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// Feed JSON into a digest with explicit type tags and lexicographically sorted
/// object keys. This remains deterministic even when serde_json is compiled
/// with insertion-order map preservation.
fn hash_canonical_json(hasher: &mut Sha256, value: &Value) {
    match value {
        Value::Null => hasher.update(b"n"),
        Value::Bool(value) => hasher.update(if *value { b"b1" } else { b"b0" }),
        Value::Number(value) => {
            hasher.update(b"d");
            hasher.update(value.to_string().as_bytes());
        }
        Value::String(value) => {
            hasher.update(b"s");
            hasher.update(value.len().to_le_bytes());
            hasher.update(value.as_bytes());
        }
        Value::Array(values) => {
            hasher.update(b"a");
            hasher.update(values.len().to_le_bytes());
            for value in values {
                hash_canonical_json(hasher, value);
            }
        }
        Value::Object(values) => {
            hasher.update(b"o");
            hasher.update(values.len().to_le_bytes());
            let mut entries = values.iter().collect::<Vec<_>>();
            entries.sort_by(|left, right| left.0.cmp(right.0));
            for (key, value) in entries {
                hasher.update(key.len().to_le_bytes());
                hasher.update(key.as_bytes());
                hash_canonical_json(hasher, value);
            }
        }
    }
}

/// TTL for a declared policy. `Never` yields `None` (uncacheable). `PerTurn`
/// uses a short retention TTL only for cleanup; its key contains the exact turn
/// identity, so the value can never be served in another turn.
pub fn cache_ttl(policy: ToolCachePolicy) -> Option<Duration> {
    match policy {
        ToolCachePolicy::Session { ttl_ms } | ToolCachePolicy::SharedReadOnly { ttl_ms, .. } => {
            Some(Duration::from_millis(ttl_ms))
        }
        ToolCachePolicy::PerTurn => Some(Duration::from_secs(300)),
        ToolCachePolicy::Never => None,
    }
}

/// The executor enforces this read-only gate. Only L0 tools that declare a
/// policy are ever served from cache — a mutating tool cannot opt in.
pub fn is_cacheable(policy: ToolCachePolicy, permission: PermissionLevel) -> bool {
    policy != ToolCachePolicy::Never && permission == PermissionLevel::L0
}

/// Bounded FIFO-with-TTL cache of read-only tool results.
#[derive(Debug)]
pub struct ReadOnlyToolResultCache {
    inner: std::sync::Mutex<Inner>,
    capacity: usize,
}

#[derive(Debug, Default)]
struct Inner {
    entries: HashMap<String, Entry>,
    order: VecDeque<String>,
    metrics: BTreeMap<String, ToolCacheCounters>,
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCacheCounters {
    pub hit_total: u64,
    pub miss_total: u64,
}

pub type ToolCacheMetricsSnapshot = BTreeMap<String, ToolCacheCounters>;

#[derive(Debug, Clone)]
struct Entry {
    stored_at: Instant,
    ttl: Duration,
    value: fabric::CapabilityResult,
}

impl ReadOnlyToolResultCache {
    pub fn new(capacity: usize) -> Self {
        Self {
            inner: std::sync::Mutex::new(Inner::default()),
            capacity: capacity.max(1),
        }
    }

    /// Look up a cached result, expiring stale entries and refreshing recency
    /// on a hit. Returns `None` on miss/expiry or lock failure (fail-open).
    pub fn get(&self, tool_name: &str, key: &str) -> Option<fabric::CapabilityResult> {
        let now = Instant::now();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let expired = match inner.entries.get(key) {
            Some(entry) => now.duration_since(entry.stored_at) > entry.ttl,
            None => true,
        };
        if expired {
            inner.entries.remove(key);
            inner.order.retain(|candidate| candidate != key);
            let metric = inner.metrics.entry(tool_name.to_owned()).or_default();
            metric.miss_total = metric.miss_total.saturating_add(1);
            return None;
        }
        let value = inner.entries.get(key).map(|entry| entry.value.clone());
        inner.order.retain(|candidate| candidate != key);
        inner.order.push_back(key.to_owned());
        value
    }

    /// Record a candidate value that passed the current policy/audit pipeline
    /// and was actually served to the caller.
    pub fn record_hit(&self, tool_name: &str) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let metric = inner.metrics.entry(tool_name.to_owned()).or_default();
        metric.hit_total = metric.hit_total.saturating_add(1);
    }

    /// Record a cached candidate rejected by current policy/audit. The caller
    /// executes the authoritative tool, so this is an effective cache miss.
    pub fn record_rejected_candidate(&self, tool_name: &str) {
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        let metric = inner.metrics.entry(tool_name.to_owned()).or_default();
        metric.miss_total = metric.miss_total.saturating_add(1);
    }

    pub fn metrics_snapshot(&self) -> ToolCacheMetricsSnapshot {
        self.inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .metrics
            .clone()
    }

    /// Store a result under a key with the policy TTL.
    pub fn insert(&self, key: String, value: fabric::CapabilityResult, ttl: Duration) {
        let now = Instant::now();
        let mut inner = self
            .inner
            .lock()
            .unwrap_or_else(|poison| poison.into_inner());
        if inner.entries.contains_key(&key) {
            inner.order.retain(|candidate| candidate != &key);
        } else {
            while inner.order.len() >= self.capacity {
                if let Some(oldest) = inner.order.pop_front() {
                    inner.entries.remove(&oldest);
                }
            }
        }
        inner.entries.insert(
            key.clone(),
            Entry {
                stored_at: now,
                ttl,
                value,
            },
        );
        inner.order.push_back(key);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::tool::PermissionLevel::{L0, L1};

    fn result(payload: &str) -> fabric::CapabilityResult {
        fabric::CapabilityResult {
            call_id: "call-1".into(),
            output: payload.into(),
            is_error: false,
            usage: Default::default(),
            audit_id: None,
            patch_delta: None,
            served_from_cache: false,
        }
    }

    fn key(
        args: serde_json::Value,
        workspace: &str,
        principal: &str,
        policy: ToolCachePolicy,
    ) -> String {
        read_only_cache_key(
            "read_file",
            "v1",
            &args,
            ToolCacheScope {
                workspace,
                principal,
                session: "session-a",
                turn: "turn-a",
            },
            policy,
        )
    }

    #[test]
    fn cacheable_gate_requires_l0_and_declared_policy() {
        assert!(is_cacheable(ToolCachePolicy::Session { ttl_ms: 1000 }, L0));
        assert!(!is_cacheable(ToolCachePolicy::Session { ttl_ms: 1000 }, L1));
        assert!(!is_cacheable(ToolCachePolicy::Never, L0));
        assert!(!is_cacheable(
            ToolCachePolicy::SharedReadOnly {
                ttl_ms: 1000,
                vary_by_principal: true,
            },
            L1
        ));
    }

    #[test]
    fn key_changes_on_workspace_principal_impl_version_and_policy() {
        let args = serde_json::json!({"path": "/tmp/a"});
        let base = key(
            args.clone(),
            "ws-a",
            "principal-a",
            ToolCachePolicy::Session { ttl_ms: 1000 },
        );
        assert_ne!(
            base,
            key(
                args.clone(),
                "ws-a",
                "principal-b",
                ToolCachePolicy::Session { ttl_ms: 1000 }
            )
        );
        assert_ne!(
            base,
            key(
                args.clone(),
                "ws-b",
                "principal-a",
                ToolCachePolicy::Session { ttl_ms: 1000 }
            )
        );
        assert_ne!(
            base,
            key(
                serde_json::json!({"path": "/tmp/b"}),
                "ws-a",
                "principal-a",
                ToolCachePolicy::Session { ttl_ms: 1000 }
            )
        );
        assert_ne!(
            base,
            key(
                args,
                "ws-a",
                "principal-a",
                ToolCachePolicy::Session { ttl_ms: 999 }
            )
        );
    }

    #[test]
    fn canonical_arguments_and_policy_scope_are_exact() {
        let left = serde_json::from_str::<Value>(r#"{"b":2,"a":{"y":1,"x":0}}"#).unwrap();
        let right = serde_json::from_str::<Value>(r#"{"a":{"x":0,"y":1},"b":2}"#).unwrap();
        assert_eq!(
            key(left, "ws-a", "principal-a", ToolCachePolicy::PerTurn),
            key(right, "ws-a", "principal-a", ToolCachePolicy::PerTurn)
        );

        let per_turn_a = read_only_cache_key(
            "read_file",
            "v1",
            &serde_json::json!({}),
            ToolCacheScope {
                workspace: "ws-a",
                principal: "principal-a",
                session: "session-a",
                turn: "turn-a",
            },
            ToolCachePolicy::PerTurn,
        );
        let per_turn_b = read_only_cache_key(
            "read_file",
            "v1",
            &serde_json::json!({}),
            ToolCacheScope {
                workspace: "ws-a",
                principal: "principal-a",
                session: "session-a",
                turn: "turn-b",
            },
            ToolCachePolicy::PerTurn,
        );
        assert_ne!(per_turn_a, per_turn_b);

        let shared_a = read_only_cache_key(
            "read_file",
            "v1",
            &serde_json::json!({}),
            ToolCacheScope {
                workspace: "ws-a",
                principal: "principal-a",
                session: "session-a",
                turn: "turn-a",
            },
            ToolCachePolicy::SharedReadOnly {
                ttl_ms: 1000,
                vary_by_principal: false,
            },
        );
        let shared_b = read_only_cache_key(
            "read_file",
            "v1",
            &serde_json::json!({}),
            ToolCacheScope {
                workspace: "ws-a",
                principal: "principal-b",
                session: "session-b",
                turn: "turn-b",
            },
            ToolCachePolicy::SharedReadOnly {
                ttl_ms: 1000,
                vary_by_principal: false,
            },
        );
        assert_eq!(shared_a, shared_b);
    }

    #[test]
    fn hit_returns_cached_and_expiry_misses() {
        let cache = ReadOnlyToolResultCache::new(4);
        let k = key(
            serde_json::json!({"path": "/tmp/a"}),
            "ws-a",
            "principal-a",
            ToolCachePolicy::Session { ttl_ms: 60000 },
        );
        assert!(cache.get("read_file", &k).is_none());
        cache.insert(k.clone(), result("cached-content"), Duration::from_secs(60));
        let hit = cache.get("read_file", &k).expect("cached hit");
        cache.record_hit("read_file");
        assert_eq!(hit.output, "cached-content");
        // A different key (implementation version bump) misses.
        let v2 = read_only_cache_key(
            "read_file",
            "v2",
            &serde_json::json!({"path": "/tmp/a"}),
            ToolCacheScope {
                workspace: "ws-a",
                principal: "principal-a",
                session: "session-a",
                turn: "turn-a",
            },
            ToolCachePolicy::Session { ttl_ms: 60000 },
        );
        assert!(cache.get("read_file", &v2).is_none());
        let expired = key(
            serde_json::json!({"path": "/tmp/expired"}),
            "ws-a",
            "principal-a",
            ToolCachePolicy::Session { ttl_ms: 0 },
        );
        cache.insert(expired.clone(), result("expired-content"), Duration::ZERO);
        std::thread::sleep(Duration::from_millis(1));
        assert!(cache.get("read_file", &expired).is_none());
        let metrics = cache.metrics_snapshot();
        assert_eq!(metrics["read_file"].hit_total, 1);
        assert_eq!(metrics["read_file"].miss_total, 3);
    }

    #[test]
    fn bounded_capacity_evicts_oldest() {
        let cache = ReadOnlyToolResultCache::new(2);
        let mut keys = Vec::new();
        for i in 0..3 {
            let k = key(
                serde_json::json!({"path": format!("/tmp/{i}")}),
                "ws-a",
                "principal-a",
                ToolCachePolicy::Session { ttl_ms: 60000 },
            );
            cache.insert(k.clone(), result(&format!("v{i}")), Duration::from_secs(60));
            keys.push(k);
        }
        // Oldest (v0) evicted; v1 and v2 present.
        assert!(cache.get("read_file", &keys[0]).is_none());
        assert!(cache.get("read_file", &keys[1]).is_some());
        assert!(cache.get("read_file", &keys[2]).is_some());
    }
}
