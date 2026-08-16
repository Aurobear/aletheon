use crate::chronos::SystemTimer;
use ::contracts::{OperationExitReason, OperationId, Timer};
use std::future::Future;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::OnceLock;
use std::time::{Duration, Instant};
use tokio::task::JoinSet;
use tokio_util::sync::CancellationToken;

static ACTIVE_SCOPES: AtomicUsize = AtomicUsize::new(0);
static ACTIVE_RESOURCES: AtomicUsize = AtomicUsize::new(0);
static DROP_FALLBACKS: AtomicUsize = AtomicUsize::new(0);
/// Resources abandoned by a dropped scope before terminal task exits were
/// observed. A non-zero value means the process has outstanding external
/// cleanup that a recovery owner must finish before the zero-resource
/// invariant can be claimed.
static PENDING_RECLAIM: AtomicUsize = AtomicUsize::new(0);
/// Dropped scopes that could not enter the bounded recovery queue. These
/// resources remain counted as active so installed acceptance fails visibly.
static RECLAIM_QUEUE_EXHAUSTED: AtomicUsize = AtomicUsize::new(0);

const RECLAIM_QUEUE_CAPACITY: usize = 128;

struct ReclaimRequest {
    operation_id: OperationId,
    tasks: JoinSet<TaskExit>,
    resource_count: usize,
}

static RECLAIM_SENDER: OnceLock<tokio::sync::mpsc::Sender<ReclaimRequest>> = OnceLock::new();

/// Return the process-wide, bounded recovery owner. The worker is started on
/// the daemon runtime once and drains the exact JoinSet owned by the dropped
/// OperationId; it never guesses by PID, name, or a later generation.
fn reclaim_sender() -> Option<&'static tokio::sync::mpsc::Sender<ReclaimRequest>> {
    if let Some(sender) = RECLAIM_SENDER.get() {
        return Some(sender);
    }

    let runtime = tokio::runtime::Handle::try_current().ok()?;
    let (sender, mut receiver) = tokio::sync::mpsc::channel(RECLAIM_QUEUE_CAPACITY);
    if RECLAIM_SENDER.set(sender).is_ok() {
        runtime.spawn(async move {
            while let Some(mut reclaim) = receiver.recv().await {
                reclaim.tasks.abort_all();
                while reclaim.tasks.join_next().await.is_some() {}
                ACTIVE_RESOURCES.fetch_sub(reclaim.resource_count, Ordering::AcqRel);
                PENDING_RECLAIM.fetch_sub(1, Ordering::AcqRel);
                tracing::debug!(
                    operation = %reclaim.operation_id.0,
                    resources = reclaim.resource_count,
                    "dropped operation scope resources reached terminal reclaim"
                );
            }
        });
    }
    RECLAIM_SENDER.get()
}

/// Structured task exit recorded by an operation scope.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TaskExit {
    pub name: String,
    pub reason: OperationExitReason,
}

/// Authoritative cleanup path used to close an operation scope.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OperationScopeCleanupKind {
    Settled,
    Aborted,
}

/// Immutable cleanup receipt. Repeated drain calls replay this same receipt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OperationScopeCleanupReport {
    pub operation_id: OperationId,
    pub kind: OperationScopeCleanupKind,
    pub exits: Vec<TaskExit>,
    /// True when cooperative completion exceeded the grace period and the
    /// remaining tasks had to be aborted.
    pub forced_abort: bool,
}

/// Process-wide gauges for resources registered with [`OperationScope`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OperationScopeMetrics {
    pub active_scopes: usize,
    pub active_resources: usize,
    pub drop_fallbacks: usize,
    /// Resources whose terminal cleanup was not observed before their scope
    /// was dropped. A zero value is the only state in which the zero-resource
    /// invariant can be claimed.
    pub pending_reclaim: usize,
    /// Dropped scopes rejected by the bounded reclaim queue. Active resource
    /// gauges intentionally remain non-zero until an operator restarts or a
    /// future durable recovery mechanism reconciles them.
    pub reclaim_queue_exhausted: usize,
}

