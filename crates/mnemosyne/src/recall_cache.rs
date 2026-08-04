//! Bounded, generation-keyed recall cache (Phase C6 of
//! docs/plans/deepseek-cache-and-message-optimization-plan.md).
//!
//! Wraps an authoritative `MemoryService` and caches
//! `recall` / `recall_with_prefilter` results under a key that includes the
//! principal scope, query, request filters, embedding model, memory-policy
//! version and a write-generation. Any successful memory write bumps the
//! generation, so a stale result is never served after a write (stale keys
//! simply never match again — no per-entry eviction scan). Concurrent misses
//! share one in-flight recall (single-flight). Cache and single-flight
//! failures always fall through to the authoritative recall (fail-open); the
//! cache never makes a permission or authority decision.

use std::collections::{HashMap, VecDeque};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::time::{Duration, Instant};

use async_trait::async_trait;
use sha2::{Digest, Sha256};
use tokio::sync::{Mutex, Notify};

use crate::service::{
    ForgetPolicy, ForgetReceipt, MemoryScope, MemoryService, RecallRequest, RecallSet,
};
use crate::{ExperienceEvent, MemoryRecord, RecallPreFilter, SynthesisRequest, SynthesisResult};

/// Versioned identity of a cached recall result.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct RecallCacheKey {
    /// Digest of the principal/workspace/session scope (from `RecallPreFilter`
    /// ancestry, or the session id when recall is un-prefiltered).
    pub principal_scope_digest: String,
    /// Digest of the query.
    pub query_digest: String,
    /// Digest of request filters (max items/bytes, historical, mode).
    pub filters_digest: String,
    /// Effective fetch cap (max items) — the top_k at this boundary.
    pub top_k: u32,
    /// Embedding model identity (empty when the backend is not vector-backed).
    pub embedding_model_id: String,
    /// Memory policy version (host-owned; never a model claim).
    pub memory_policy_version: String,
    /// Write generation: bumped on every successful memory write.
    pub memory_generation: u64,
}

#[derive(Debug, Clone)]
struct RecallCacheEntry {
    stored_at: Instant,
    value: RecallSet,
}

/// Bounded FIFO-with-TTL cache. The cap is enforced on insert and a hit
/// refreshes recency, keeping it small and deterministic.
#[derive(Debug)]
struct RecallCache {
    entries: HashMap<RecallCacheKey, RecallCacheEntry>,
    order: VecDeque<RecallCacheKey>,
    capacity: usize,
    ttl: Duration,
}

impl RecallCache {
    fn new(capacity: usize, ttl: Duration) -> Self {
        Self {
            entries: HashMap::new(),
            order: VecDeque::new(),
            capacity: capacity.max(1),
            ttl,
        }
    }

    fn get(&mut self, key: &RecallCacheKey, now: Instant) -> Option<RecallSet> {
        let expired = match self.entries.get(key) {
            Some(entry) => now.duration_since(entry.stored_at) > self.ttl,
            None => true,
        };
        if expired {
            self.entries.remove(key);
            self.order.retain(|candidate| candidate != key);
            return None;
        }
        let value = self.entries.get(key).map(|entry| entry.value.clone());
        // Refresh recency (approximate LRU via remove + push).
        self.order.retain(|candidate| candidate != key);
        self.order.push_back(key.clone());
        value
    }

    fn insert(&mut self, key: RecallCacheKey, value: RecallSet, now: Instant) {
        if self.entries.contains_key(&key) {
            self.order.retain(|candidate| candidate != &key);
        } else {
            while self.order.len() >= self.capacity {
                if let Some(oldest) = self.order.pop_front() {
                    self.entries.remove(&oldest);
                }
            }
        }
        self.entries.insert(
            key.clone(),
            RecallCacheEntry {
                stored_at: now,
                value,
            },
        );
        self.order.push_back(key);
    }
}

struct InFlight {
    value: Arc<Mutex<Option<RecallSet>>>,
    notify: Arc<Notify>,
}

/// A `MemoryService` that caches recall results and invalidates them on any
/// successful write. Fail-open on every cache path.
pub struct CachingMemoryService {
    inner: Arc<dyn MemoryService>,
    cache: Arc<Mutex<RecallCache>>,
    inflight: Arc<Mutex<HashMap<RecallCacheKey, InFlight>>>,
    generation: Arc<AtomicU64>,
    policy_version: String,
    embedding_model_id: Option<String>,
}

impl CachingMemoryService {
    pub fn new(
        inner: Arc<dyn MemoryService>,
        policy_version: String,
        embedding_model_id: Option<String>,
        capacity: usize,
        ttl: Duration,
    ) -> Self {
        Self {
            inner,
            cache: Arc::new(Mutex::new(RecallCache::new(capacity, ttl))),
            inflight: Arc::new(Mutex::new(HashMap::new())),
            generation: Arc::new(AtomicU64::new(0)),
            policy_version,
            embedding_model_id,
        }
    }

