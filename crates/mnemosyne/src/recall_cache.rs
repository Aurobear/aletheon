//! Bounded, generation-keyed recall cache (see
//! docs/testing/deepseek-cache.md#generation-keyed-recall-cache).
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
use tokio::sync::{watch, Mutex};

use crate::service::{
    ForgetPolicy, ForgetReceipt, MemoryScope, MemoryService, RecallRequest, RecallSet,
};
use crate::MemoryMetrics;
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

#[derive(Debug, Clone)]
enum InFlightOutcome {
    Pending,
    Ready(RecallSet),
    Failed,
}

struct InFlight {
    outcome: watch::Sender<InFlightOutcome>,
}

/// Cancellation guard for the single-flight leader. A task can be aborted at
/// any await point after publishing its in-flight slot; without this guard the
/// sender retained by the map would keep every waiter blocked forever.
struct InFlightLeaderGuard {
    key: Option<RecallCacheKey>,
    inflight: Arc<Mutex<HashMap<RecallCacheKey, InFlight>>>,
}

impl InFlightLeaderGuard {
    fn new(key: RecallCacheKey, inflight: Arc<Mutex<HashMap<RecallCacheKey, InFlight>>>) -> Self {
        Self {
            key: Some(key),
            inflight,
        }
    }

    fn disarm(&mut self) {
        self.key = None;
    }
}

impl Drop for InFlightLeaderGuard {
    fn drop(&mut self) {
        let Some(key) = self.key.take() else {
            return;
        };
        if let Ok(mut inflight) = self.inflight.try_lock() {
            if let Some(entry) = inflight.remove(&key) {
                entry.outcome.send_replace(InFlightOutcome::Failed);
            }
            return;
        }
        let inflight = self.inflight.clone();
        if let Ok(runtime) = tokio::runtime::Handle::try_current() {
            runtime.spawn(async move {
                if let Some(entry) = inflight.lock().await.remove(&key) {
                    entry.outcome.send_replace(InFlightOutcome::Failed);
                }
            });
        }
    }
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
    metrics: MemoryMetrics,
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
            metrics: MemoryMetrics::default(),
        }
    }

    /// Attach the existing Mnemosyne metrics authority so cache counters appear
    /// in the same snapshot as the rest of the recall pipeline.
    pub fn with_metrics(mut self, metrics: MemoryMetrics) -> Self {
        self.metrics = metrics;
        self
    }

    fn build_key(
        &self,
        req: &RecallRequest,
        prefilter: Option<&RecallPreFilter>,
    ) -> RecallCacheKey {
        let query_digest = sha256_hex(req.query.as_bytes());
        let scope_identity = prefilter.map_or_else(
            || serde_json::json!({ "session_id": req.session }),
            |p| {
                serde_json::json!({
                    "principal_id": p.ancestry.principal_id,
                    "workspace_id": p.ancestry.workspace_id,
                    "session_id": p.ancestry.session_id,
                    "goal_id": p.ancestry.goal_id,
                    "agent_id": p.ancestry.agent_id,
                    "task_id": p.ancestry.task_id,
                })
            },
        );
        let mut allowed_authorities = prefilter
            .map(|p| p.allowed_authorities.clone())
            .unwrap_or_default();
        allowed_authorities.sort();
        allowed_authorities.dedup();
        let filter_identity = serde_json::json!({
            "session": req.session,
            "max_items": req.max_items,
            "max_content_bytes": req.max_content_bytes,
            "current_at": req.current_at,
            "include_historical": req.include_historical,
            "mode": req.mode,
            "max_sensitivity": prefilter.map(|p| p.max_sensitivity),
            "allowed_authorities": allowed_authorities,
        });
        let scope = sha256_hex(
            &serde_json::to_vec(&scope_identity).expect("recall scope identity serializes"),
        );
        RecallCacheKey {
            principal_scope_digest: scope,
            query_digest,
            filters_digest: sha256_hex(
                &serde_json::to_vec(&filter_identity).expect("recall filter identity serializes"),
            ),
            top_k: u32::try_from(req.max_items).unwrap_or(u32::MAX),
            embedding_model_id: self.embedding_model_id.clone().unwrap_or_default(),
            memory_policy_version: self.policy_version.clone(),
            memory_generation: self.generation.load(Ordering::Acquire),
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

        // Fast path: cache hit. The bounded in-memory mutex has no external
        // failure mode; waiting here avoids turning ordinary contention into a
        // duplicate authoritative recall.
        if let Some(set) = self.cache.lock().await.get(&key, Instant::now()) {
            self.metrics.recall_cache_hit();
            return Ok(set);
        }
        self.metrics.recall_cache_miss();

        // Single-flight: one in-flight recall per key; waiters share the result.
        let waiter = {
            let mut inflight = self.inflight.lock().await;
            match inflight.get(&key) {
                Some(entry) => Some(entry.outcome.subscribe()),
                None => {
                    let (outcome, _) = watch::channel(InFlightOutcome::Pending);
                    inflight.insert(key.clone(), InFlight { outcome });
                    None
                }
            }
        };

        if let Some(mut outcome) = waiter {
            self.metrics.recall_cache_singleflight_wait();
            loop {
                let current = { outcome.borrow_and_update().clone() };
                match current {
                    InFlightOutcome::Ready(set) => return Ok(set),
                    InFlightOutcome::Failed => {
                        // The leader failed. Fail open by performing a fresh
                        // authoritative recall rather than caching an error.
                        return self.recall_inner(req, prefilter).await;
                    }
                    InFlightOutcome::Pending => {}
                }
                if outcome.changed().await.is_err() {
                    // A dropped leader must never strand a waiter.
                    return self.recall_inner(req, prefilter).await;
                }
            }
        }

        let mut leader_guard = InFlightLeaderGuard::new(key.clone(), self.inflight.clone());

        // A prior leader can complete between the first cache lookup and our
        // in-flight insertion. Recheck after becoming leader to avoid a second
        // backend request in that race.
        if let Some(set) = self.cache.lock().await.get(&key, Instant::now()) {
            let released = self.inflight.lock().await.remove(&key);
            if let Some(entry) = released {
                entry
                    .outcome
                    .send_replace(InFlightOutcome::Ready(set.clone()));
            }
            leader_guard.disarm();
            return Ok(set);
        }

        // Leader path.
        let result = self.recall_inner(req, prefilter).await;
        if let Ok(set) = &result {
            // Do not publish a result under an obsolete generation if a memory
            // write committed while the authoritative recall was running.
            if self.generation.load(Ordering::Acquire) == key.memory_generation {
                self.cache
                    .lock()
                    .await
                    .insert(key.clone(), set.clone(), Instant::now());
            }
        } else {
            self.metrics.recall_cache_load_error();
        }
        // Always release the in-flight slot and wake waiters (success or
        // failure) so a failed leader does not strand them.
        let released = {
            let mut inflight = self.inflight.lock().await;
            inflight.remove(&key)
        };
        if let Some(entry) = released {
            entry.outcome.send_replace(match &result {
                Ok(set) => InFlightOutcome::Ready(set.clone()),
                Err(_) => InFlightOutcome::Failed,
            });
        }
        leader_guard.disarm();
        result
    }
}

