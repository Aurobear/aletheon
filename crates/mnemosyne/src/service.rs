//! MemoryService — unified facade over the Mnemosyne memory objects.
//!
//! Additive, low-risk facade (see docs/arch §11): wraps the existing
//! `RecallMemory` / `FactStore` / `CoreMemory` / `EpisodicMemory` handles
//! behind a single `record` / `recall` / `consolidate` / `forget` contract,
//! without removing or renaming any of the underlying fields.
//!
//! NOTE: `MemoryScope` here is a facade-local, coarse-grained scope
//! (`All` | `Session`) used only for `consolidate`/`forget`. It is
//! intentionally NOT re-exported at the crate root because the crate
//! already exports a richer multi-agent `MemoryScope`
//! (`r#impl::core_memory::scope::MemoryScope`, `Global`/`Session`/`Agent`)
//! used by `CoreMemory`. Re-exporting both under the same name would
//! collide, so callers reach this type via `mnemosyne::service::MemoryScope`.

use std::sync::Arc;
use std::time::Instant;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fabric::{self, wall_to_datetime, ReflectionEntry, ReflectionOutcome, ReflectionTrigger};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

use crate::adapters::storage::fact_store::FactStore;
use crate::adapters::storage::recall_memory::RecallMemory;
use crate::backends::EpisodicMemory;
use crate::domain::core_memory::{CoreMemory, MemoryBlock};
pub use crate::model::{
    MemoryAuthority, MemoryMetadata, MemoryProvenance, MemoryScope, MemorySensitivity,
    TemporalState,
};
use crate::model::{MemoryKind, MemoryRecord, MemoryRecordId, MemoryStatus};
use crate::observability::{
    MemoryMetrics, RecallOmittedReason, RecallSourceLabel, TombstoneDestination,
};

/// A unit of experience to be recorded into memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ExperienceEvent {
    /// A conversational turn (user or assistant message).
    Message {
        /// Logical session this message belongs to.
        session: String,
        /// "user" | "assistant" | other role label.
        role: String,
        content: String,
        metadata: MemoryMetadata,
    },
    /// A reflection summary (e.g. produced after a task).
    Reflection {
        content: String,
        metadata: MemoryMetadata,
    },
    ArchitectureDecision {
        title: String,
        content: String,
        metadata: MemoryMetadata,
    },
    GoalOutcome {
        goal_id: String,
        outcome: String,
        content: String,
        metadata: MemoryMetadata,
    },
}

/// A recall query.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallRequest {
    pub session: String,
    pub query: String,
    pub max_items: usize,
    pub max_content_bytes: usize,
    pub current_at: Option<DateTime<Utc>>,
    pub include_historical: bool,
    /// Optional recall mode override. When `Some`, the pipeline resolves
    /// the corresponding `RecallModeBundle` instead of using defaults.
    #[serde(default)]
    pub mode: Option<crate::recall::pipeline::RecallMode>,
}

impl RecallRequest {
    pub const MAX_QUERY_BYTES: usize = 4 * 1024;
    pub const MAX_ITEMS: usize = 100;
    pub const MAX_CONTENT_BYTES: usize = 256 * 1024;

    pub fn bounded(session: impl Into<String>, query: impl Into<String>) -> Self {
        Self {
            session: session.into(),
            query: query.into(),
            max_items: 20,
            max_content_bytes: 64 * 1024,
            current_at: None,
            include_historical: false,
            mode: None,
        }
    }

    pub(crate) fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.query.trim().is_empty(),
            "memory recall query is required"
        );
        anyhow::ensure!(
            self.query.len() <= Self::MAX_QUERY_BYTES,
            "memory recall query exceeds byte limit"
        );
        anyhow::ensure!(
            (1..=Self::MAX_ITEMS).contains(&self.max_items),
            "memory recall item limit is invalid"
        );
        anyhow::ensure!(
            (1..=Self::MAX_CONTENT_BYTES).contains(&self.max_content_bytes),
            "memory recall content limit is invalid"
        );
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct RecallItem {
    pub content: String,
    pub metadata: MemoryMetadata,
    pub temporal_state: TemporalState,
    #[serde(default)]
    pub authority: MemoryAuthority,
    pub scope: MemoryScope,
    /// Relevance score set by the recall pipeline (0.0 = unranked).
    #[serde(default)]
    pub score: f32,
    /// Evidence level stamped during recall post-processing.
    #[serde(default)]
    pub evidence: Option<crate::recall::evidence::EvidenceLevel>,
}

/// Result of a recall query.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecallSet {
    pub items: Vec<RecallItem>,
    #[serde(default)]
    pub degraded_sources: Vec<String>,
}

impl RecallSet {
    /// Compatibility view for consumers that have not migrated to provenance.
    pub fn texts(&self) -> Vec<&str> {
        self.items
            .iter()
            .map(|item| item.content.as_str())
            .collect()
    }

    pub fn into_texts(self) -> Vec<String> {
        self.items.into_iter().map(|item| item.content).collect()
    }
}

impl RecallItem {
    pub fn into_record(self, kind: MemoryKind, scope: MemoryScope) -> anyhow::Result<MemoryRecord> {
        let status = match self.temporal_state {
            TemporalState::Current | TemporalState::Unknown => MemoryStatus::Current,
            TemporalState::Superseded => MemoryStatus::Superseded,
            TemporalState::Expired => MemoryStatus::Expired,
        };
        let record = MemoryRecord {
            id: MemoryRecordId(self.metadata.record_id.clone()),
            kind,
            scope,
            content: self.content,
            metadata: self.metadata,
            status,
            authority: self.authority,
            source_event_ids: Vec::new(),
            tags: Vec::new(),
        };
        record.validate()?;
        Ok(record)
    }