    fn build_key(
        &self,
        req: &RecallRequest,
        prefilter: Option<&RecallPreFilter>,
    ) -> RecallCacheKey {
        let query_digest = sha256_hex(req.query.as_bytes());
        let scope = prefilter
            .map(|p| sha256_hex(format!("{:?}", p.ancestry).as_bytes()))
            .unwrap_or_else(|| sha256_hex(req.session.as_bytes()));
        let filters = format!(
            "items={}|bytes={}|hist={}|mode={:?}",
            req.max_items, req.max_content_bytes, req.include_historical, req.mode
        );
        RecallCacheKey {
            principal_scope_digest: scope,
            query_digest,
            filters_digest: sha256_hex(filters.as_bytes()),
            top_k: u32::try_from(req.max_items).unwrap_or(u32::MAX),
            embedding_model_id: self.embedding_model_id.clone().unwrap_or_default(),
            memory_policy_version: self.policy_version.clone(),
            memory_generation: self.generation.load(Ordering::Relaxed),
        }
    }

    async fn recall_inner(
        &self,
        req: RecallRequest,
        prefilter: Option<&RecallPreFilter>,
    ) -> anyhow::Result<RecallSet> {
        match prefilter {
            Some(p) => self.inner.recall_with_prefilter(req, p).await,
            None => self.inner.recall(req).await,
        }
    }

    async fn cached_recall(
        &self,
        req: RecallRequest,
        prefilter: Option<&RecallPreFilter>,
    ) -> anyhow::Result<RecallSet> {
        let key = self.build_key(&req, prefilter);

        // Fast path: cache hit. `try_lock` fails open under contention.
        if let Ok(mut cache) = self.cache.try_lock() {
            if let Some(set) = cache.get(&key, Instant::now()) {
                return Ok(set);
            }
        }

        // Single-flight: one in-flight recall per key; waiters share the result.
        let (waiter_value, waiter_notify) = {
            let mut inflight = self.inflight.lock().await;
            match inflight.get(&key) {
                Some(entry) => (Some(entry.value.clone()), Some(entry.notify.clone())),
                None => {
                    inflight.insert(
                        key.clone(),
                        InFlight {
                            value: Arc::new(Mutex::new(None)),
                            notify: Arc::new(Notify::new()),
                        },
                    );
                    (None, None)
                }
            }
        };

        if let Some(value) = waiter_value {
            if let Some(notify) = waiter_notify {
                notify.notified().await;
            }
            let guard = value.lock().await;
            if let Some(set) = guard.as_ref() {
                return Ok(set.clone());
            }
            // The leader failed or produced nothing — fall through to our own
            // authoritative recall.
            return self.recall_inner(req, prefilter).await;
        }

        // Leader path.
        let result = self.recall_inner(req, prefilter).await;
        if let Ok(set) = &result {
            let mut cache = self.cache.lock().await;
            cache.insert(key.clone(), set.clone(), Instant::now());
        }
        // Always release the in-flight slot and wake waiters (success or
        // failure) so a failed leader does not strand them.
        let released = {
            let mut inflight = self.inflight.lock().await;
            inflight.remove(&key)
        };
        if let Some(entry) = released {
            *entry.value.lock().await = result.as_ref().ok().cloned();
            entry.notify.notify_waiters();
        }
        result
    }
}

