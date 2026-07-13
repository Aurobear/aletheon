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
use chrono::Utc;
use fabric::{ReflectionEntry, ReflectionOutcome, ReflectionTrigger};
use tokio::sync::Mutex;
use uuid::Uuid;

use crate::{CoreMemory, EpisodicMemory, FactStore, RecallMemory};

/// A unit of experience to be recorded into memory.
#[derive(Debug, Clone)]
pub enum ExperienceEvent {
    /// A conversational turn (user or assistant message).
    Message {
        /// Logical session this message belongs to.
        session: String,
        /// "user" | "assistant" | other role label.
        role: String,
        content: String,
    },
    /// A reflection summary (e.g. produced after a task).
    Reflection { content: String },
}

/// A recall query.
#[derive(Debug, Clone)]
pub struct RecallRequest {
    pub session: String,
    pub query: String,
}

/// Result of a recall query.
#[derive(Debug, Clone, Default)]
pub struct RecallSet {
    pub facts: Vec<String>,
}

/// Facade-local consolidation/forget scope. See module docs for why this is
/// not re-exported at the crate root.
#[derive(Debug, Clone)]
pub enum MemoryScope {
    All,
    Session(String),
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
}

impl DefaultMemoryService {
    pub fn new(
        recall_memory: Arc<Mutex<RecallMemory>>,
        fact_store: Arc<Mutex<FactStore>>,
        core_memory: Arc<Mutex<CoreMemory>>,
        episodic: Arc<Mutex<EpisodicMemory>>,
    ) -> Self {
        Self {
            recall_memory,
            fact_store,
            core_memory,
            episodic,
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
            } => {
                let entry_type = match role.as_str() {
                    "user" => "user_message",
                    "assistant" => "assistant_message",
                    other => other,
                };
                let rm = self.recall_memory.lock().await;
                rm.store(&session, entry_type, &content, None)?;
                Ok(())
            }
            ExperienceEvent::Reflection { content } => {
                let entry = ReflectionEntry {
                    id: Uuid::new_v4().to_string(),
                    timestamp: Utc::now(),
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
        let _ = req.session; // facts are not session-scoped today.
        let fact_store = self.fact_store.lock().await;
        let rows = fact_store.search_facts(&req.query, None, 0.0, 20)?;
        Ok(RecallSet {
            facts: rows.into_iter().map(|row| row.content).collect(),
        })
    }

    async fn consolidate(&self, scope: MemoryScope) -> anyhow::Result<()> {
        // Facts are not session-scoped, so `Session(_)` and `All` behave the
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

    async fn build_service(dir: &Path) -> DefaultMemoryService {
        let recall_memory = Arc::new(Mutex::new(
            RecallMemory::new(&dir.join("recall.db"), test_clock()).unwrap(),
        ));
        let fact_store = Arc::new(Mutex::new(FactStore::open(&dir.join("facts.db")).unwrap()));
        let core_memory = Arc::new(Mutex::new(CoreMemory::new()));
        let mut episodic_memory = EpisodicMemory::new(dir.join("episodic.db"), test_clock());
        let ctx = SubsystemContext {
            name: "episodic_memory".into(),
            working_dir: dir.to_path_buf(),
            config: serde_json::Value::Null,
            bus: None,
        };
        episodic_memory.init(&ctx).await.unwrap();
        let episodic = Arc::new(Mutex::new(episodic_memory));
        DefaultMemoryService::new(recall_memory, fact_store, core_memory, episodic)
    }

    #[tokio::test]
    async fn record_message_stores_into_recall_memory() {
        let dir = tempfile::tempdir().unwrap();
        let svc = build_service(dir.path()).await;
        svc.record(ExperienceEvent::Message {
            session: "s1".into(),
            role: "user".into(),
            content: "hello world".into(),
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
            .recall(RecallRequest {
                session: "s1".into(),
                query: "rust".into(),
            })
            .await
            .unwrap();
        assert_eq!(result.facts, vec!["rust is great".to_string()]);
    }

    #[tokio::test]
    async fn consolidate_and_forget_are_ok() {
        let dir = tempfile::tempdir().unwrap();
        let svc = build_service(dir.path()).await;
        svc.consolidate(MemoryScope::All).await.unwrap();
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
