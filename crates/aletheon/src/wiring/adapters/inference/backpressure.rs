//! Shared provider admission, cooldown, and observability.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use ::contracts::memory::{
    ProviderBackpressurePort, ProviderBackpressureSnapshot, ProviderRequestPermit,
};
use async_trait::async_trait;
use tokio::sync::{Mutex as AsyncMutex, OwnedSemaphorePermit, Semaphore};

use cognit::config::ProviderBackpressureConfig;

#[derive(Debug, Clone)]
/// Lightweight policy handle over the process-global provider-state registry.
///
/// Constructing another handle does not create an independent semaphore or
/// cooldown. All handles in the machine-core process resolve the same
/// `provider_key` through `state_for()` and therefore share admission state.
pub struct MachineProviderBackpressure {
    config: ProviderBackpressureConfig,
}

impl MachineProviderBackpressure {
    pub fn new(config: ProviderBackpressureConfig) -> Self {
        Self { config }
    }
}

#[async_trait]
impl ProviderBackpressurePort for MachineProviderBackpressure {
    async fn acquire(&self, provider_key: &str) -> anyhow::Result<Box<dyn ProviderRequestPermit>> {
        Ok(Box::new(
            acquire(&state_for(provider_key, self.config)).await?,
        ))
    }

    async fn observe_retry_after(&self, provider_key: &str, retry_after_ms: Option<u64>) {
        state_for(provider_key, self.config).observe_retry_after(retry_after_ms);
    }
}

#[derive(Debug)]
pub(crate) struct ProviderState {
    config: ProviderBackpressureConfig,
    permits: Arc<Semaphore>,
    cooldown_until: Mutex<Option<Instant>>,
    next_admission_at: AsyncMutex<Instant>,
    admitted: AtomicU64,
    queued: AtomicU64,
    paced: AtomicU64,
    rejected: AtomicU64,
    cooldown_updates: AtomicU64,
}

impl ProviderState {
    fn new(config: ProviderBackpressureConfig) -> Self {
        Self {
            permits: Arc::new(Semaphore::new(config.max_concurrent_requests.max(1))),
            config,
            cooldown_until: Mutex::new(None),
            next_admission_at: AsyncMutex::new(Instant::now()),
            admitted: AtomicU64::new(0),
            queued: AtomicU64::new(0),
            paced: AtomicU64::new(0),
            rejected: AtomicU64::new(0),
            cooldown_updates: AtomicU64::new(0),
        }
    }

    async fn acquire(self: &Arc<Self>) -> anyhow::Result<OwnedSemaphorePermit> {
        let deadline = Instant::now() + Duration::from_millis(self.config.queue_timeout_ms);
        let mut counted_as_queued = false;
        let cooling = self
            .cooldown_until
            .lock()
            .expect("provider cooldown lock poisoned")
            .is_some_and(|until| until > Instant::now());
        if cooling || self.permits.available_permits() == 0 {
            self.queued.fetch_add(1, Ordering::Relaxed);
            counted_as_queued = true;
        }

        let budget = deadline.saturating_duration_since(Instant::now());
        let permit = match tokio::time::timeout(budget, self.permits.clone().acquire_owned()).await
        {
            Ok(result) => result.map_err(|_| anyhow::anyhow!("provider_backpressure_closed")),
            Err(_) => Err(anyhow::anyhow!("provider_backpressure_timeout")),
        };
        let permit = match permit {
            Ok(permit) => permit,
            Err(error) => {
                self.rejected.fetch_add(1, Ordering::Relaxed);
                return Err(error);
            }
        };

        let budget = deadline.saturating_duration_since(Instant::now());
        let mut next_admission_at =
            match tokio::time::timeout(budget, self.next_admission_at.lock()).await {
                Ok(guard) => guard,
                Err(_) => {
                    self.rejected.fetch_add(1, Ordering::Relaxed);
                    return Err(anyhow::anyhow!("provider_backpressure_timeout"));
                }
            };
        let mut counted_as_paced = false;
        loop {
            let now = Instant::now();
            let cooldown_until = self
                .cooldown_until
                .lock()
                .expect("provider cooldown lock poisoned")
                .filter(|until| *until > now);
            let pacing_until = (*next_admission_at > now).then_some(*next_admission_at);
            let ready_at = cooldown_until
                .into_iter()
                .chain(pacing_until)
                .max()
                .unwrap_or(now);

            if ready_at <= now {
                *next_admission_at =
                    now + Duration::from_millis(self.config.min_request_interval_ms);
                self.admitted.fetch_add(1, Ordering::Relaxed);
                return Ok(permit);
            }

            if !counted_as_queued {
                self.queued.fetch_add(1, Ordering::Relaxed);
                counted_as_queued = true;
            }
            if pacing_until.is_some() && !counted_as_paced {
                self.paced.fetch_add(1, Ordering::Relaxed);
                counted_as_paced = true;
            }

            let remaining = ready_at.saturating_duration_since(now);
            let budget = deadline.saturating_duration_since(now);
            if remaining > budget || budget.is_zero() {
                self.rejected.fetch_add(1, Ordering::Relaxed);
                anyhow::bail!("provider_backpressure_timeout");
            }
            tokio::time::sleep(remaining).await;
        }
    }