    pub fn from_record(record: MemoryRecord) -> anyhow::Result<Self> {
        record.validate()?;
        let temporal_state = match record.status {
            MemoryStatus::Current | MemoryStatus::Candidate | MemoryStatus::Rejected => {
                record.metadata.temporal_state(None)
            }
            MemoryStatus::Superseded | MemoryStatus::Tombstoned => TemporalState::Superseded,
            MemoryStatus::Expired => TemporalState::Expired,
        };
        Ok(Self {
            content: record.content,
            metadata: record.metadata,
            temporal_state,
            authority: record.authority,
            scope: record.scope,
            score: 0.0,
            evidence: None,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForgetSelector {
    Exact {
        record_ids: Vec<MemoryRecordId>,
        within: MemoryScope,
    },
    Scope {
        scope: MemoryScope,
        limit: usize,
    },
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum ForgetAuthority {
    Ordinary,
    Elevated { proof: String },
}

/// Audited logical-deletion policy. Every selector is exact or explicitly bounded.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetPolicy {
    pub request_id: String,
    pub selector: ForgetSelector,
    pub requester: String,
    pub reason: String,
    pub authority: ForgetAuthority,
}

impl ForgetPolicy {
    pub const MAX_RECORDS: usize = 100;

    pub fn validate(&self) -> anyhow::Result<()> {
        anyhow::ensure!(
            !self.request_id.trim().is_empty(),
            "forget request ID is required"
        );
        anyhow::ensure!(
            !self.requester.trim().is_empty(),
            "forget requester is required"
        );
        anyhow::ensure!(!self.reason.trim().is_empty(), "forget reason is required");
        if let ForgetAuthority::Elevated { proof } = &self.authority {
            anyhow::ensure!(
                !proof.trim().is_empty(),
                "elevated forget proof is required"
            );
        }
        match &self.selector {
            ForgetSelector::Exact { record_ids, within } => {
                anyhow::ensure!(
                    !record_ids.is_empty() && record_ids.len() <= Self::MAX_RECORDS,
                    "exact forget selector is unbounded"
                );
                anyhow::ensure!(
                    record_ids.iter().all(|id| !id.0.trim().is_empty()),
                    "forget record ID is required"
                );
                within.validate()?;
            }
            ForgetSelector::Scope { scope, limit } => {
                anyhow::ensure!(
                    (1..=Self::MAX_RECORDS).contains(limit),
                    "scope forget selector is unbounded"
                );
                scope.validate()?;
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ForgetReceipt {
    pub tombstoned: Vec<MemoryRecordId>,
    pub already_tombstoned: Vec<MemoryRecordId>,
    pub denied: Vec<MemoryRecordId>,
    pub remote_pending: Vec<MemoryRecordId>,
}

impl ForgetReceipt {
    pub(crate) fn sort(&mut self) {
        for ids in [
            &mut self.tombstoned,
            &mut self.already_tombstoned,
            &mut self.denied,
            &mut self.remote_pending,
        ] {
            ids.sort_by(|left, right| left.0.cmp(&right.0));
            ids.dedup();
        }
    }
}

// ---------------------------------------------------------------------------
// Wave 3: Synthesis + Gap Analysis (supplemental memory "think" absorption)
// ---------------------------------------------------------------------------

/// A single inline citation linking back to a source record.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisCitation {
    pub index: usize,
    pub record_id: String,
    pub excerpt: String,
    #[serde(default)]
    pub evidence: Option<crate::recall::evidence::EvidenceLevel>,
}

/// A gap — something the memory system knows it doesn't know.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisGap {
    pub question: String,
    pub reason: String,
}

/// Synthesized answer with citations and gap analysis.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisResult {
    /// Markdown answer with inline citation markers.
    pub answer: String,
    pub citations: Vec<SynthesisCitation>,
    pub gaps: Vec<SynthesisGap>,
    pub confidence: f64,
}

/// Request to synthesize an answer from memory.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisRequest {
    pub question: String,
    pub session: String,
    /// Pre-fetched recall items (optional — if omitted, auto-recalled).
    #[serde(default)]
    pub pre_fetched: Option<Vec<RecallItem>>,
    #[serde(default = "default_max_citations")]
    pub max_citations: usize,
}

fn default_max_citations() -> usize {
    8
}

/// A block of context fed to the synthesis model.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SynthesisContextBlock {
    pub record_id: String,
    pub content: String,
    pub score: f32,
    pub evidence: Option<crate::recall::evidence::EvidenceLevel>,
}

/// Model trait for LLM-powered synthesis (feature-gated behind `llm-synthesis`).
#[cfg(feature = "llm-synthesis")]
#[async_trait]
pub trait SynthesisModel: Send + Sync {
    async fn synthesize(
        &self,
        question: &str,
        context: &[SynthesisContextBlock],
    ) -> anyhow::Result<SynthesisResult>;
}

/// Unified facade over the Mnemosyne memory objects.
#[async_trait]
pub trait MemoryService: Send + Sync {
    async fn record(&self, event: ExperienceEvent) -> anyhow::Result<()>;
    async fn recall(&self, req: RecallRequest) -> anyhow::Result<RecallSet>;
    async fn consolidate(&self, scope: MemoryScope) -> anyhow::Result<()>;
    async fn preview_forget(&self, _policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        anyhow::bail!("forget preview is unavailable")
    }
    async fn forget(&self, policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt>;

    /// Synthesize an answer from memory with citations and gap analysis.
    /// Default implementation returns an error (like `preview_forget`).
    async fn synthesize(&self, _request: SynthesisRequest) -> anyhow::Result<SynthesisResult> {
        anyhow::bail!("synthesis is unavailable")
    }

    /// Promote high-confidence consolidated facts into CoreMemory learned blocks.
    /// Returns the count of facts promoted. Default no-op.
    async fn promote_facts(
        &self,
        _min_confidence: f64,
        _max_count: usize,
    ) -> anyhow::Result<usize> {
        Ok(0)
    }
}

/// Default `MemoryService` implementation delegating to the real
/// SQLite-backed memory objects behind shared `Arc<Mutex<_>>` handles.
pub struct DefaultMemoryService {
    recall_memory: Arc<Mutex<RecallMemory>>,
    fact_store: Arc<Mutex<FactStore>>,
    #[allow(dead_code)]
    core_memory: Arc<Mutex<CoreMemory>>,
    episodic: Arc<Mutex<EpisodicMemory>>,
    clock: Arc<dyn fabric::Clock>,
    consolidation: Option<Arc<crate::consolidation::ConsolidationRepository>>,
    retention: Option<Arc<crate::retention::RetentionRepository>>,
    metrics: MemoryMetrics,
    hybrid_params: Option<crate::RecallSearchParams>,
    vector_search: Option<Arc<dyn crate::RecallSearchBackend>>,
    embedding_endpoint_trusted: bool,
    #[cfg(feature = "llm-synthesis")]
    synthesis_model: Option<Arc<dyn SynthesisModel>>,
    #[allow(dead_code)]
    knowledge_graph: Option<Arc<Mutex<crate::knowledge_graph::KnowledgeGraph>>>,
}

fn all_memory_authorities() -> Vec<MemoryAuthority> {
    vec![
        MemoryAuthority::ApprovedCore,
        MemoryAuthority::VerifiedLocalSemantic,
        MemoryAuthority::LocalEpisode,
        MemoryAuthority::AletheonExternal,
        MemoryAuthority::ExternalReference,
        MemoryAuthority::RawExperience,
    ]
}

struct LexicalSnapshotBackend {
    items: Vec<RecallItem>,
}

#[async_trait]
impl crate::RecallSearchBackend for LexicalSnapshotBackend {
    async fn search(
        &self,
        _request: &RecallRequest,
        predicate: &crate::ScopePredicate,
        top_k: usize,
    ) -> anyhow::Result<crate::SearchOutcome> {
        Ok(crate::SearchOutcome {
            items: self
                .items
                .iter()
                .take(top_k)
                .enumerate()
                .filter(|(_, item)| predicate.allows(item))
                .map(|(index, item)| crate::RankedRecallItem {
                    item: item.clone(),
                    score: 1.0 / (index + 1) as f32,
                })
                .collect(),
            index_stale: false,
        })
    }
}

impl DefaultMemoryService {
    pub fn new(
        recall_memory: Arc<Mutex<RecallMemory>>,
        fact_store: Arc<Mutex<FactStore>>,
        core_memory: Arc<Mutex<CoreMemory>>,
        episodic: Arc<Mutex<EpisodicMemory>>,
        clock: Arc<dyn fabric::Clock>,
    ) -> Self {
        Self {
            recall_memory,
            fact_store,
            core_memory,
            episodic,
            clock,
            consolidation: None,
            retention: None,
            metrics: MemoryMetrics::default(),
            hybrid_params: None,
            vector_search: None,
            embedding_endpoint_trusted: false,
            #[cfg(feature = "llm-synthesis")]
            synthesis_model: None,
            knowledge_graph: None,
        }
    }

    /// Single production composition switch. Disabled preserves the existing
    /// FTS-only call graph byte-for-byte; enabled adds the optional vector path.
    pub fn with_memory_hybrid(mut self, enabled: bool) -> Self {
        self.hybrid_params = enabled.then(|| crate::RecallSearchParams {
            vector_enabled: true,
            ..crate::RecallSearchParams::default()
        });
        self
    }

    pub fn with_vector_search_backend(
        mut self,
        backend: Arc<dyn crate::RecallSearchBackend>,
        grant: &crate::credential::EmbeddingCredentialGrant,
        endpoint_base_url: &str,
        now_unix: u64,
    ) -> Self {
        self.vector_search = Some(backend);
        // Trust is derived from the endpoint-scoped grant at composition time;
        // callers cannot assert it with an unaudited boolean. Keeping the
        // backend installed on rejection lets hybrid recall fail closed to FTS
        // while reporting EmbeddingEndpointUntrusted.
        self.embedding_endpoint_trusted = grant.approved_for(endpoint_base_url, now_unix);
        self
    }

    pub fn with_metrics(mut self, metrics: MemoryMetrics) -> Self {
        if let Some(repository) = &self.retention {
            repository.set_metrics(metrics.clone());
        }
        if let Some(repository) = &self.consolidation {
            repository.set_metrics(metrics.clone());
        }
        self.metrics = metrics;
        self
    }

    pub fn metrics(&self) -> &MemoryMetrics {
        &self.metrics
    }

    pub fn with_retention_repository(
        mut self,
        repository: Arc<crate::retention::RetentionRepository>,
    ) -> Self {
        repository.set_metrics(self.metrics.clone());
        self.retention = Some(repository);
        self
    }

    pub fn retention_repository(&self) -> Option<&Arc<crate::retention::RetentionRepository>> {
        self.retention.as_ref()
    }

    pub fn preview_forget(&self, policy: &ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        let repository = self
            .retention
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("retention repository is unavailable"))?;
        repository.preview_forget(policy, self.clock.wall_now().0.max(0))
    }

    pub fn with_consolidation_repository(
        mut self,
        repository: Arc<crate::consolidation::ConsolidationRepository>,
    ) -> Self {
        repository.set_metrics(self.metrics.clone());
        self.consolidation = Some(repository);
        self
    }

    fn enqueue_for_consolidation(
        &self,
        event_id: &str,
        kind: &str,
        content: &str,
        scope: &MemoryScope,
        completed: bool,
    ) -> anyhow::Result<()> {
        let Some(repository) = &self.consolidation else {
            return Ok(());
        };
        if !matches!(scope, MemoryScope::Session(_) | MemoryScope::Goal(_)) {
            return Ok(());
        }
        let now_ms = self.clock.wall_now().0.max(0) as u64;
        let (session_id, goal_id) = match scope {
            MemoryScope::Session(session) => (session.clone(), None),
            MemoryScope::Goal(goal) => (format!("goal:{goal}"), Some(goal.clone())),
            _ => (format!("scope:{}", serde_json::to_string(scope)?), None),
        };
        repository.enqueue_experience(
            &crate::consolidation::ExtractionJob {
                idempotency_key: format!("experience:{event_id}"),
                session_id: session_id.clone(),
                goal_id,
                ephemeral: session_id.starts_with("ephemeral:"),
                memory_worker: session_id.starts_with("memory-worker:"),
                completed_at_ms: completed.then_some(now_ms),
                watermark: event_id.to_owned(),
                created_at_ms: now_ms,
            },
            scope,
            &crate::consolidation::CanonicalMemoryEvent {
                event_id: event_id.to_owned(),
                kind: kind.to_owned(),
                content: content.to_owned(),
            },
        )?;
        Ok(())
    }

    fn advance_consolidation(&self, requested: &MemoryScope, now_ms: u64) -> anyhow::Result<()> {
        let Some(repository) = &self.consolidation else {
            return Ok(());
        };
        let owner = format!("mnemosyne-consolidation:{}", std::process::id());
        for _ in 0..32 {
            let Some(lease) =
                repository.claim_extraction(&owner, now_ms, 60_000, 30 * 24 * 60 * 60 * 1_000)?
            else {
                break;
            };
            let events = repository.extraction_events(&lease, 128)?;
            let completion = crate::consolidation::CandidateExtractor::default().extract(
                &crate::consolidation::ExtractionBatch {
                    scope: lease.scope.clone(),
                    events,
                },
            );
            match completion {
                Ok(completion) => repository.complete(&lease, completion, now_ms)?,
                Err(error) if lease.attempts < 3 => repository.complete(
                    &lease,
                    crate::consolidation::ExtractionCompletion::RetryableFailure {
                        error: error.to_string(),
                        retry_at_ms: now_ms.saturating_add(1_000 * (1 << lease.attempts)),
                    },
                    now_ms,
                )?,
                Err(error) => repository.complete(
                    &lease,
                    crate::consolidation::ExtractionCompletion::PermanentFailure {
                        error: error.to_string(),
                    },
                    now_ms,
                )?,
            }
        }

        let mut scopes = repository.pending_scopes(64)?;
        scopes.push(requested.clone());
        scopes.sort_by_key(|scope| serde_json::to_string(scope).unwrap_or_default());
        scopes.dedup();
        for scope in scopes {
            crate::consolidation::ScopedConsolidator::new(repository)
                .run(&scope, &owner, now_ms, None)?;
        }
        Ok(())
    }
    /// Recall through an explicit, already-verified authority filter. This is
    /// the production entry point for child Agent memory access.
    pub async fn recall_with_prefilter(
        &self,
        req: RecallRequest,
        prefilter: &crate::RecallPreFilter,
    ) -> anyhow::Result<RecallSet> {
        if let Err(error) = req.validate() {
            self.metrics
                .recall_omitted(RecallOmittedReason::InvalidRequest, 1);
            return Err(error);
        }
        let fetch_limit = req
            .max_items
            .saturating_mul(4)
            .min(RecallRequest::MAX_ITEMS);
        let predicate = prefilter.to_scope_predicate();
        let now = wall_to_datetime(self.clock.wall_now());
        let messages = async {
            let started = Instant::now();
            let result = self
                .recall_memory
                .lock()
                .await
                .search_in_session_prefiltered(&req.session, &req.query, fetch_limit, &predicate)
                .map(|rows| crate::recall::local::messages(rows, &req));
            (started.elapsed(), result)
        };
        let facts = async {
            let started = Instant::now();
            let result = self
                .fact_store
                .lock()
                .await
                .search_facts_prefiltered(&req.query, &req.session, 0.0, fetch_limit, &predicate)
                .map(|rows| crate::recall::local::facts(rows, &req, now));
            (started.elapsed(), result)
        };
        let reflections = async {
            let started = Instant::now();
            let result = if predicate.allows_scope(&MemoryScope::Session(req.session.clone()))
                && predicate.allows_authority(MemoryAuthority::LocalEpisode)
                && predicate.allows_sensitivity(MemorySensitivity::Internal)
            {
                self.episodic
                    .lock()
                    .await
                    .recall_reflections(fetch_limit)
                    .map(|rows| crate::recall::local::reflections(rows, &req))
            } else {
                Ok(Vec::new())
            };
            (started.elapsed(), result)
        };
        let core = async {
            let started = Instant::now();
            let blocks = if predicate.allows_scope(&MemoryScope::Global)
                && predicate.allows_authority(MemoryAuthority::ApprovedCore)
                && predicate.allows_sensitivity(MemorySensitivity::Internal)
            {
                self.core_memory
                    .lock()
                    .await
                    .blocks()
                    .iter()
                    .map(|(label, block)| (label.clone(), block.value.clone()))
                    .collect::<Vec<_>>()
            } else {
                Vec::new()
            };
            (
                started.elapsed(),
                Ok::<_, anyhow::Error>(crate::recall::local::core(blocks, &req, now)),
            )
        };
        let (messages, facts, reflections, core) = tokio::join!(messages, facts, reflections, core);
        let mut sources = Vec::with_capacity(4);
        let mut degraded_sources = Vec::new();
        for (name, source, kind, (elapsed, result)) in [
            (
                "recall_memory",
                RecallSourceLabel::RecallMemory,
                MemoryKind::Message,
                messages,
            ),
            (
                "fact_store",
                RecallSourceLabel::FactStore,
                MemoryKind::SemanticFact,
                facts,
            ),
            (
                "episodic",
                RecallSourceLabel::Episodic,
                MemoryKind::Reflection,
                reflections,
            ),
            ("core", RecallSourceLabel::Core, MemoryKind::CoreState, core),
        ] {
            self.metrics.observe_recall_latency(source, elapsed);
            match result {
                Ok(items) => {
                    self.metrics.recall_hit(source, kind, items.len());
                    sources.push(items);
                }
                Err(error) => {
                    tracing::warn!(source = name, %error, "local memory recall source degraded");
                    self.metrics
                        .recall_omitted(RecallOmittedReason::SourceDegraded, 1);
                    degraded_sources.push(name.to_string());
                }
            }
        }
        let mut items = crate::recall::merge_items(sources, &req, Some(&self.metrics));
        if let Some(retention) = &self.retention {
            let before = items.len();
            items.retain(|item| {
                !retention
                    .is_tombstoned(&item.metadata.record_id)
                    .unwrap_or(false)
            });
            self.metrics
                .recall_omitted(RecallOmittedReason::Tombstoned, before - items.len());
            self.metrics.set_tombstone_pending(
                TombstoneDestination::Supplemental,
                retention.pending_remote_count().unwrap_or_default(),
            );
        }
        Ok(RecallSet {
            items,
            degraded_sources,
        })
    }

    /// Derive authority from a server-verified child context. Query text cannot
    /// widen the resulting Agent/Task ancestry.
    pub async fn recall_for_agent(
        &self,
        context: &crate::AgentMemoryContext,
        req: RecallRequest,
        max_sensitivity: MemorySensitivity,
    ) -> anyhow::Result<RecallSet> {
        let prefilter = crate::RecallPreFilter {
            ancestry: context.recall_ancestry()?,
            max_sensitivity,
            allowed_authorities: all_memory_authorities(),
        };
        self.recall_with_prefilter(req, &prefilter).await
    }
}

#[async_trait]
impl MemoryService for DefaultMemoryService {
    async fn record(&self, event: ExperienceEvent) -> anyhow::Result<()> {
        match event {
            ExperienceEvent::Message {
                session,
                role,
                content,
                metadata,
            } => {
                metadata.validate()?;
                let entry_type = match role.as_str() {
                    "user" => "user_message",
                    "assistant" => "assistant_message",
                    other => other,
                };
                let rm = self.recall_memory.lock().await;
                rm.store(
                    &session,
                    entry_type,
                    &content,
                    Some(&serde_json::to_string(&metadata)?),
                )?;
                drop(rm);
                let scope = MemoryScope::Session(session.clone());
                self.enqueue_for_consolidation(
                    &metadata.record_id,
                    entry_type,
                    &content,
                    &scope,
                    false,
                )?;
                self.maybe_extract_to_kg(&content, &metadata.record_id)
                    .await;
                if let Some(retention) = &self.retention {
                    let record = MemoryRecord {
                        id: MemoryRecordId(metadata.record_id.clone()),
                        kind: MemoryKind::Message,
                        scope: scope.clone(),
                        content,
                        metadata,
                        status: MemoryStatus::Current,
                        authority: MemoryAuthority::RawExperience,
                        source_event_ids: Vec::new(),
                        tags: Vec::new(),
                    };
                    retention.register(&record, self.clock.wall_now().0.max(0))?;
                }
                self.metrics.record_stored(MemoryKind::Message, &scope);
                Ok(())
            }
            event @ (ExperienceEvent::Reflection { .. }
            | ExperienceEvent::ArchitectureDecision { .. }
            | ExperienceEvent::GoalOutcome { .. }) => {
                let (content, metadata, kind, scope, completed) = match event {
                    ExperienceEvent::Reflection { content, metadata } => {
                        let scope = metadata
                            .provenance
                            .principal
                            .clone()
                            .map(MemoryScope::Principal)
                            .unwrap_or(MemoryScope::Global);
                        (content, metadata, MemoryKind::Reflection, scope, false)
                    }
                    ExperienceEvent::ArchitectureDecision {
                        content, metadata, ..
                    } => (
                        content,
                        metadata,
                        MemoryKind::ArchitectureDecision,
                        MemoryScope::Global,
                        false,
                    ),
                    ExperienceEvent::GoalOutcome {
                        goal_id,
                        content,
                        metadata,
                        ..
                    } => (
                        content,
                        metadata,
                        MemoryKind::GoalOutcome,
                        MemoryScope::Goal(goal_id),
                        true,
                    ),
                    ExperienceEvent::Message { .. } => unreachable!(),
                };
                metadata.validate()?;
                let entry = ReflectionEntry {
                    id: metadata.record_id.clone(),
                    timestamp: wall_to_datetime(self.clock.wall_now()),
                    trigger: ReflectionTrigger::Manual,
                    task_summary: content.clone(),
                    outcome: ReflectionOutcome::Success,
                    what_worked: Vec::new(),
                    what_failed: Vec::new(),
                    learned: vec![content.clone()],
                    behavior_changes: Vec::new(),
                    confidence: 0.5,
                };
                let episodic = self.episodic.lock().await;
                episodic.store_reflection(&entry)?;
                drop(episodic);
                let extraction_kind = match kind {
                    MemoryKind::ArchitectureDecision => "architecture_decision",
                    MemoryKind::GoalOutcome => "goal_outcome",
                    _ => "reflection",
                };
                self.enqueue_for_consolidation(
                    &metadata.record_id,
                    extraction_kind,
                    &content,
                    &scope,
                    completed,
                )?;
                // Grab record_id before content/metadata move into MemoryRecord
                let record_id = metadata.record_id.clone();
                self.maybe_extract_to_kg(&content, &record_id).await;
                if let Some(retention) = &self.retention {
                    retention.register(
                        &MemoryRecord {
                            id: MemoryRecordId(record_id),
                            kind,
                            scope: scope.clone(),
                            content,
                            metadata,
                            status: MemoryStatus::Current,
                            authority: MemoryAuthority::LocalEpisode,
                            source_event_ids: Vec::new(),
                            tags: Vec::new(),
                        },
                        self.clock.wall_now().0.max(0),
                    )?;
                }
                self.metrics.record_stored(kind, &scope);
                Ok(())
            }
        }
    }

    async fn recall(&self, req: RecallRequest) -> anyhow::Result<RecallSet> {
        let prefilter = crate::RecallPreFilter {
            ancestry: crate::ScopeAncestry {
                session_id: Some(req.session.clone()),
                ..Default::default()
            },
            max_sensitivity: MemorySensitivity::Restricted,
            allowed_authorities: all_memory_authorities(),
        };
        let lexical = self.recall_with_prefilter(req.clone(), &prefilter).await?;
        let Some(params) = &self.hybrid_params else {
            self.metrics.recall_fts_only();
            return Ok(lexical);
        };
        let lexical_backend = LexicalSnapshotBackend {
            items: lexical.items,
        };
        let (items, degraded) = crate::recall::pipeline::hybrid_recall_with_metrics(
            &prefilter,
            params,
            crate::HybridRecallBackends {
                fts: Some(&lexical_backend),
                vector: self.vector_search.as_deref(),
                embedding_endpoint_trusted: self.embedding_endpoint_trusted,
            },
            &req,
            Some(&self.metrics),
        )
        .await;
        let mut degraded_sources = lexical.degraded_sources;
        degraded_sources.extend(degraded.iter().map(|source| source.as_str().to_string()));
        let vector_used = self.vector_search.is_some()
            && !degraded.contains(&crate::DegradedSource::EmbeddingEndpointUntrusted)
            && !degraded.contains(&crate::DegradedSource::EmbeddingTimeout);
        if vector_used {
            self.metrics.recall_vector_used();
        }
        if degraded.is_empty() {
            tracing::info!(
                event = "memory.recall.vector_used",
                "hybrid memory recall completed"
            );
        } else {
            if degraded.contains(&crate::DegradedSource::EmbeddingEndpointUntrusted) {
                self.metrics.embedding_credential_rejected();
            }
            self.metrics.recall_fts_only();
            tracing::warn!(event = "memory.recall.degraded", degraded_sources = ?degraded, "hybrid memory recall degraded");
        }
        Ok(RecallSet {
            items,
            degraded_sources,
        })
    }

    async fn consolidate(&self, scope: MemoryScope) -> anyhow::Result<()> {
        let mut lifecycle = crate::lifecycle::MemoryOperationLifecycle::default();
        lifecycle.apply(crate::lifecycle::MemoryOperationEvent::BeginReconciliation)?;
        let result: anyhow::Result<()> = async {
            let now_ms = self.clock.wall_now().0.max(0) as u64;
            if let Some(repository) = &self.consolidation {
                if matches!(scope, MemoryScope::Session(_) | MemoryScope::Goal(_)) {
                    // A scoped consolidate request is the existing contract's explicit
                    // lifecycle boundary. Periodic Global consolidation cannot infer that
                    // an active Session or Goal has completed.
                    repository.complete_scope(&scope, now_ms)?;
                }
            }
            self.advance_consolidation(&scope, now_ms)?;
            self.fact_store.lock().await.decay_stale()?;
            Ok(())
        }
        .await;
        match result {
            Ok(()) => {
                lifecycle.apply(crate::lifecycle::MemoryOperationEvent::ReconciliationFinished)?;
                Ok(())
            }
            Err(error) => {
                lifecycle.apply(crate::lifecycle::MemoryOperationEvent::Fail)?;
                Err(error)
            }
        }
    }

    async fn preview_forget(&self, policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        DefaultMemoryService::preview_forget(self, &policy)
    }

    async fn forget(&self, policy: ForgetPolicy) -> anyhow::Result<ForgetReceipt> {
        let mut lifecycle = crate::lifecycle::MemoryOperationLifecycle::default();
        lifecycle.apply(crate::lifecycle::MemoryOperationEvent::BeginRetention)?;
        let repository = self
            .retention
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("retention repository is unavailable"));
        let result = match repository {
            Ok(repository) => repository.forget(&policy, self.clock.wall_now().0.max(0)),
            Err(error) => Err(error),
        };
        match result {
            Ok(receipt) => {
                lifecycle.apply(crate::lifecycle::MemoryOperationEvent::RetentionFinished)?;
                Ok(receipt)
            }
            Err(error) => {
                lifecycle.apply(crate::lifecycle::MemoryOperationEvent::Fail)?;
                Err(error)
            }
        }
    }

    async fn synthesize(&self, request: SynthesisRequest) -> anyhow::Result<SynthesisResult> {
        self.synthesize(request).await
    }

    async fn promote_facts(&self, min_confidence: f64, max_count: usize) -> anyhow::Result<usize> {
        let Some(repository) = &self.consolidation else {
            return Ok(0);
        };
        let records =
            repository.high_confidence_records(&MemoryScope::Global, min_confidence, max_count)?;
        let count = records.len();
        if count == 0 {
            self.core_memory
                .lock()
                .await
                .retain_promoted_facts(&std::collections::HashSet::new());
            return Ok(0);
        }
        use sha2::{Digest, Sha256};
        let mut core = self.core_memory.lock().await;
        let mut promoted_labels = std::collections::HashSet::with_capacity(count);
        for (content, kind_json, confidence) in records {
            let mut hasher = Sha256::new();
            hasher.update(content.as_bytes());
            let hash_hex = format!("{:x}", hasher.finalize());
            let kind: &str = &kind_json;
            let label = format!("fact:{kind}:{}", &hash_hex[..16]);
            promoted_labels.insert(label.clone());
            let value = format!("[{kind}] confidence={confidence:.2}\n{content}");
            let truncated: String = value.chars().take(2500).collect();
            core.set_block(MemoryBlock::new(label, truncated, 3000));
        }
        core.retain_promoted_facts(&promoted_labels);
        Ok(count)
    }
}

impl DefaultMemoryService {
    /// Attach a knowledge graph for zero-LLM entity extraction during `record`.
    pub fn with_knowledge_graph(mut self, kg: crate::knowledge_graph::KnowledgeGraph) -> Self {
        self.knowledge_graph = Some(Arc::new(Mutex::new(kg)));
        self
    }

    /// Fire-and-forget entity/relation extraction into the knowledge graph.
    /// Failures are logged and swallowed (fail-open, matching supplemental memory).
    async fn maybe_extract_to_kg(&self, content: &str, record_id: &str) {
        let Some(kg) = &self.knowledge_graph else {
            return;
        };
        let mut kg = kg.lock().await;
        let entities = crate::knowledge_graph::extract_entities_from_content(content, record_id);
        let relations = crate::knowledge_graph::infer_relations(content, &entities, record_id);
        for entity in entities {
            kg.upsert_entity(entity);
        }
        for relation in relations {
            kg.add_relation(relation);
        }
    }

    /// Attach a synthesis model for LLM-powered `synthesize()`.
    #[cfg(feature = "llm-synthesis")]
    pub fn with_synthesis_model(mut self, model: Arc<dyn SynthesisModel>) -> Self {
        self.synthesis_model = Some(model);
        self
    }

    /// Synthesize an answer from memory with citations and gap analysis.
    #[cfg(feature = "llm-synthesis")]
    async fn synthesize(&self, request: SynthesisRequest) -> anyhow::Result<SynthesisResult> {
        let model = self
            .synthesis_model
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("synthesis model is unavailable"))?;
        let items = match request.pre_fetched {
            Some(items) => items,
            None => {
                let recall_req = RecallRequest {
                    session: request.session.clone(),
                    query: request.question.clone(),
                    max_items: request.max_citations * 4,
                    max_content_bytes: 128 * 1024,
                    current_at: None,
                    include_historical: false,
                    mode: None,
                };
                self.recall(recall_req).await?.items
            }
        };
        let context: Vec<SynthesisContextBlock> = items
            .into_iter()
            .take(request.max_citations * 2)
            .map(|item| SynthesisContextBlock {
                record_id: item.metadata.record_id.clone(),
                content: item.content,
                score: item.score,
                evidence: item.evidence,
            })
            .collect();
        model.synthesize(&request.question, &context).await
    }

    /// Fallback when `llm-synthesis` feature is disabled.
    #[cfg(not(feature = "llm-synthesis"))]
    async fn synthesize(&self, _request: SynthesisRequest) -> anyhow::Result<SynthesisResult> {
        anyhow::bail!("synthesis is unavailable (enable the llm-synthesis feature)")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{Subsystem, SubsystemContext};
    use std::path::Path;

    struct EmptyVectorBackend;

    #[async_trait::async_trait]
    impl crate::RecallSearchBackend for EmptyVectorBackend {
        async fn search(
            &self,
            _request: &RecallRequest,
            _predicate: &crate::ScopePredicate,
            _top_k: usize,
        ) -> anyhow::Result<crate::SearchOutcome> {
            Ok(crate::SearchOutcome::default())
        }
    }

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(kernel::chronos::TestClock::default())
    }

    fn metadata(id: &str) -> MemoryMetadata {
        MemoryMetadata::local(id, id, DateTime::<Utc>::UNIX_EPOCH)
    }

    async fn build_service(dir: &Path) -> DefaultMemoryService {
        let clock = test_clock();
        let recall_memory = Arc::new(Mutex::new(
            RecallMemory::new(&dir.join("recall.db"), clock.clone()).unwrap(),
        ));
        let fact_store = Arc::new(Mutex::new(FactStore::open(&dir.join("facts.db")).unwrap()));
        let core_memory = Arc::new(Mutex::new(CoreMemory::new()));
        let mut episodic_memory = EpisodicMemory::new(dir.join("episodic.db"), clock.clone());
        let ctx = SubsystemContext {
            name: "episodic_memory".into(),
            working_dir: dir.to_path_buf(),
            config: serde_json::Value::Null,
            bus: None,
        };
        episodic_memory.init(&ctx).await.unwrap();
        let episodic = Arc::new(Mutex::new(episodic_memory));
        DefaultMemoryService::new(recall_memory, fact_store, core_memory, episodic, clock)
    }

    #[tokio::test]
    async fn vector_backend_trust_is_derived_from_exact_endpoint_grant() {
        let grant = crate::credential::EmbeddingCredentialGrant::new(
            "test-principal",
            "https://embedding.example/v1",
            "test-provider",
            100,
            1,
            "secret",
        );
        let dir = tempfile::tempdir().unwrap();
        let rejected = build_service(dir.path()).await.with_vector_search_backend(
            Arc::new(EmptyVectorBackend),
            &grant,
            "https://embedding.example.evil/v1",
            10,
        );
        assert!(!rejected.embedding_endpoint_trusted);

        let dir = tempfile::tempdir().unwrap();
        let approved = build_service(dir.path()).await.with_vector_search_backend(
            Arc::new(EmptyVectorBackend),
            &grant,
            "https://embedding.example/v1",
            10,
        );
        assert!(approved.embedding_endpoint_trusted);
    }

    #[tokio::test]
    async fn record_message_stores_into_recall_memory() {
        let dir = tempfile::tempdir().unwrap();
        let svc = build_service(dir.path()).await;
        svc.record(ExperienceEvent::Message {
            session: "s1".into(),
            role: "user".into(),
            content: "hello world".into(),
            metadata: metadata("message-1"),
        })
        .await
        .unwrap();

        let rm = svc.recall_memory.lock().await;
        let count = rm.count().unwrap();
        assert_eq!(count, 1);
    }

    #[tokio::test]
    async fn record_reflection_stores_into_episodic_memory() {
        let dir = tempfile::tempdir().unwrap();
        let svc = build_service(dir.path()).await;
        svc.record(ExperienceEvent::Reflection {
            content: "learned something".into(),
            metadata: metadata("reflection-1"),
        })
        .await
        .unwrap();

        let episodic = svc.episodic.lock().await;
        assert_eq!(episodic.reflection_count().unwrap(), 1);
    }

    #[tokio::test]
    async fn recall_returns_matching_facts() {
        let dir = tempfile::tempdir().unwrap();
        let svc = build_service(dir.path()).await;
        {
            let fact_store = svc.fact_store.lock().await;
            fact_store
                .add_fact("rust is great", "general", "", "test", 0.5, "long", 0)
                .unwrap();
        }

        let result = svc
            .recall(RecallRequest::bounded("s1", "rust"))
            .await
            .unwrap();
        assert_eq!(result.texts(), vec!["rust is great"]);
        let item = &result.items[0];
        assert_eq!(item.metadata.record_id, "mnemosyne:fact:1");
        assert_eq!(item.metadata.provenance.source_id, "1");
        assert_eq!(item.metadata.confidence, 0.5);
        assert_eq!(item.metadata.sensitivity, MemorySensitivity::Internal);
        assert_eq!(item.temporal_state, TemporalState::Unknown);
    }

    #[tokio::test]
    async fn memory_hybrid_flag_off_is_fts_path_equivalent() {
        let dir = tempfile::tempdir().unwrap();
        let svc = build_service(dir.path()).await;
        {
            let fact_store = svc.fact_store.lock().await;
            fact_store
                .add_fact("stable lexical", "general", "", "test", 0.5, "long", 0)
                .unwrap();
        }
        let request = RecallRequest::bounded("s1", "stable");
        let prefilter = crate::RecallPreFilter {
            ancestry: crate::ScopeAncestry {
                session_id: Some("s1".into()),
                ..Default::default()
            },
            max_sensitivity: MemorySensitivity::Restricted,
            allowed_authorities: all_memory_authorities(),
        };
        let legacy = svc
            .recall_with_prefilter(request.clone(), &prefilter)
            .await
            .unwrap();
        let flagged_off = svc.recall(request).await.unwrap();
        assert_eq!(flagged_off, legacy);
    }

    #[tokio::test]
    async fn verified_agent_recall_cannot_widen_scope_from_request_or_query() {
        use fabric::{AgentId, AgentTaskId, ProcessId};
        use uuid::Uuid;

        let dir = tempfile::tempdir().unwrap();
        let svc = build_service(dir.path()).await;
        svc.record(ExperienceEvent::Message {
            session: "parent-session".into(),
            role: "user".into(),
            content: "parent-only marker".into(),
            metadata: metadata("parent-message"),
        })
        .await
        .unwrap();
        let context = crate::AgentMemoryContext::verified(
            ProcessId(Uuid::new_v4()),
            AgentId(Uuid::new_v4()),
            AgentTaskId("child-task".into()),
            "verified-parent-projection",
        )
        .unwrap();
        let result = svc
            .recall_for_agent(
                &context,
                RecallRequest::bounded(
                    "parent-session",
                    "parent-only marker global parent session",
                ),
                MemorySensitivity::Internal,
            )
            .await
            .unwrap();
        assert!(result.items.is_empty());
    }

    #[tokio::test]
    async fn governed_fact_recall_materializes_only_allowed_principal_candidates() {
        let dir = tempfile::tempdir().unwrap();
        let svc = build_service(dir.path()).await;
        {
            let facts = svc.fact_store.lock().await;
            facts
                .add_fact_governed(
                    "shared principal marker allowed",
                    "general",
                    "",
                    "principal",
                    "explicit",
                    "owner-a",
                    0.8,
                    "long",
                    0,
                )
                .unwrap();
            facts
                .add_fact_governed(
                    "shared principal marker denied",
                    "general",
                    "",
                    "principal",
                    "explicit",
                    "owner-b",
                    0.8,
                    "long",
                    0,
                )
                .unwrap();
        }
        let prefilter = crate::RecallPreFilter {
            ancestry: crate::ScopeAncestry {
                principal_id: Some("owner-a".into()),
                ..Default::default()
            },
            max_sensitivity: MemorySensitivity::Internal,
            allowed_authorities: vec![MemoryAuthority::VerifiedLocalSemantic],
        };
        let result = svc
            .recall_with_prefilter(
                RecallRequest::bounded("untrusted-session", "shared principal marker"),
                &prefilter,
            )
            .await
            .unwrap();
        assert_eq!(result.texts(), vec!["shared principal marker allowed"]);
    }

    #[test]
    fn temporal_state_uses_explicit_validity_and_supersession() {
        let now = DateTime::<Utc>::UNIX_EPOCH;
        let mut value = metadata("decision-1");
        assert_eq!(value.temporal_state(Some(now)), TemporalState::Current);
        value.valid_until = Some(now);
        assert_eq!(value.temporal_state(Some(now)), TemporalState::Expired);
        value.superseded_by = Some("decision-2".into());
        assert_eq!(value.temporal_state(Some(now)), TemporalState::Superseded);
        value.superseded_by = None;
        value.valid_until = None;
        assert_eq!(value.temporal_state(None), TemporalState::Unknown);
    }

    #[test]
    fn metadata_round_trip_preserves_contract_fields() {
        let now = DateTime::<Utc>::UNIX_EPOCH;
        let value = MemoryMetadata {
            record_id: "goal:g1:outcome".into(),
            provenance: MemoryProvenance {
                source: "goal_store".into(),
                source_id: "g1".into(),
                principal: Some("owner".into()),
                source_commit: Some("abc123".into()),
            },
            source_time: Some(now),
            observed_time: now,
            valid_from: Some(now),
            valid_until: Some(now + chrono::Duration::days(1)),
            supersedes: Some("goal:g0:outcome".into()),
            superseded_by: None,
            confidence: 0.9,
            sensitivity: MemorySensitivity::Confidential,
        };
        let encoded = serde_json::to_string(&value).unwrap();
        assert_eq!(
            serde_json::from_str::<MemoryMetadata>(&encoded).unwrap(),
            value
        );
    }

    #[tokio::test]
    async fn recall_rejects_unbounded_requests() {
        let dir = tempfile::tempdir().unwrap();
        let svc = build_service(dir.path()).await;
        let mut req = RecallRequest::bounded("s1", "rust");
        req.max_items = RecallRequest::MAX_ITEMS + 1;
        assert!(svc.recall(req).await.is_err());
    }

    #[tokio::test]
    async fn consolidate_is_ok_and_forget_fails_without_retention() {
        let dir = tempfile::tempdir().unwrap();
        let svc = build_service(dir.path()).await;
        svc.consolidate(MemoryScope::Global).await.unwrap();
        svc.consolidate(MemoryScope::Session("s1".into()))
            .await
            .unwrap();
        assert!(svc
            .forget(ForgetPolicy {
                request_id: "request-1".into(),
                selector: ForgetSelector::Scope {
                    scope: MemoryScope::Session("s1".into()),
                    limit: 1,
                },
                requester: "owner".into(),
                reason: "test".into(),
                authority: ForgetAuthority::Ordinary,
            })
            .await
            .is_err());
    }
}