#[async_trait]
impl MemoryService for CachingMemoryService {
    async fn record(&self, event: ExperienceEvent) -> anyhow::Result<()> {
        self.inner.record(event).await?;
        self.generation.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    async fn record_canonical(&self, record: MemoryRecord) -> anyhow::Result<()> {
        self.inner.record_canonical(record).await?;
        self.generation.fetch_add(1, Ordering::AcqRel);
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
        self.generation.fetch_add(1, Ordering::AcqRel);
        Ok(())
    }

    async fn preview_forget(&self, policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        self.inner.preview_forget(policy).await
    }

    async fn forget(&self, policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        let receipt = self.inner.forget(policy).await?;
        self.generation.fetch_add(1, Ordering::AcqRel);
        Ok(receipt)
    }

    async fn synthesize(&self, request: SynthesisRequest) -> anyhow::Result<SynthesisResult> {
        self.inner.synthesize(request).await
    }

    async fn promote_facts(&self, min_confidence: f64, max_count: usize) -> anyhow::Result<usize> {
        let promoted = self.inner.promote_facts(min_confidence, max_count).await?;
        if promoted > 0 {
            self.generation.fetch_add(1, Ordering::AcqRel);
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
        delay_ms: AtomicU64,
        fail_next: std::sync::atomic::AtomicBool,
    }

    #[async_trait]
    impl MemoryService for StubMemory {
        async fn recall(&self, _req: RecallRequest) -> anyhow::Result<RecallSet> {
            self.recalls.fetch_add(1, Ordering::Relaxed);
            let delay_ms = self.delay_ms.load(Ordering::Relaxed);
            if delay_ms > 0 {
                tokio::time::sleep(Duration::from_millis(delay_ms)).await;
            }
            if self.fail_next.swap(false, Ordering::Relaxed) {
                anyhow::bail!("injected authoritative recall failure");
            }
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
    fn key_isolates_authorization_and_temporal_filters() {
        let (svc, _) = wrapper();
        let req = request("q", "s1");
        let public = prefilter("s1");
        let mut confidential = public.clone();
        confidential.max_sensitivity = MemorySensitivity::Confidential;
        assert_ne!(
            svc.build_key(&req, Some(&public)),
            svc.build_key(&req, Some(&confidential)),
            "a broader sensitivity filter must not reuse a narrower cache entry"
        );

        let mut authorities_a = public.clone();
        authorities_a.allowed_authorities = vec![
            crate::MemoryAuthority::RawExperience,
            crate::MemoryAuthority::ApprovedCore,
        ];
        let mut authorities_b = public.clone();
        authorities_b.allowed_authorities = vec![
            crate::MemoryAuthority::ApprovedCore,
            crate::MemoryAuthority::RawExperience,
        ];
        assert_eq!(
            svc.build_key(&req, Some(&authorities_a)),
            svc.build_key(&req, Some(&authorities_b)),
            "authority ordering is not semantically meaningful"
        );

        let mut later = req.clone();
        later.current_at = Some(chrono::Utc::now());
        assert_ne!(
            svc.build_key(&req, Some(&public)),
            svc.build_key(&later, Some(&public)),
            "temporal recall boundaries must participate in cache identity"
        );
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
        let metrics = svc.metrics.snapshot();
        assert_eq!(metrics.memory_recall_cache_hit_total, 1);
        assert_eq!(metrics.memory_recall_cache_miss_total, 1);

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

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn concurrent_misses_are_single_flight_without_lost_wakeup() {
        let (svc, inner) = wrapper();
        inner.delay_ms.store(40, Ordering::Relaxed);
        let svc = Arc::new(svc);
        let req = request("same", "session-a");

        let mut calls = tokio::task::JoinSet::new();
        for _ in 0..8 {
            let svc = svc.clone();
            let req = req.clone();
            calls.spawn(async move { svc.recall(req).await });
        }
        tokio::time::timeout(Duration::from_secs(2), async {
            while let Some(call) = calls.join_next().await {
                call.expect("recall task panicked").expect("recall failed");
            }
        })
        .await
        .expect("single-flight waiters must not hang");
        assert_eq!(inner.recalls.load(Ordering::Relaxed), 1);
        let metrics = svc.metrics.snapshot();
        assert_eq!(metrics.memory_recall_cache_miss_total, 8);
        assert_eq!(metrics.memory_recall_cache_singleflight_wait_total, 7);
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn failed_leader_releases_waiters_and_does_not_cache_error() {
        let (svc, inner) = wrapper();
        inner.delay_ms.store(30, Ordering::Relaxed);
        inner.fail_next.store(true, Ordering::Relaxed);
        let svc = Arc::new(svc);
        let req = request("same", "session-a");

        let leader = {
            let svc = svc.clone();
            let req = req.clone();
            tokio::spawn(async move { svc.recall(req).await })
        };
        tokio::time::sleep(Duration::from_millis(5)).await;
        let waiter = {
            let svc = svc.clone();
            let req = req.clone();
            tokio::spawn(async move { svc.recall(req).await })
        };
        let (leader, waiter) = tokio::time::timeout(Duration::from_secs(2), async {
            (leader.await.unwrap(), waiter.await.unwrap())
        })
        .await
        .expect("failed leader must not strand waiters");
        assert!(leader.is_err());
        waiter.expect("waiter must fail open to authoritative recall");
        assert_eq!(inner.recalls.load(Ordering::Relaxed), 2);
        assert_eq!(
            svc.metrics.snapshot().memory_recall_cache_load_error_total,
            1
        );
    }

    #[tokio::test(flavor = "multi_thread", worker_threads = 2)]
    async fn cancelled_leader_releases_waiters() {
        let (svc, inner) = wrapper();
        inner.delay_ms.store(100, Ordering::Relaxed);
        let svc = Arc::new(svc);
        let req = request("same", "session-a");

        let leader = {
            let svc = svc.clone();
            let req = req.clone();
            tokio::spawn(async move { svc.recall(req).await })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        let waiter = {
            let svc = svc.clone();
            let req = req.clone();
            tokio::spawn(async move { svc.recall(req).await })
        };
        tokio::time::sleep(Duration::from_millis(10)).await;
        leader.abort();

        tokio::time::timeout(Duration::from_secs(2), waiter)
            .await
            .expect("cancelled leader must not strand waiters")
            .expect("waiter task panicked")
            .expect("waiter must fail open to authoritative recall");
        assert_eq!(inner.recalls.load(Ordering::Relaxed), 2);
    }
}
