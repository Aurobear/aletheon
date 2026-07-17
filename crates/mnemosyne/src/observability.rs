//! Sanitized, bounded-cardinality observability for the unified memory runtime.
//!
//! Labels are closed enums rather than caller-provided strings. Snapshots therefore
//! cannot accidentally retain record IDs, queries, content, principals, or session IDs.

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{MemoryKind, MemoryScope};

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryKindLabel {
    Message,
    ToolOutcome,
    GoalOutcome,
    Reflection,
    Episodic,
    SemanticFact,
    Procedure,
    CoreState,
    ArchitectureDecision,
    ExternalReference,
}

impl From<MemoryKind> for MemoryKindLabel {
    fn from(value: MemoryKind) -> Self {
        match value {
            MemoryKind::Message => Self::Message,
            MemoryKind::ToolOutcome => Self::ToolOutcome,
            MemoryKind::GoalOutcome => Self::GoalOutcome,
            MemoryKind::Reflection => Self::Reflection,
            MemoryKind::Episodic => Self::Episodic,
            MemoryKind::SemanticFact => Self::SemanticFact,
            MemoryKind::Procedure => Self::Procedure,
            MemoryKind::CoreState => Self::CoreState,
            MemoryKind::ArchitectureDecision => Self::ArchitectureDecision,
            MemoryKind::ExternalReference => Self::ExternalReference,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum MemoryScopeLabel {
    Global,
    Principal,
    Session,
    Goal,
    Agent,
    Task,
}

impl From<&MemoryScope> for MemoryScopeLabel {
    fn from(value: &MemoryScope) -> Self {
        match value {
            MemoryScope::Global => Self::Global,
            MemoryScope::Principal(_) => Self::Principal,
            MemoryScope::Session(_) => Self::Session,
            MemoryScope::Goal(_) => Self::Goal,
            MemoryScope::Agent(_) => Self::Agent,
            MemoryScope::Task(_) => Self::Task,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallSourceLabel {
    RecallMemory,
    FactStore,
    Episodic,
    Core,
    Gbrain,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum RecallOmittedReason {
    InvalidRequest,
    SourceDegraded,
    Historical,
    Tombstoned,
    ItemLimit,
    ByteLimit,
    Duplicate,
    Sensitive,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ConsolidationJobState {
    Pending,
    Leased,
    Succeeded,
    SucceededNoOutput,
    RetryableFailure,
    PermanentFailure,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CandidateDecisionLabel {
    Insert,
    Merge,
    Reject,
    Supersede,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum GbrainDegradedCategory {
    Auth,
    Schema,
    InvalidPage,
    RejectedArguments,
    Timeout,
    Cancelled,
    RateLimited,
    Provider,
    Transport,
    MalformedResponse,
    OversizedResponse,
    Spool,
    Unsupported,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum TombstoneDestination {
    Local,
    Gbrain,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LatencySamples {
    pub count: u64,
    pub sum_ms: u64,
    pub max_ms: u64,
    pub last_ms: u64,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct MemoryMetricsSnapshot {
    pub memory_record_total: BTreeMap<MemoryKindLabel, BTreeMap<MemoryScopeLabel, u64>>,
    pub memory_recall_latency_ms: BTreeMap<RecallSourceLabel, LatencySamples>,
    pub memory_recall_hits: BTreeMap<RecallSourceLabel, BTreeMap<MemoryKindLabel, u64>>,
    pub memory_recall_omitted_total: BTreeMap<RecallOmittedReason, u64>,
    pub memory_consolidation_jobs: BTreeMap<ConsolidationJobState, u64>,
    pub memory_candidate_decisions: BTreeMap<CandidateDecisionLabel, u64>,
    pub memory_gbrain_queue_depth: u64,
    pub memory_gbrain_degraded: BTreeMap<GbrainDegradedCategory, u64>,
    pub memory_tombstone_pending_total: BTreeMap<TombstoneDestination, u64>,
}

/// Cloneable metrics handle shared by all memory pipeline components.
#[derive(Clone, Debug, Default)]
pub struct MemoryMetrics(Arc<Mutex<MemoryMetricsSnapshot>>);

impl MemoryMetrics {
    pub fn snapshot(&self) -> MemoryMetricsSnapshot {
        self.0
            .lock()
            .expect("memory metrics mutex poisoned")
            .clone()
    }

    pub fn record_stored(&self, kind: MemoryKind, scope: &MemoryScope) {
        increment_nested(
            &mut self.lock().memory_record_total,
            kind.into(),
            scope.into(),
            1,
        );
    }

    pub fn observe_recall_latency(&self, source: RecallSourceLabel, elapsed: Duration) {
        let millis = u64::try_from(elapsed.as_millis()).unwrap_or(u64::MAX);
        let mut guard = self.lock();
        let samples = guard.memory_recall_latency_ms.entry(source).or_default();
        samples.count = samples.count.saturating_add(1);
        samples.sum_ms = samples.sum_ms.saturating_add(millis);
        samples.max_ms = samples.max_ms.max(millis);
        samples.last_ms = millis;
    }

    pub fn recall_hit(&self, source: RecallSourceLabel, kind: MemoryKind, count: usize) {
        increment_nested(
            &mut self.lock().memory_recall_hits,
            source,
            kind.into(),
            count as u64,
        );
    }

    pub fn recall_omitted(&self, reason: RecallOmittedReason, count: usize) {
        increment(
            &mut self.lock().memory_recall_omitted_total,
            reason,
            count as u64,
        );
    }

    pub fn set_consolidation_jobs(&self, state: ConsolidationJobState, count: usize) {
        self.lock()
            .memory_consolidation_jobs
            .insert(state, count as u64);
    }

    pub fn candidate_decision(&self, decision: CandidateDecisionLabel) {
        increment(&mut self.lock().memory_candidate_decisions, decision, 1);
    }

    pub fn set_gbrain_queue_depth(&self, depth: usize) {
        self.lock().memory_gbrain_queue_depth = depth as u64;
    }

    pub fn gbrain_degraded(&self, category: GbrainDegradedCategory) {
        increment(&mut self.lock().memory_gbrain_degraded, category, 1);
    }

    pub fn set_tombstone_pending(&self, destination: TombstoneDestination, count: usize) {
        self.lock()
            .memory_tombstone_pending_total
            .insert(destination, count as u64);
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, MemoryMetricsSnapshot> {
        self.0.lock().expect("memory metrics mutex poisoned")
    }
}

fn increment<K: Ord>(values: &mut BTreeMap<K, u64>, key: K, amount: u64) {
    let value = values.entry(key).or_default();
    *value = value.saturating_add(amount);
}

fn increment_nested<K1: Ord, K2: Ord>(
    values: &mut BTreeMap<K1, BTreeMap<K2, u64>>,
    outer: K1,
    inner: K2,
    amount: u64,
) {
    increment(values.entry(outer).or_default(), inner, amount);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn snapshot_has_bounded_labels_and_never_retains_sensitive_strings() {
        let metrics = MemoryMetrics::default();
        metrics.record_stored(
            MemoryKind::Message,
            &MemoryScope::Session("secret-session".into()),
        );
        metrics.recall_hit(RecallSourceLabel::FactStore, MemoryKind::SemanticFact, 2);
        metrics.observe_recall_latency(RecallSourceLabel::FactStore, Duration::from_millis(7));
        metrics.gbrain_degraded(GbrainDegradedCategory::Auth);
        metrics.set_gbrain_queue_depth(3);
        metrics.set_tombstone_pending(TombstoneDestination::Gbrain, 4);

        let snapshot = metrics.snapshot();
        assert_eq!(
            snapshot
                .memory_record_total
                .get(&MemoryKindLabel::Message)
                .and_then(|scopes| scopes.get(&MemoryScopeLabel::Session)),
            Some(&1)
        );
        assert_eq!(snapshot.memory_gbrain_queue_depth, 3);
        assert_eq!(
            snapshot.memory_recall_latency_ms[&RecallSourceLabel::FactStore],
            LatencySamples {
                count: 1,
                sum_ms: 7,
                max_ms: 7,
                last_ms: 7,
            }
        );
        let debug = format!("{snapshot:?}");
        assert!(!debug.contains("secret-session"));
        assert!(serde_json::to_string(&snapshot).is_ok());
    }

    #[test]
    fn consolidation_job_metrics_are_current_state_gauges() {
        use crate::consolidation::{ConsolidationRepository, ExtractionCompletion, ExtractionJob};

        let directory = tempfile::tempdir().unwrap();
        let repository = ConsolidationRepository::open(directory.path().join("jobs.db")).unwrap();
        let metrics = MemoryMetrics::default();
        repository.set_metrics(metrics.clone());
        repository
            .enqueue_extraction(&ExtractionJob {
                idempotency_key: "job-1".into(),
                session_id: "session-private".into(),
                goal_id: None,
                ephemeral: false,
                memory_worker: false,
                completed_at_ms: Some(1),
                watermark: "private-watermark".into(),
                created_at_ms: 1,
            })
            .unwrap();
        let pending = metrics.snapshot();
        assert_eq!(
            pending.memory_consolidation_jobs[&ConsolidationJobState::Pending],
            1
        );

        let lease = repository
            .claim_extraction("worker", 2, 100, 100)
            .unwrap()
            .unwrap();
        let leased = metrics.snapshot();
        assert_eq!(
            leased.memory_consolidation_jobs[&ConsolidationJobState::Pending],
            0
        );
        assert_eq!(
            leased.memory_consolidation_jobs[&ConsolidationJobState::Leased],
            1
        );

        repository
            .complete(&lease, ExtractionCompletion::SucceededNoOutput, 3)
            .unwrap();
        let succeeded = metrics.snapshot();
        assert_eq!(
            succeeded.memory_consolidation_jobs[&ConsolidationJobState::Leased],
            0
        );
        assert_eq!(
            succeeded.memory_consolidation_jobs[&ConsolidationJobState::SucceededNoOutput],
            1
        );
        assert!(!format!("{succeeded:?}").contains("private"));
    }
}