    fn observe_retry_after(&self, retry_after_ms: Option<u64>) {
        let Some(delay_ms) = retry_after_ms else {
            return;
        };
        let delay_ms = delay_ms.min(self.config.max_cooldown_ms);
        let candidate = Instant::now() + Duration::from_millis(delay_ms);
        let mut until = self
            .cooldown_until
            .lock()
            .expect("provider cooldown lock poisoned");
        if until.is_none_or(|current| candidate > current) {
            *until = Some(candidate);
            self.cooldown_updates.fetch_add(1, Ordering::Relaxed);
        }
    }

    fn snapshot(&self) -> ProviderBackpressureSnapshot {
        let available = self.permits.available_permits();
        ProviderBackpressureSnapshot {
            admitted: self.admitted.load(Ordering::Relaxed),
            queued: self.queued.load(Ordering::Relaxed),
            paced: self.paced.load(Ordering::Relaxed),
            rejected: self.rejected.load(Ordering::Relaxed),
            cooldown_updates: self.cooldown_updates.load(Ordering::Relaxed),
            active: self
                .config
                .max_concurrent_requests
                .max(1)
                .saturating_sub(available),
            available_permits: available,
        }
    }
}

fn registry() -> &'static Mutex<HashMap<String, Arc<ProviderState>>> {
    static REGISTRY: OnceLock<Mutex<HashMap<String, Arc<ProviderState>>>> = OnceLock::new();
    REGISTRY.get_or_init(|| Mutex::new(HashMap::new()))
}

pub(crate) fn state_for(
    provider_key: &str,
    config: ProviderBackpressureConfig,
) -> Arc<ProviderState> {
    let mut states = registry().lock().expect("provider registry lock poisoned");
    states
        .entry(provider_key.to_string())
        .or_insert_with(|| Arc::new(ProviderState::new(config)))
        .clone()
}

pub fn provider_backpressure_snapshot(provider_key: &str) -> Option<ProviderBackpressureSnapshot> {
    registry()
        .lock()
        .expect("provider registry lock poisoned")
        .get(provider_key)
        .map(|state| state.snapshot())
}

pub fn all_provider_backpressure_snapshots() -> HashMap<String, ProviderBackpressureSnapshot> {
    registry()
        .lock()
        .expect("provider registry lock poisoned")
        .iter()
        .map(|(key, state)| (key.clone(), state.snapshot()))
        .collect()
}

pub(crate) async fn acquire(state: &Arc<ProviderState>) -> anyhow::Result<OwnedSemaphorePermit> {
    state.acquire().await
}

pub(crate) fn observe_retry_after(state: &ProviderState, retry_after_ms: Option<u64>) {
    state.observe_retry_after(retry_after_ms);
}

#[cfg(test)]
mod tests {
    use super::*;

    fn config() -> ProviderBackpressureConfig {
        ProviderBackpressureConfig {
            max_concurrent_requests: 1,
            min_request_interval_ms: 0,
            queue_timeout_ms: 40,
            max_cooldown_ms: 20,
        }
    }

