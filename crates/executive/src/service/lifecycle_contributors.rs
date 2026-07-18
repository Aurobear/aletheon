//! Ordered, data-only lifecycle extensions owned by Executive.

use std::collections::{BTreeMap, HashSet};
use std::sync::{Arc, LazyLock, Mutex};
use std::time::Instant;

use async_trait::async_trait;

pub const MAX_EFFECTS_PER_DISPATCH: usize = 32;
pub const MAX_CONTEXT_FRAGMENT_BYTES: usize = 8 * 1024;
const MAX_CONTRIBUTOR_METRIC_SERIES: usize = 256;
const MAX_CONTRIBUTOR_ID_BYTES: usize = 128;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ContributorMetricSnapshot {
    pub dispatch_total: u64,
    pub failed_total: u64,
    pub critical_failclosed_total: u64,
    pub latency_micros_total: u64,
}

static CONTRIBUTOR_METRICS: LazyLock<Mutex<BTreeMap<String, ContributorMetricSnapshot>>> =
    LazyLock::new(|| Mutex::new(BTreeMap::new()));

pub fn contributor_metrics(id: &str) -> ContributorMetricSnapshot {
    CONTRIBUTOR_METRICS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
        .get(&bounded_contributor_id(id))
        .copied()
        .unwrap_or_default()
}

fn bounded_contributor_id(id: &str) -> String {
    let mut end = id.len().min(MAX_CONTRIBUTOR_ID_BYTES);
    while !id.is_char_boundary(end) {
        end -= 1;
    }
    id[..end].to_owned()
}

