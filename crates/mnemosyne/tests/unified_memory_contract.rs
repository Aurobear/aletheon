use std::path::{Path, PathBuf};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use fabric::{Clock, Subsystem, SubsystemContext};
use mnemosyne::backends::gbrain::{
    EnqueueOutcome, GbrainBackendError, SupplementalErrorCategory, SupplementalRecall,
    SupplementalRecallHealth,
};
use mnemosyne::{
    CompositeMemoryService, CoreMemory, DefaultMemoryService, EpisodicMemory, ExperienceEvent,
    FactStore, ForgetPolicy, MemoryBlock, MemoryKindLabel, MemoryMetadata, MemoryScopeLabel,
    MemoryService, RecallMemory, RecallRequest, RecallSourceLabel, SupplementalMemoryService,
};
use rusqlite::Connection;
use serde_json::Value;
use tokio::sync::Mutex;
use tokio_util::sync::CancellationToken;

struct Fixture {
    root: PathBuf,
    core: Arc<Mutex<CoreMemory>>,
    service: DefaultMemoryService,
}

impl Fixture {
    async fn open(root: &Path) -> Self {
        let clock = test_clock();
        let recall = Arc::new(Mutex::new(
            RecallMemory::new(&root.join("recall.db"), clock.clone()).unwrap(),
        ));
        let facts = Arc::new(Mutex::new(FactStore::open(&root.join("facts.db")).unwrap()));
        let core = Arc::new(Mutex::new(CoreMemory::new()));
        let mut episodic = EpisodicMemory::new(root.join("episodic.db"), clock.clone());
        episodic
            .init(&SubsystemContext {
                name: "unified-memory-contract".into(),
                working_dir: root.to_path_buf(),
                config: Value::Null,
                bus: None,
            })
            .await
            .unwrap();
        Self {
            root: root.to_path_buf(),
            core: core.clone(),
            service: DefaultMemoryService::new(
                recall,
                facts,
                core,
                Arc::new(Mutex::new(episodic)),
                clock,
            ),
        }
    }

    async fn episodic_store(&self) -> EpisodicMemory {
        let mut store = EpisodicMemory::new(self.root.join("episodic.db"), test_clock());
        store
            .init(&SubsystemContext {
                name: "unified-memory-contract-reopen".into(),
                working_dir: self.root.clone(),
                config: Value::Null,
                bus: None,
            })
            .await
            .unwrap();
        store
    }
}

#[tokio::test]
async fn message_recall_does_not_leak_across_sessions() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::open(dir.path()).await;
    for session in ["session-a", "session-b"] {
        fixture
            .service
            .record(ExperienceEvent::Message {
                session: session.into(),
                role: "user".into(),
                content: format!("isolated recall marker for {session}"),
                metadata: metadata(&format!("message-{session}")),
            })
            .await
            .unwrap();
    }
    let recalled = fixture
        .service
        .recall(request("isolated recall marker", 10, 4096))
        .await
        .unwrap();
    assert!(recalled
        .items
        .iter()
        .any(|item| item.content.ends_with("session-a")));
    assert!(!recalled
        .items
        .iter()
        .any(|item| item.content.ends_with("session-b")));
    let snapshot = fixture.service.metrics().snapshot();
    assert_eq!(
        snapshot.memory_record_total[&MemoryKindLabel::Message][&MemoryScopeLabel::Session],
        2
    );
    assert_eq!(
        snapshot.memory_recall_hits[&RecallSourceLabel::RecallMemory][&MemoryKindLabel::Message],
        1
    );
    assert_eq!(
        snapshot.memory_recall_latency_ms[&RecallSourceLabel::RecallMemory].count,
        1
    );
}