pub fn operation_scope_metrics() -> OperationScopeMetrics {
    OperationScopeMetrics {
        active_scopes: ACTIVE_SCOPES.load(Ordering::Acquire),
        active_resources: ACTIVE_RESOURCES.load(Ordering::Acquire),
        drop_fallbacks: DROP_FALLBACKS.load(Ordering::Acquire),
        pending_reclaim: PENDING_RECLAIM.load(Ordering::Acquire),
        reclaim_queue_exhausted: RECLAIM_QUEUE_EXHAUSTED.load(Ordering::Acquire),
    }
}

/// Structured concurrency scope for tasks owned by one Operation.
pub struct OperationScope {
    pub id: OperationId,
    cancel: CancellationToken,
    tasks: JoinSet<TaskExit>,
    registered_resources: usize,
    cleanup: Option<OperationScopeCleanupReport>,
    counted_active: bool,
}

impl std::fmt::Debug for OperationScope {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("OperationScope")
            .field("id", &self.id)
            .field("registered_resources", &self.registered_resources)
            .field("cleanup", &self.cleanup)
            .finish_non_exhaustive()
    }
}

impl OperationScope {
    pub fn new(id: OperationId) -> Self {
        Self::with_cancellation(id, CancellationToken::new())
    }

    /// Bind the scope to the turn's host-owned cancellation lineage.
    pub fn with_cancellation(id: OperationId, cancel: CancellationToken) -> Self {
        ACTIVE_SCOPES.fetch_add(1, Ordering::AcqRel);
        Self {
            id,
            cancel,
            tasks: JoinSet::new(),
            registered_resources: 0,
            cleanup: None,
            counted_active: true,
        }
    }

    pub fn token(&self) -> CancellationToken {
        self.cancel.clone()
    }

    pub fn cancel(&self) {
        self.cancel.cancel();
    }

    pub fn is_empty(&self) -> bool {
        self.tasks.is_empty()
    }

    pub fn spawn<F>(&mut self, name: impl Into<String>, fut: F)
    where
        F: Future<Output = OperationExitReason> + Send + 'static,
    {
        assert!(
            self.cleanup.is_none(),
            "cannot register a resource after OperationScope cleanup"
        );
        let name = name.into();
        self.tasks.spawn(async move {
            let reason = fut.await;
            TaskExit { name, reason }
        });
        self.registered_resources = self.registered_resources.saturating_add(1);
        ACTIVE_RESOURCES.fetch_add(1, Ordering::AcqRel);
    }

    pub async fn join_next(&mut self) -> Option<TaskExit> {
        let joined = self.tasks.join_next().await?;
        self.record_resource_released();
        match joined {
            Ok(exit) => Some(exit),
            Err(err) => Some(TaskExit {
                name: "<join-error>".into(),
                reason: if err.is_panic() {
                    OperationExitReason::Panic(format!("{err}"))
                } else {
                    OperationExitReason::Cancelled(::contracts::CancelReason::Other(format!(
                        "{err}"
                    )))
                },
            }),
        }
    }

    /// Wait for normal task completion without cancelling the turn. If the
    /// grace period expires, the method falls back to the abort protocol.
    pub async fn settle_and_drain(
        &mut self,
        _clock: &dyn ::contracts::Clock,
        grace: Duration,
    ) -> OperationScopeCleanupReport {
        self.drain(OperationScopeCleanupKind::Settled, grace).await
    }

    /// Cancel the turn and account for every registered task before returning.
    pub async fn abort_and_drain(
        &mut self,
        _clock: &dyn ::contracts::Clock,
        grace: Duration,
    ) -> OperationScopeCleanupReport {
        self.cancel.cancel();
        self.drain(OperationScopeCleanupKind::Aborted, grace).await
    }