fn record_contributor_metric(id: &str, elapsed_micros: u64, failed: bool, critical: bool) {
    let id = bounded_contributor_id(id);
    let mut metrics = CONTRIBUTOR_METRICS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if !metrics.contains_key(&id) && metrics.len() == MAX_CONTRIBUTOR_METRIC_SERIES {
        return;
    }
    let metric = metrics.entry(id).or_default();
    metric.dispatch_total = metric.dispatch_total.saturating_add(1);
    metric.failed_total = metric.failed_total.saturating_add(u64::from(failed));
    metric.critical_failclosed_total = metric
        .critical_failclosed_total
        .saturating_add(u64::from(critical));
    metric.latency_micros_total = metric.latency_micros_total.saturating_add(elapsed_micros);
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub enum LifecyclePhase {
    BeforeSessionStart,
    AfterSessionStart,
    BeforeSessionEnd,
    AfterSessionEnd,
    BeforeTurnInput,
    AfterContextProjection,
    BeforeModelCall,
    BeforeToolBatch,
    AfterToolTerminal,
    AfterTurnTerminal,
    OnAbort,
}

impl LifecyclePhase {
    pub const fn is_blocking(self) -> bool {
        matches!(self, Self::BeforeToolBatch)
    }
}

#[derive(Debug, Clone)]
pub struct LifecycleInput {
    pub phase: LifecyclePhase,
    pub principal_id: fabric::PrincipalId,
    pub thread_id: fabric::ThreadId,
    pub turn_id: Option<fabric::TurnId>,
    pub session_id: String,
    pub detail: serde_json::Value,
}

#[derive(Debug, Clone, PartialEq)]
pub enum LifecycleEffect {
    Continue,
    AddContextFragment {
        source: String,
        content: String,
    },
    EmitEvent {
        schema: String,
        payload: serde_json::Value,
    },
    RequestCheckpoint,
    RequestCancellation {
        reason: String,
    },
    RejectInput {
        reason: String,
    },
}

#[derive(Debug, thiserror::Error)]
#[error("lifecycle contributor failed: {0}")]
pub struct ContributorError(pub String);

#[async_trait]
pub trait LifecycleContributor: Send + Sync {
    fn id(&self) -> &str;

    fn priority(&self) -> i32 {
        0
    }

    fn is_critical(&self) -> bool {
        false
    }

    async fn on_lifecycle(
        &self,
        input: &LifecycleInput,
    ) -> Result<Vec<LifecycleEffect>, ContributorError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContributorOutcome {
    pub id: String,
    pub elapsed_micros: u64,
    pub failed: bool,
}

#[derive(Debug, Clone, Default, PartialEq)]
pub struct LifecycleDispatch {
    pub effects: Vec<LifecycleEffect>,
    pub outcomes: Vec<ContributorOutcome>,
    pub effects_truncated: bool,
}

#[derive(Debug, thiserror::Error)]
pub enum LifecycleDispatchError {
    #[error("critical lifecycle contributor '{id}' failed: {detail}")]
    Critical { id: String, detail: String },
}

#[derive(Default)]
pub struct LifecycleRegistry {
    contributors: BTreeMap<LifecyclePhase, Vec<Arc<dyn LifecycleContributor>>>,
    ids: HashSet<String>,
}

impl LifecycleRegistry {
    pub fn register(
        &mut self,
        phase: LifecyclePhase,
        contributor: Arc<dyn LifecycleContributor>,
    ) -> Result<(), String> {
        let id = contributor.id().to_owned();
        if id.is_empty() {
            return Err("lifecycle contributor id cannot be empty".into());
        }
        if !self.ids.insert(id.clone()) {
            return Err(format!("duplicate lifecycle contributor id: {id}"));
        }
        let bucket = self.contributors.entry(phase).or_default();
        bucket.push(contributor);
        bucket.sort_by(|left, right| {
            left.priority()
                .cmp(&right.priority())
                .then_with(|| left.id().cmp(right.id()))
        });
        Ok(())
    }

    pub async fn dispatch(
        &self,
        input: LifecycleInput,
    ) -> Result<LifecycleDispatch, LifecycleDispatchError> {
        let Some(contributors) = self.contributors.get(&input.phase) else {
            return Ok(LifecycleDispatch::default());
        };
        let mut dispatch = LifecycleDispatch::default();
        for contributor in contributors {
            let started = Instant::now();
            match contributor.on_lifecycle(&input).await {
                Ok(effects) => {
                    let elapsed_micros =
                        started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
                    record_contributor_metric(contributor.id(), elapsed_micros, false, false);
                    dispatch.outcomes.push(ContributorOutcome {
                        id: contributor.id().to_owned(),
                        elapsed_micros,
                        failed: false,
                    });
                    for effect in effects {
                        if dispatch.effects.len() == MAX_EFFECTS_PER_DISPATCH {
                            dispatch.effects_truncated = true;
                            break;
                        }
                        if let Some(effect) = normalize_effect(input.phase, effect) {
                            dispatch.effects.push(effect);
                        }
                    }
                }
                Err(error) if contributor.is_critical() => {
                    let elapsed_micros =
                        started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
                    record_contributor_metric(contributor.id(), elapsed_micros, true, true);
                    return Err(LifecycleDispatchError::Critical {
                        id: contributor.id().to_owned(),
                        detail: error.to_string(),
                    });
                }
                Err(error) => {
                    let elapsed_micros =
                        started.elapsed().as_micros().try_into().unwrap_or(u64::MAX);
                    record_contributor_metric(contributor.id(), elapsed_micros, true, false);
                    tracing::warn!(
                        contributor = contributor.id(),
                        phase = ?input.phase,
                        %error,
                        "Non-critical lifecycle contributor failed"
                    );
                    dispatch.outcomes.push(ContributorOutcome {
                        id: contributor.id().to_owned(),
                        elapsed_micros,
                        failed: true,
                    });
                }
            }
        }
        Ok(dispatch)
    }

    /// Feature-gated dispatch boundary used by every production integration.
    pub async fn dispatch_if_enabled(
        &self,
        enabled: bool,
        input: LifecycleInput,
    ) -> Result<LifecycleDispatch, LifecycleDispatchError> {
        if !enabled {
            return Ok(LifecycleDispatch::default());
        }
        self.dispatch(input).await
    }
}

fn normalize_effect(phase: LifecyclePhase, effect: LifecycleEffect) -> Option<LifecycleEffect> {
    match effect {
        LifecycleEffect::AddContextFragment {
            source,
            mut content,
        } => {
            truncate_string(&mut content, MAX_CONTEXT_FRAGMENT_BYTES);
            Some(LifecycleEffect::AddContextFragment { source, content })
        }
        LifecycleEffect::RejectInput { reason } if !phase.is_blocking() => {
            tracing::warn!(?phase, %reason, "Ignoring RejectInput outside blocking lifecycle phase");
            None
        }
        other => Some(other),
    }
}

fn truncate_string(value: &mut String, max_bytes: usize) {
    if value.len() <= max_bytes {
        return;
    }
    let mut end = max_bytes;
    while !value.is_char_boundary(end) {
        end -= 1;
    }
    value.truncate(end);
}

#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use super::*;

    struct TestContributor {
        id: String,
        priority: i32,
        critical: bool,
        fail: bool,
        effects: Vec<LifecycleEffect>,
        calls: Arc<Mutex<Vec<String>>>,
    }

    #[async_trait]
    impl LifecycleContributor for TestContributor {
        fn id(&self) -> &str {
            &self.id
        }

        fn priority(&self) -> i32 {
            self.priority
        }

        fn is_critical(&self) -> bool {
            self.critical
        }

        async fn on_lifecycle(
            &self,
            _input: &LifecycleInput,
        ) -> Result<Vec<LifecycleEffect>, ContributorError> {
            self.calls.lock().unwrap().push(self.id.clone());
            if self.fail {
                Err(ContributorError("boom".into()))
            } else {
                Ok(self.effects.clone())
            }
        }
    }

    fn input(phase: LifecyclePhase) -> LifecycleInput {
        LifecycleInput {
            phase,
            principal_id: fabric::PrincipalId("principal".into()),
            thread_id: fabric::ThreadId("thread".into()),
            turn_id: None,
            session_id: "session".into(),
            detail: serde_json::Value::Null,
        }
    }

    fn contributor(
        id: &str,
        priority: i32,
        calls: Arc<Mutex<Vec<String>>>,
    ) -> Arc<TestContributor> {
        Arc::new(TestContributor {
            id: id.into(),
            priority,
            critical: false,
            fail: false,
            effects: vec![LifecycleEffect::Continue],
            calls,
        })
    }

    #[tokio::test]
    async fn registration_rejects_duplicate_ids_and_dispatch_is_stable() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut registry = LifecycleRegistry::default();
        registry
            .register(
                LifecyclePhase::BeforeTurnInput,
                contributor("z", 0, calls.clone()),
            )
            .unwrap();
        registry
            .register(
                LifecyclePhase::BeforeTurnInput,
                contributor("a", 0, calls.clone()),
            )
            .unwrap();
        registry
            .register(
                LifecyclePhase::BeforeTurnInput,
                contributor("first", -1, calls.clone()),
            )
            .unwrap();
        assert!(registry
            .register(
                LifecyclePhase::AfterTurnTerminal,
                contributor("a", 99, calls.clone()),
            )
            .is_err());

        registry
            .dispatch(input(LifecyclePhase::BeforeTurnInput))
            .await
            .unwrap();
        assert_eq!(&*calls.lock().unwrap(), &["first", "a", "z"]);
    }

    #[tokio::test]
    async fn effects_and_context_are_bounded_and_reject_is_phase_safe() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut registry = LifecycleRegistry::default();
        registry
            .register(
                LifecyclePhase::AfterTurnTerminal,
                Arc::new(TestContributor {
                    id: "bounded".into(),
                    priority: 0,
                    critical: false,
                    fail: false,
                    effects: std::iter::repeat_n(LifecycleEffect::Continue, 40)
                        .chain([
                            LifecycleEffect::AddContextFragment {
                                source: "test".into(),
                                content: "界".repeat(8_000),
                            },
                            LifecycleEffect::RejectInput {
                                reason: "too late".into(),
                            },
                        ])
                        .collect(),
                    calls,
                }),
            )
            .unwrap();
        let dispatch = registry
            .dispatch(input(LifecyclePhase::AfterTurnTerminal))
            .await
            .unwrap();
        assert_eq!(dispatch.effects.len(), MAX_EFFECTS_PER_DISPATCH);
        assert!(dispatch.effects_truncated);

        let mut nonblocking = LifecycleRegistry::default();
        nonblocking
            .register(
                LifecyclePhase::AfterTurnTerminal,
                Arc::new(TestContributor {
                    id: "late-reject".into(),
                    priority: 0,
                    critical: false,
                    fail: false,
                    effects: vec![LifecycleEffect::RejectInput {
                        reason: "too late".into(),
                    }],
                    calls: Arc::new(Mutex::new(Vec::new())),
                }),
            )
            .unwrap();
        assert!(nonblocking
            .dispatch(input(LifecyclePhase::AfterTurnTerminal))
            .await
            .unwrap()
            .effects
            .is_empty());

        let mut registry = LifecycleRegistry::default();
        registry
            .register(
                LifecyclePhase::BeforeToolBatch,
                Arc::new(TestContributor {
                    id: "fragment".into(),
                    priority: 0,
                    critical: false,
                    fail: false,
                    effects: vec![
                        LifecycleEffect::AddContextFragment {
                            source: "test".into(),
                            content: "界".repeat(8_000),
                        },
                        LifecycleEffect::RejectInput {
                            reason: "denied".into(),
                        },
                    ],
                    calls: Arc::new(Mutex::new(Vec::new())),
                }),
            )
            .unwrap();
        let dispatch = registry
            .dispatch(input(LifecyclePhase::BeforeToolBatch))
            .await
            .unwrap();
        assert!(matches!(
            &dispatch.effects[0],
            LifecycleEffect::AddContextFragment { content, .. }
                if content.len() <= MAX_CONTEXT_FRAGMENT_BYTES && content.is_char_boundary(content.len())
        ));
        assert!(matches!(
            &dispatch.effects[1],
            LifecycleEffect::RejectInput { reason } if reason == "denied"
        ));
    }

    #[tokio::test]
    async fn critical_failure_is_closed_and_noncritical_failure_isolated() {
        for critical in [false, true] {
            let calls = Arc::new(Mutex::new(Vec::new()));
            let mut registry = LifecycleRegistry::default();
            registry
                .register(
                    LifecyclePhase::BeforeTurnInput,
                    Arc::new(TestContributor {
                        id: format!("failure-{critical}"),
                        priority: 0,
                        critical,
                        fail: true,
                        effects: vec![],
                        calls,
                    }),
                )
                .unwrap();
            let result = registry
                .dispatch(input(LifecyclePhase::BeforeTurnInput))
                .await;
            let metric = contributor_metrics(&format!("failure-{critical}"));
            assert_eq!(metric.dispatch_total, 1);
            assert_eq!(metric.failed_total, 1);
            assert_eq!(metric.critical_failclosed_total, u64::from(critical));
            if critical {
                assert!(matches!(
                    result,
                    Err(LifecycleDispatchError::Critical { .. })
                ));
            } else {
                let dispatch = result.unwrap();
                assert!(dispatch.outcomes[0].failed);
            }
        }
    }

    #[tokio::test]
    async fn disabled_dispatch_is_a_strict_noop() {
        let calls = Arc::new(Mutex::new(Vec::new()));
        let mut registry = LifecycleRegistry::default();
        registry
            .register(
                LifecyclePhase::BeforeTurnInput,
                contributor("must-not-run", 0, calls.clone()),
            )
            .unwrap();
        let dispatch = registry
            .dispatch_if_enabled(false, input(LifecyclePhase::BeforeTurnInput))
            .await
            .unwrap();
        assert_eq!(dispatch, LifecycleDispatch::default());
        assert!(calls.lock().unwrap().is_empty());
    }
}