#[tokio::test]
async fn approved_core_record_ranks_before_conflicting_local_fact() {
    let dir = tempfile::tempdir().unwrap();
    let facts = FactStore::open(&dir.path().join("facts.db")).unwrap();
    facts
        .add_fact(
            "conflict marker says old value",
            "contract",
            "authority",
            "test",
            1.0,
            "semantic",
            0,
        )
        .unwrap();
    drop(facts);
    let fixture = Fixture::open(dir.path()).await;
    fixture.core.lock().await.set_block(MemoryBlock::read_only(
        "authority-test",
        "conflict marker says approved value",
        1024,
    ));

    let recalled = fixture
        .service
        .recall(request("conflict marker says", 10, 4096))
        .await
        .unwrap();
    assert_eq!(recalled.items.len(), 2);
    assert_eq!(
        recalled.items[0].content,
        "conflict marker says approved value"
    );
    assert_eq!(
        recalled.items[0].authority,
        mnemosyne::MemoryAuthority::ApprovedCore
    );
}

fn test_clock() -> Arc<dyn Clock> {
    Arc::new(kernel::chronos::TestClock::default())
}

fn metadata(id: &str) -> MemoryMetadata {
    MemoryMetadata::local(id, id, DateTime::<Utc>::UNIX_EPOCH)
}

fn request(query: &str, max_items: usize, max_content_bytes: usize) -> RecallRequest {
    RecallRequest {
        session: "session-a".into(),
        query: query.into(),
        max_items,
        max_content_bytes,
        current_at: Some(DateTime::<Utc>::UNIX_EPOCH),
        include_historical: false,
        mode: None,
    }
}

#[tokio::test]
async fn user_and_assistant_messages_persist_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::open(dir.path()).await;

    for (id, role, content) in [
        ("message-user", "user", "durable user statement"),
        (
            "message-assistant",
            "assistant",
            "durable assistant response",
        ),
    ] {
        fixture
            .service
            .record(ExperienceEvent::Message {
                session: "session-a".into(),
                role: role.into(),
                content: content.into(),
                metadata: metadata(id),
            })
            .await
            .unwrap();
    }

    drop(fixture.service);
    let reopened = RecallMemory::new(&dir.path().join("recall.db"), test_clock()).unwrap();
    let user = reopened.search("durable user statement", 10).unwrap();
    let assistant = reopened.search("durable assistant response", 10).unwrap();
    assert_eq!(user[0].session_id, "session-a");
    assert_eq!(user[0].entry_type, "user_message");
    assert_eq!(assistant[0].entry_type, "assistant_message");
}

#[tokio::test]
async fn reflection_decision_and_goal_outcome_persist_after_reopen() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::open(dir.path()).await;
    let events = [
        ExperienceEvent::Reflection {
            content: "reflection payload".into(),
            metadata: metadata("reflection-1"),
        },
        ExperienceEvent::ArchitectureDecision {
            title: "decision".into(),
            content: "architecture decision payload".into(),
            metadata: metadata("decision-1"),
        },
        ExperienceEvent::GoalOutcome {
            goal_id: "goal-1".into(),
            outcome: "complete".into(),
            content: "goal outcome payload".into(),
            metadata: metadata("outcome-1"),
        },
    ];
    for event in events {
        fixture.service.record(event).await.unwrap();
    }

    let root = fixture.root.clone();
    drop(fixture.service);
    let reopened = Fixture::open(&root).await.episodic_store().await;
    let rows = reopened.recall_reflections(10).unwrap();
    let ids = rows.iter().map(|row| row.id.as_str()).collect::<Vec<_>>();
    assert!(ids.contains(&"reflection-1"));
    assert!(ids.contains(&"decision-1"));
    assert!(ids.contains(&"outcome-1"));
    assert!(rows
        .iter()
        .any(|row| row.task_summary == "architecture decision payload"));
}

#[tokio::test]
async fn recorded_message_is_recalled_in_its_session() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::open(dir.path()).await;
    fixture
        .service
        .record(ExperienceEvent::Message {
            session: "session-a".into(),
            role: "user".into(),
            content: "message target recall token".into(),
            metadata: metadata("message-target"),
        })
        .await
        .unwrap();
    let recalled = fixture
        .service
        .recall(request("message target recall token", 10, 4096))
        .await
        .unwrap();
    assert!(recalled
        .items
        .iter()
        .any(|item| item.content == "message target recall token"));
}