    /// Compatibility adapter for callers using the original PR-3 API.
    pub async fn cancel_and_drain(
        &mut self,
        clock: &dyn ::contracts::Clock,
        grace: Duration,
    ) -> Vec<TaskExit> {
        self.abort_and_drain(clock, grace).await.exits
    }

    async fn drain(
        &mut self,
        kind: OperationScopeCleanupKind,
        grace: Duration,
    ) -> OperationScopeCleanupReport {
        if let Some(report) = &self.cleanup {
            return report.clone();
        }

        let timer = SystemTimer;
        let started = Instant::now();
        let mut exits = Vec::new();
        let mut forced_abort = false;
        while !self.tasks.is_empty() {
            let remaining = grace.saturating_sub(started.elapsed());
            if remaining.is_zero() {
                forced_abort = true;
                break;
            }
            match timer.timeout(remaining, self.join_next()).await {
                Ok(Some(exit)) => exits.push(exit),
                Ok(None) => break,
                Err(_) => {
                    forced_abort = true;
                    break;
                }
            }
        }
        if forced_abort {
            self.cancel.cancel();
            self.tasks.abort_all();
            while let Some(exit) = self.join_next().await {
                exits.push(exit);
            }
        }

        let report = OperationScopeCleanupReport {
            operation_id: self.id,
            kind,
            exits,
            forced_abort,
        };
        self.cleanup = Some(report.clone());
        self.finish_active_scope();
        report
    }

    fn record_resource_released(&mut self) {
        if self.registered_resources > 0 {
            self.registered_resources -= 1;
            ACTIVE_RESOURCES.fetch_sub(1, Ordering::AcqRel);
        }
    }

    fn finish_active_scope(&mut self) {
        if self.counted_active {
            self.counted_active = false;
            ACTIVE_SCOPES.fetch_sub(1, Ordering::AcqRel);
        }
    }
}

impl Drop for OperationScope {
    fn drop(&mut self) {
        if self.cleanup.is_none() {
            self.cancel.cancel();
            self.tasks.abort_all();
            if self.registered_resources > 0 {
                // Abort is asynchronous with respect to the external cleanup
                // owned by each task. Move the exact operation-owned JoinSet to
                // the one bounded recovery owner and retain the resource gauge
                // until every terminal task exit has been observed.
                let resource_count = self.registered_resources;
                let tasks = std::mem::replace(&mut self.tasks, JoinSet::new());
                PENDING_RECLAIM.fetch_add(1, Ordering::AcqRel);
                let reclaim = ReclaimRequest {
                    operation_id: self.id,
                    tasks,
                    resource_count,
                };
                let queued =
                    reclaim_sender().is_some_and(|sender| sender.try_send(reclaim).is_ok());
                if queued {
                    self.registered_resources = 0;
                } else {
                    // The request (and JoinSet) is dropped here, which still
                    // aborts tasks, but terminal cleanup was not authoritatively
                    // observed. Keep both gauges non-zero and make saturation
                    // explicit rather than claiming a false clean state.
                    RECLAIM_QUEUE_EXHAUSTED.fetch_add(1, Ordering::AcqRel);
                }
            }
            DROP_FALLBACKS.fetch_add(1, Ordering::AcqRel);
        }
        self.finish_active_scope();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chronos::TestClock;
    use ::contracts::CancelReason;
    use std::sync::atomic::AtomicBool;

    static GAUGE_TEST_LOCK: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());

