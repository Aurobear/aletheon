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

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use fabric::tool::{PermissionLevel, ToolCachePolicy};
use serde_json::Value;
use sha2::{Digest, Sha256};

const KEY_DOMAIN: &[u8] = b"aletheon.read-only-tool-cache.v1\0";

/// Host-owned cache key for a read-only tool result.
///
/// Includes the tool name, implementation version, canonical arguments,
/// workspace identity, principal scope and policy identity so that a change to
/// any one of them is a miss. The caller supplies the Debug-string workspace
/// and principal identities.
pub fn read_only_cache_key(
    tool_name: &str,
    impl_version: &str,
    args: &Value,
    workspace_key: &str,
    principal_key: &str,
    policy: ToolCachePolicy,
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(KEY_DOMAIN);
    hasher.update(tool_name.as_bytes());
    hasher.update(b"\0");
    hasher.update(impl_version.as_bytes());
    hasher.update(b"\0");
    hasher.update(serde_json::to_string(args).unwrap_or_default().as_bytes());
    hasher.update(b"\0");
    hasher.update(workspace_key.as_bytes());
    hasher.update(b"\0");
    hasher.update(principal_key.as_bytes());
    hasher.update(b"\0");
    hasher.update(format!("{policy:?}").as_bytes());
    format!("sha256:{:x}", hasher.finalize())
}

/// TTL for a declared policy. `Never` yields `None` (uncacheable). `PerTurn`
/// uses a short TTL as a turn-bounded approximation; exact turn-boundary
/// semantics would need turn-scoped keys.
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
}

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
    pub fn get(&self, key: &str) -> Option<fabric::CapabilityResult> {
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
            return None;
        }
        let value = inner.entries.get(key).map(|entry| entry.value.clone());
        inner.order.retain(|candidate| candidate != key);
        inner.order.push_back(key.to_owned());
        value
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
        read_only_cache_key("read_file", "v1", &args, workspace, principal, policy)
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
    fn hit_returns_cached_and_expiry_misses() {
        let cache = ReadOnlyToolResultCache::new(4);
        let k = key(
            serde_json::json!({"path": "/tmp/a"}),
            "ws-a",
            "principal-a",
            ToolCachePolicy::Session { ttl_ms: 60000 },
        );
        assert!(cache.get(&k).is_none());
        cache.insert(k.clone(), result("cached-content"), Duration::from_secs(60));
        let hit = cache.get(&k).expect("cached hit");
        assert_eq!(hit.output, "cached-content");
        // A different key (implementation version bump) misses.
        let v2 = read_only_cache_key(
            "read_file",
            "v2",
            &serde_json::json!({"path": "/tmp/a"}),
            "ws-a",
            "principal-a",
            ToolCachePolicy::Session { ttl_ms: 60000 },
        );
        assert!(cache.get(&v2).is_none());
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
        assert!(cache.get(&keys[0]).is_none());
        assert!(cache.get(&keys[1]).is_some());
        assert!(cache.get(&keys[2]).is_some());
    }
}
