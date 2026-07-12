
//! In-memory resource lease manager — Phase 5A.
//!
//! Manages exclusive/shared access to named resources (e.g. "gpu",
//! "sandbox-instance", "database"). Leases are time-limited; they must be
//! released explicitly or they expire after their duration.
//!
//! # Concurrency model
//!
//! Resources are exclusive by default: only one lease per resource at a time.
//! If a resource is already leased, new requests are denied with
//! `AdmissionError::LeaseUnavailable`.

use fabric::{AdmissionError, LeaseRequest, ResourceLeaseId};
use std::collections::HashMap;
use tokio::sync::Mutex;

// ---------------------------------------------------------------------------
// Lease entry
// ---------------------------------------------------------------------------

/// An active lease on a resource.
#[derive(Debug, Clone)]
struct LeaseEntry {
    id: ResourceLeaseId,
    resource: String,
    principal: String,
    duration_ms: u64,
    /// Monotonic timestamp when the lease was acquired.
    acquired_at: u64,
}

// ---------------------------------------------------------------------------
// InMemoryResourceLeaseManager
// ---------------------------------------------------------------------------

/// In-memory resource lease manager.
///
/// Tracks which resources are currently leased and enforces exclusive
/// access with time-limited leases.
pub struct InMemoryResourceLeaseManager {
    /// Resource name → active lease.
    leases: Mutex<HashMap<String, LeaseEntry>>,
    /// Lease id → resource name (for quick release lookup).
    by_id: Mutex<HashMap<ResourceLeaseId, String>>,
}

impl std::fmt::Debug for InMemoryResourceLeaseManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("InMemoryResourceLeaseManager")
            .finish_non_exhaustive()
    }
}

impl InMemoryResourceLeaseManager {
    /// Create an empty lease manager.
    pub fn new() -> Self {
        Self {
            leases: Mutex::new(HashMap::new()),
            by_id: Mutex::new(HashMap::new()),
        }
    }

    /// Attempt to acquire a lease on the given resource.
    ///
    /// Returns `AdmissionError::LeaseUnavailable` if the resource is already
    /// leased and the existing lease hasn't expired.
    pub async fn acquire(
        &self,
        principal: &str,
        request: &LeaseRequest,
        now_mono_ms: u64,
    ) -> Result<ResourceLeaseId, AdmissionError> {
        let mut leases = self.leases.lock().await;

        // Check if resource is already held and not expired.
        if let Some(existing) = leases.get(&request.resource) {
            let elapsed = now_mono_ms.saturating_sub(existing.acquired_at);
            if elapsed < existing.duration_ms {
                return Err(AdmissionError::LeaseUnavailable);
            }
            // Lease expired — remove it.
            let mut by_id = self.by_id.lock().await;
            by_id.remove(&existing.id);
        }

        let id = ResourceLeaseId::new();
        let entry = LeaseEntry {
            id,
            resource: request.resource.clone(),
            principal: principal.to_string(),
            duration_ms: request.duration_ms,
            acquired_at: now_mono_ms,
        };

        leases.insert(request.resource.clone(), entry);
        let mut by_id = self.by_id.lock().await;
        by_id.insert(id, request.resource.clone());

        Ok(id)
    }

    /// Release a lease by id.
    ///
    /// This is the normal cleanup path after capability execution completes.
    pub async fn release(&self, lease_id: ResourceLeaseId) {
        let resource = {
            let mut by_id = self.by_id.lock().await;
            by_id.remove(&lease_id)
        };

        if let Some(resource) = resource {
            let mut leases = self.leases.lock().await;
            leases.remove(&resource);
        }
    }

    /// Check whether a resource is currently leased (and not expired).
    pub async fn is_leased(&self, resource: &str, now_mono_ms: u64) -> bool {
        let leases = self.leases.lock().await;
        match leases.get(resource) {
            Some(entry) => {
                let elapsed = now_mono_ms.saturating_sub(entry.acquired_at);
                elapsed < entry.duration_ms
            }
            None => false,
        }
    }

    /// Return the number of active (non-expired) leases.
    pub async fn active_count(&self, now_mono_ms: u64) -> usize {
        let leases = self.leases.lock().await;
        leases
            .values()
            .filter(|e| {
                let elapsed = now_mono_ms.saturating_sub(e.acquired_at);
                elapsed < e.duration_ms
            })
            .count()
    }
}

impl Default for InMemoryResourceLeaseManager {
    fn default() -> Self {
        Self::new()
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn acquire_free_resource_succeeds() {
        let mgr = InMemoryResourceLeaseManager::new();
        let req = LeaseRequest {
            resource: "gpu-0".into(),
            duration_ms: 30_000,
        };
        let id = mgr.acquire("agent-1", &req, 0).await.unwrap();
        assert!(id.0 != uuid::Uuid::nil());
        assert!(mgr.is_leased("gpu-0", 0).await);
    }

    #[tokio::test]
    async fn acquire_already_leased_resource_fails() {
        let mgr = InMemoryResourceLeaseManager::new();
        let req = LeaseRequest {
            resource: "gpu-0".into(),
            duration_ms: 30_000,
        };
        mgr.acquire("agent-1", &req, 0).await.unwrap();

        let err = mgr.acquire("agent-2", &req, 0).await.unwrap_err();
        assert!(matches!(err, AdmissionError::LeaseUnavailable));
    }

    #[tokio::test]
    async fn release_then_reacquire_succeeds() {
        let mgr = InMemoryResourceLeaseManager::new();
        let req = LeaseRequest {
            resource: "gpu-0".into(),
            duration_ms: 30_000,
        };
        let id = mgr.acquire("agent-1", &req, 0).await.unwrap();
        mgr.release(id).await;

        // Second agent can now acquire.
        let id2 = mgr.acquire("agent-2", &req, 0).await.unwrap();
        assert!(id2.0 != id.0);
    }

    #[tokio::test]
    async fn expired_lease_allows_reacquire() {
        let mgr = InMemoryResourceLeaseManager::new();
        let req = LeaseRequest {
            resource: "gpu-0".into(),
            duration_ms: 1_000,
        };
        mgr.acquire("agent-1", &req, 0).await.unwrap();

        // After 2_000ms, the lease should be expired.
        assert!(!mgr.is_leased("gpu-0", 2_000).await);

        // New agent can acquire.
        let id2 = mgr.acquire("agent-2", &req, 2_000).await.unwrap();
        assert!(id2.0 != uuid::Uuid::nil());
    }

    #[tokio::test]
    async fn active_count_tracks_only_non_expired() {
        let mgr = InMemoryResourceLeaseManager::new();
        mgr.acquire(
            "agent-1",
            &LeaseRequest {
                resource: "r1".into(),
                duration_ms: 5_000,
            },
            0,
        )
        .await
        .unwrap();
        mgr.acquire(
            "agent-1",
            &LeaseRequest {
                resource: "r2".into(),
                duration_ms: 1_000,
            },
            0,
        )
        .await
        .unwrap();

        assert_eq!(mgr.active_count(500).await, 2);

        // r2 expired at 1_000ms
        assert_eq!(mgr.active_count(2_000).await, 1);

        // Both expired
        assert_eq!(mgr.active_count(10_000).await, 0);
    }
}
