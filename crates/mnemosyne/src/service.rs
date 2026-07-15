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

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fabric::{self, wall_to_datetime, ReflectionEntry, ReflectionOutcome, ReflectionTrigger};
use serde::{Deserialize, Serialize};
use tokio::sync::Mutex;

pub use crate::model::{
    MemoryAuthority, MemoryMetadata, MemoryProvenance, MemoryScope, MemorySensitivity,
    TemporalState,
};
use crate::model::{MemoryKind, MemoryRecord, MemoryRecordId, MemoryStatus};
use crate::{CoreMemory, EpisodicMemory, FactStore, RecallMemory};

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
}

/// Result of a recall query.
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct RecallSet {
    pub items: Vec<RecallItem>,
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
        })
    }
}

/// Conservative forget policy. `session: None` means "no-op" (see `forget`).
#[derive(Debug, Clone, Default)]
pub struct ForgetPolicy {
    pub session: Option<String>,
}

/// Unified facade over the Mnemosyne memory objects.
#[async_trait]
pub trait MemoryService: Send + Sync {
    async fn record(&self, event: ExperienceEvent) -> anyhow::Result<()>;
    async fn recall(&self, req: RecallRequest) -> anyhow::Result<RecallSet>;
    async fn consolidate(&self, scope: MemoryScope) -> anyhow::Result<()>;
    async fn forget(&self, policy: ForgetPolicy) -> anyhow::Result<()>;
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
        }
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
                rm.store(&session, entry_type, &content, None)?;
                Ok(())
            }
            ExperienceEvent::Reflection { content, metadata }
            | ExperienceEvent::ArchitectureDecision {
                content, metadata, ..
            }
            | ExperienceEvent::GoalOutcome {
                content, metadata, ..
            } => {
                metadata.validate()?;
                let entry = ReflectionEntry {
                    id: metadata.record_id,
                    timestamp: wall_to_datetime(self.clock.wall_now()),
                    trigger: ReflectionTrigger::Manual,
                    task_summary: content.clone(),
                    outcome: ReflectionOutcome::Success,
                    what_worked: Vec::new(),
                    what_failed: Vec::new(),
                    learned: vec![content],
                    behavior_changes: Vec::new(),
                    confidence: 0.5,
                };
                let episodic = self.episodic.lock().await;
                episodic.store_reflection(&entry)?;
                Ok(())
            }
        }
    }

    async fn recall(&self, req: RecallRequest) -> anyhow::Result<RecallSet> {
        req.validate()?;
        let fetch_limit = req
            .max_items
            .saturating_mul(4)
            .min(RecallRequest::MAX_ITEMS);
        let now = wall_to_datetime(self.clock.wall_now());
        let messages = async {
            self.recall_memory
                .lock()
                .await
                .search_in_session(&req.session, &req.query, fetch_limit)
                .map(|rows| crate::recall::local::messages(rows, &req))
        };
        let facts = async {
            self.fact_store
                .lock()
                .await
                .search_facts(&req.query, None, 0.0, fetch_limit)
                .map(|rows| crate::recall::local::facts(rows, &req, now))
        };
        let reflections = async {
            self.episodic
                .lock()
                .await
                .recall_reflections(fetch_limit)
                .map(|rows| crate::recall::local::reflections(rows, &req))
        };
        let core = async {
            let blocks = self
                .core_memory
                .lock()
                .await
                .blocks()
                .iter()
                .map(|(label, block)| (label.clone(), block.value.clone()))
                .collect::<Vec<_>>();
            Ok::<_, anyhow::Error>(crate::recall::local::core(blocks, &req, now))
        };
        let (messages, facts, reflections, core) = tokio::join!(messages, facts, reflections, core);
        let mut sources = Vec::with_capacity(4);
        for (name, result) in [
            ("recall_memory", messages),
            ("fact_store", facts),
            ("episodic", reflections),
            ("core", core),
        ] {
            match result {
                Ok(items) => sources.push(items),
                Err(error) => {
                    tracing::warn!(source = name, %error, "local memory recall source degraded")
                }
            }
        }
        Ok(RecallSet {
            items: crate::recall::merge_items(sources, &req),
        })
    }

    async fn consolidate(&self, scope: MemoryScope) -> anyhow::Result<()> {
        // Facts are not session-scoped, so scoped and Global calls behave the
        // same for now: decay stale facts.
        let _ = scope;
        let fact_store = self.fact_store.lock().await;
        fact_store.decay_stale()?;
        Ok(())
    }

    async fn forget(&self, policy: ForgetPolicy) -> anyhow::Result<()> {
        // No destructive delete-by-session method exists on RecallMemory or
        // FactStore today; implementing one is out of scope for this
        // additive facade. Conservatively no-op and document the gap.
        let _ = policy;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::{Subsystem, SubsystemContext};
    use std::path::Path;

    fn test_clock() -> Arc<dyn fabric::Clock> {
        Arc::new(aletheon_kernel::chronos::TestClock::default())
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
    async fn consolidate_and_forget_are_ok() {
        let dir = tempfile::tempdir().unwrap();
        let svc = build_service(dir.path()).await;
        svc.consolidate(MemoryScope::Global).await.unwrap();
        svc.consolidate(MemoryScope::Session("s1".into()))
            .await
            .unwrap();
        svc.forget(ForgetPolicy {
            session: Some("s1".into()),
        })
        .await
        .unwrap();
        svc.forget(ForgetPolicy::default()).await.unwrap();
    }
}