#[tokio::test]
async fn recorded_reflection_is_recalled_for_relevant_query() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::open(dir.path()).await;
    fixture
        .service
        .record(ExperienceEvent::Reflection {
            content: "reflection target recall token".into(),
            metadata: metadata("reflection-target"),
        })
        .await
        .unwrap();
    let recalled = fixture
        .service
        .recall(request("reflection target recall token", 10, 4096))
        .await
        .unwrap();
    assert!(recalled
        .items
        .iter()
        .any(|item| item.content.contains("reflection target recall token")));
}

#[tokio::test]
async fn final_recall_obeys_item_and_byte_bounds() {
    let dir = tempfile::tempdir().unwrap();
    let facts = FactStore::open(&dir.path().join("facts.db")).unwrap();
    for suffix in ["alpha", "beta", "gamma"] {
        facts
            .add_fact(
                &format!("bounded-memory-token-{suffix}"),
                "contract",
                "bounds",
                "test",
                1.0,
                "semantic",
                0,
            )
            .unwrap();
    }
    drop(facts);
    let fixture = Fixture::open(dir.path()).await;

    let one = fixture
        .service
        .recall(request("bounded memory token", 1, 4096))
        .await
        .unwrap();
    assert_eq!(one.items.len(), 1);

    let bytes = fixture
        .service
        .recall(request("bounded memory token", 10, 26))
        .await
        .unwrap();
    assert!(
        bytes
            .items
            .iter()
            .map(|item| item.content.len())
            .sum::<usize>()
            <= 26
    );
}

struct OutageSupplemental;

#[async_trait]
impl SupplementalMemoryService for OutageSupplemental {
    fn queue_depth(&self) -> usize {
        0
    }

    fn record(&self, _: &ExperienceEvent, _: i64) -> Result<EnqueueOutcome, GbrainBackendError> {
        Err(GbrainBackendError::Unsupported)
    }

    async fn recall(&self, _: RecallRequest, _: &CancellationToken) -> SupplementalRecall {
        SupplementalRecall {
            items: Vec::new(),
            health: SupplementalRecallHealth {
                degraded: true,
                error_category: Some(SupplementalErrorCategory::Transport),
                queue_depth: 0,
            },
        }
    }

    fn forget(&self, _: ForgetPolicy) -> Result<(), GbrainBackendError> {
        Ok(())
    }
}

#[tokio::test]
async fn supplemental_outage_keeps_local_recall() {
    let dir = tempfile::tempdir().unwrap();
    let facts = FactStore::open(&dir.path().join("facts.db")).unwrap();
    facts
        .add_fact(
            "local recall survives supplemental outage",
            "contract",
            "outage",
            "test",
            1.0,
            "semantic",
            0,
        )
        .unwrap();
    drop(facts);
    let fixture = Fixture::open(dir.path()).await;
    let composite = CompositeMemoryService::new(
        Arc::new(fixture.service),
        Some(Arc::new(OutageSupplemental)),
        test_clock(),
        std::time::Duration::from_secs(1),
        std::time::Duration::from_secs(1),
    );
    let health = composite.health_handle();

    let recalled = composite
        .recall(request(
            "local recall survives supplemental outage",
            10,
            4096,
        ))
        .await
        .unwrap();
    assert_eq!(recalled.items.len(), 1);
    assert!(health.lock().unwrap().degraded);
}

#[tokio::test]
async fn sqlite_paths_and_defining_tables_are_stable() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::open(dir.path()).await;
    drop(fixture.service);

    for (name, table) in [
        ("recall.db", "recall_memory"),
        ("facts.db", "facts"),
        ("episodic.db", "reflection_events"),
    ] {
        let path = dir.path().join(name);
        assert!(path.exists(), "missing {}", path.display());
        let db = Connection::open(path).unwrap();
        let exists: i64 = db
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type='table' AND name=?1)",
                [table],
                |row| row.get(0),
            )
            .unwrap();
        assert_eq!(exists, 1, "missing defining table {table}");
    }
}

#[tokio::test]
async fn consolidation_baseline_remains_callable() {
    let dir = tempfile::tempdir().unwrap();
    let fixture = Fixture::open(dir.path()).await;
    fixture
        .service
        .consolidate(mnemosyne::MemoryScope::Session("session-a".into()))
        .await
        .unwrap();
}