#[async_trait]
impl MemoryService for CachingMemoryService {
    async fn record(&self, event: ExperienceEvent) -> anyhow::Result<()> {
        self.inner.record(event).await?;
        self.generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn record_canonical(&self, record: MemoryRecord) -> anyhow::Result<()> {
        self.inner.record_canonical(record).await?;
        self.generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn recall(&self, req: RecallRequest) -> anyhow::Result<RecallSet> {
        self.cached_recall(req, None).await
    }

    async fn recall_with_prefilter(
        &self,
        req: RecallRequest,
        prefilter: &RecallPreFilter,
    ) -> anyhow::Result<RecallSet> {
        self.cached_recall(req, Some(prefilter)).await
    }

    async fn consolidate(&self, scope: MemoryScope) -> anyhow::Result<()> {
        self.inner.consolidate(scope).await?;
        self.generation.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }

    async fn preview_forget(&self, policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        self.inner.preview_forget(policy).await
    }

    async fn forget(&self, policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        let receipt = self.inner.forget(policy).await?;
        self.generation.fetch_add(1, Ordering::Relaxed);
        Ok(receipt)
    }

    async fn synthesize(&self, request: SynthesisRequest) -> anyhow::Result<SynthesisResult> {
        self.inner.synthesize(request).await
    }

    async fn promote_facts(&self, min_confidence: f64, max_count: usize) -> anyhow::Result<usize> {
        let promoted = self.inner.promote_facts(min_confidence, max_count).await?;
        if promoted > 0 {
            self.generation.fetch_add(1, Ordering::Relaxed);
        }
        Ok(promoted)
    }
}

fn sha256_hex(value: &[u8]) -> String {
    format!("sha256:{:x}", Sha256::digest(value))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::model::ScopeAncestry;
    use crate::MemorySensitivity;

    fn block_on<F: std::future::Future>(future: F) -> F::Output {
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(future)
    }

    /// Minimal in-memory MemoryService used only by the cache wrapper tests.
    #[derive(Debug, Default)]
    struct StubMemory {
        recalls: AtomicU64,
    }

    #[async_trait]
    impl MemoryService for StubMemory {
        async fn recall(&self, _req: RecallRequest) -> anyhow::Result<RecallSet> {
            self.recalls.fetch_add(1, Ordering::Relaxed);
            Ok(RecallSet {
                items: vec![],
                degraded_sources: vec![],
            })
        }
        async fn consolidate(&self, _scope: MemoryScope) -> anyhow::Result<()> {
            Ok(())
        }
        async fn forget(&self, _policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
            Ok(ForgetReceipt::default())
        }
        async fn record(&self, _event: ExperienceEvent) -> anyhow::Result<()> {
            Ok(())
        }
        async fn promote_facts(&self, _min: f64, _max: usize) -> anyhow::Result<usize> {
            Ok(0)
        }
    }

    fn request(query: &str, session: &str) -> RecallRequest {
        RecallRequest {
            session: session.to_owned(),
            query: query.to_owned(),
            max_items: 4,
            max_content_bytes: 4096,
            current_at: None,
            include_historical: false,
            mode: None,
        }
    }

    fn prefilter(session: &str) -> RecallPreFilter {
        RecallPreFilter {
            ancestry: ScopeAncestry {
                principal_id: Some("principal-a".to_owned()),
                workspace_id: Some("workspace-a".to_owned()),
                session_id: Some(session.to_owned()),
                goal_id: None,
                agent_id: None,
                task_id: None,
            },
            max_sensitivity: MemorySensitivity::Public,
            allowed_authorities: vec![],
        }
    }

    fn wrapper() -> (CachingMemoryService, Arc<StubMemory>) {
        let inner = Arc::new(StubMemory::default());
        let svc = CachingMemoryService::new(
            inner.clone(),
            "policy-v1".into(),
            Some("v4".into()),
            64,
            Duration::from_secs(60),
        );
        (svc, inner)
    }

    #[test]
    fn key_differs_across_scope_query_and_generation() {
        let (svc, _) = wrapper();
        let k1 = svc.build_key(&request("q", "s1"), Some(&prefilter("s1")));
        let k2 = svc.build_key(&request("q", "s1"), Some(&prefilter("s2")));
        let k3 = svc.build_key(&request("q2", "s1"), Some(&prefilter("s1")));
        assert_ne!(k1, k2); // different principal/session scope
        assert_ne!(k1, k3); // different query
        svc.generation.fetch_add(1, Ordering::Relaxed);
        let k4 = svc.build_key(&request("q", "s1"), Some(&prefilter("s1")));
        assert_ne!(k1, k4); // different generation
    }

    #[test]
    fn same_key_without_prefilter_uses_session_scope() {
        let (svc, _) = wrapper();
        let a = svc.build_key(&request("q", "s1"), None);
        let b = svc.build_key(&request("q", "s1"), None);
        assert_eq!(a, b);
        let c = svc.build_key(&request("q", "s2"), None);
        assert_ne!(a, c);
    }

    #[test]
    fn recall_is_served_from_cache_then_invalidated_by_write() {
        let (svc, inner) = wrapper();
        let req = request("question", "session-a");

        // First recall is a cache miss and calls the authoritative service.
        block_on(svc.recall(req.clone())).unwrap();
        assert_eq!(inner.recalls.load(Ordering::Relaxed), 1);

        // Identical recall is served from cache (no second authoritative call).
        block_on(svc.recall(req.clone())).unwrap();
        assert_eq!(inner.recalls.load(Ordering::Relaxed), 1);

        // A successful write bumps the generation; the next recall misses again.
        block_on(svc.consolidate(MemoryScope::Session("session-a".into()))).unwrap();
        block_on(svc.recall(req)).unwrap();
        assert_eq!(inner.recalls.load(Ordering::Relaxed), 2);
    }

    #[test]
    fn recall_with_prefilter_is_scope_isolated() {
        let (svc, inner) = wrapper();
        // Same query in two sessions must not share a cached result.
        block_on(svc.recall_with_prefilter(request("q", "s1"), &prefilter("s1"))).unwrap();
        block_on(svc.recall_with_prefilter(request("q", "s2"), &prefilter("s2"))).unwrap();
        assert_eq!(inner.recalls.load(Ordering::Relaxed), 2);
    }
}