    #[tokio::test]
    async fn provider_key_shares_a_fair_concurrency_permit() {
        let key = format!("test-provider-{}", uuid::Uuid::new_v4());
        let first_state = state_for(&key, config());
        let second_state = state_for(&key, config());
        assert!(Arc::ptr_eq(&first_state, &second_state));

        let permit = acquire(&first_state).await.unwrap();
        let waiter = tokio::spawn({
            let state = second_state.clone();
            async move { acquire(&state).await }
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        assert!(!waiter.is_finished());
        drop(permit);
        drop(waiter.await.unwrap().unwrap());

        let snapshot = provider_backpressure_snapshot(&key).unwrap();
        assert_eq!(snapshot.admitted, 2);
        assert_eq!(snapshot.queued, 1);
        assert_eq!(snapshot.rejected, 0);
    }

    #[tokio::test]
    async fn retry_after_is_shared_capped_and_observable() {
        let key = format!("test-provider-{}", uuid::Uuid::new_v4());
        let state = state_for(&key, config());
        observe_retry_after(&state, Some(1_000));
        let started = Instant::now();
        drop(acquire(&state).await.unwrap());
        assert!(started.elapsed() >= Duration::from_millis(15));
        assert_eq!(
            provider_backpressure_snapshot(&key)
                .unwrap()
                .cooldown_updates,
            1
        );
    }

    #[tokio::test]
    async fn queue_timeout_fails_closed_and_is_counted() {
        let key = format!("test-provider-{}", uuid::Uuid::new_v4());
        let state = state_for(
            &key,
            ProviderBackpressureConfig {
                queue_timeout_ms: 5,
                ..config()
            },
        );
        let _permit = acquire(&state).await.unwrap();
        let error = acquire(&state).await.unwrap_err();
        assert!(error.to_string().contains("provider_backpressure_timeout"));
        assert_eq!(provider_backpressure_snapshot(&key).unwrap().rejected, 1);
    }

    #[tokio::test]
    async fn provider_key_paces_request_starts_across_instances() {
        let key = format!("test-provider-{}", uuid::Uuid::new_v4());
        let pacing = ProviderBackpressureConfig {
            max_concurrent_requests: 2,
            min_request_interval_ms: 30,
            queue_timeout_ms: 100,
            max_cooldown_ms: 80,
        };
        let first_state = state_for(&key, pacing);
        let second_state = state_for(&key, pacing);

        drop(acquire(&first_state).await.unwrap());
        let started = Instant::now();
        drop(acquire(&second_state).await.unwrap());

        assert!(started.elapsed() >= Duration::from_millis(20));
        let snapshot = provider_backpressure_snapshot(&key).unwrap();
        assert_eq!(snapshot.admitted, 2);
        assert_eq!(snapshot.queued, 1);
        assert_eq!(snapshot.paced, 1);
        assert_eq!(snapshot.rejected, 0);
    }

    #[tokio::test]
    async fn shared_cooldown_extends_an_already_paced_waiter() {
        let key = format!("test-provider-{}", uuid::Uuid::new_v4());
        let pacing = ProviderBackpressureConfig {
            max_concurrent_requests: 2,
            min_request_interval_ms: 20,
            queue_timeout_ms: 120,
            max_cooldown_ms: 80,
        };
        let state = state_for(&key, pacing);
        drop(acquire(&state).await.unwrap());

        let waiter = tokio::spawn({
            let state = state.clone();
            async move {
                let started = Instant::now();
                drop(acquire(&state).await.unwrap());
                started.elapsed()
            }
        });
        tokio::time::sleep(Duration::from_millis(5)).await;
        observe_retry_after(&state, Some(45));

        assert!(waiter.await.unwrap() >= Duration::from_millis(40));
        let snapshot = provider_backpressure_snapshot(&key).unwrap();
        assert_eq!(snapshot.cooldown_updates, 1);
        assert_eq!(snapshot.paced, 1);
        assert_eq!(snapshot.rejected, 0);
    }
}