    #[tokio::test]
    async fn interleaved_scopes_keep_cancellation_and_resources_local() {
        let _guard = GAUGE_TEST_LOCK.lock().await;
        let baseline = operation_scope_metrics();
        let mut first = OperationScope::new(OperationId::new());
        let mut second = OperationScope::new(OperationId::new());
        let first_token = first.token();
        let second_token = second.token();
        let second_observer = second_token.clone();
        first.spawn("first", async move {
            first_token.cancelled().await;
            OperationExitReason::Cancelled(CancelReason::User)
        });
        second.spawn("second", async move {
            second_observer.cancelled().await;
            OperationExitReason::Cancelled(CancelReason::Shutdown)
        });

        first.cancel();
        assert!(!second_token.is_cancelled());
        let clock = TestClock::default();
        let first_report = first
            .abort_and_drain(&clock, Duration::from_millis(50))
            .await;
        assert_eq!(first_report.exits[0].name, "first");
        assert_eq!(
            operation_scope_metrics().active_resources,
            baseline.active_resources + 1
        );

        second.cancel();
        let second_report = second
            .abort_and_drain(&clock, Duration::from_millis(50))
            .await;
        assert_eq!(second_report.exits[0].name, "second");
        assert_eq!(
            operation_scope_metrics().active_scopes,
            baseline.active_scopes
        );
        assert_eq!(
            operation_scope_metrics().active_resources,
            baseline.active_resources
        );
    }

    #[tokio::test]
    async fn repeated_drain_replays_one_cleanup_receipt() {
        let _guard = GAUGE_TEST_LOCK.lock().await;
        let baseline = operation_scope_metrics();
        let mut scope = OperationScope::new(OperationId::new());
        scope.spawn("done", async { OperationExitReason::Completed });
        let clock = TestClock::default();
        let first = scope
            .settle_and_drain(&clock, Duration::from_millis(50))
            .await;
        let replay = scope
            .abort_and_drain(&clock, Duration::from_millis(50))
            .await;
        assert_eq!(first, replay);
        assert_eq!(first.kind, OperationScopeCleanupKind::Settled);
        assert_eq!(
            operation_scope_metrics().active_scopes,
            baseline.active_scopes
        );
        assert_eq!(
            operation_scope_metrics().active_resources,
            baseline.active_resources
        );
    }

    #[tokio::test]
    async fn drop_fallback_reclaims_before_removing_resource_gauges() {
        let _guard = GAUGE_TEST_LOCK.lock().await;
        let baseline = operation_scope_metrics();
        let cleanup_observed = std::sync::Arc::new(AtomicBool::new(false));
        struct CleanupSignal(std::sync::Arc<AtomicBool>);
        impl Drop for CleanupSignal {
            fn drop(&mut self) {
                self.0.store(true, Ordering::Release);
            }
        }

        let token = {
            let mut scope = OperationScope::new(OperationId::new());
            let token = scope.token();
            let cleanup_observed = cleanup_observed.clone();
            scope.spawn("pending", async move {
                let _cleanup = CleanupSignal(cleanup_observed);
                std::future::pending().await
            });
            tokio::task::yield_now().await;
            token
        };
        assert!(token.is_cancelled());
        assert_eq!(
            operation_scope_metrics().active_scopes,
            baseline.active_scopes
        );

        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                let snapshot = operation_scope_metrics();
                if snapshot.active_resources == baseline.active_resources
                    && snapshot.pending_reclaim == baseline.pending_reclaim
                {
                    assert!(
                        cleanup_observed.load(Ordering::Acquire),
                        "resource gauge reached zero before task cleanup was observed"
                    );
                    break;
                }
                assert!(
                    snapshot.active_resources > baseline.active_resources
                        || snapshot.pending_reclaim > baseline.pending_reclaim,
                    "reclaim must stay fail-visible until terminal cleanup"
                );
                tokio::task::yield_now().await;
            }
        })
        .await
        .expect("bounded recovery owner should reclaim the dropped scope");

        let after = operation_scope_metrics();
        assert_eq!(after.drop_fallbacks, baseline.drop_fallbacks + 1);
        assert_eq!(
            after.reclaim_queue_exhausted, baseline.reclaim_queue_exhausted,
            "the normal drop fallback must fit in the bounded recovery queue"
        );
    }
}
