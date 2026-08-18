use std::{collections::HashMap, sync::Arc};

use ::contracts::{
    Clock, ContentBlock, InferenceUsage, LlmProvider, LlmResponse, LlmStream, Message, SessionId,
    StopReason, ToolDefinition,
};
use adapters_sqlite::session::canonical_store::CanonicalSessionStore;
use adapters_sqlite::session::store::SessionStore;
use aletheon::daemon::legacy_session::{
    LegacySessionResources, LegacySessionService, LegacySessionUseCases,
};
use async_trait::async_trait;
use kernel::chronos::TestClock;
use mnemosyne::context_compactor::MnemosyneContextCompactorFactory;
use runtime::session_service::SessionService;
use runtime::ContextCompactorFactory;
use runtime::ContextWorkingSet;
use tokio::sync::Mutex;

struct SummaryLlm;

struct EmptySpine;

impl runtime::event_spine::EventSpine for EmptySpine {
    fn append(
        &self,
        _event: runtime::event_spine::UnsequencedEvent,
    ) -> anyhow::Result<runtime::event_spine::SpineEvent> {
        anyhow::bail!("Runtime journal shadow must not append")
    }

    fn read_committed_page(
        &self,
        _after_offset: u64,
        _through_offset: u64,
        _limit: usize,
    ) -> anyhow::Result<Vec<(u64, runtime::event_spine::SpineEvent)>> {
        Ok(Vec::new())
    }
}

#[async_trait]
impl LlmProvider for SummaryLlm {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse {
            content: vec![ContentBlock::Text {
                text: [
                    "## Active Task\nPreserve the session across compaction.",
                    "## Goal\nKeep the relevant conversation state.",
                    "## Completed Actions\nEarlier turns were recorded.",
                    "## Active State\nThe session is being compacted.",
                    "## In Progress\nCompaction validation.",
                    "## Blocked\nNone.",
                    "## Key Decisions\nUse an immutable projected session.",
                    "## Pending User Asks\nContinue the conversation.",
                    "## Relevant Files\nNone.",
                    "## Remaining Work\nResume from the protected tail.",
                    "## Critical Context\nRetain the user's requirements and constraints.",
                ]
                .join("\n\n"),
            }],
            stop_reason: StopReason::EndTurn,
            usage: InferenceUsage::default(),
        })
    }

    async fn complete_stream(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmStream> {
        anyhow::bail!("streaming is not used")
    }

    fn name(&self) -> &str {
        "summary-test"
    }

    fn max_context_length(&self) -> usize {
        8_000
    }
}

async fn service_with_history(
    messages: &[Message],
) -> (tempfile::TempDir, LegacySessionService, Arc<SessionService>) {
    let temp = tempfile::tempdir().unwrap();
    let initial_id = "legacy-initial".to_string();
    let clock: Arc<dyn Clock> = Arc::new(TestClock::default());
    SessionStore::new(temp.path())
        .unwrap()
        .create_session(&initial_id)
        .unwrap();
    let factory = MnemosyneContextCompactorFactory;
    let mut manager = ContextWorkingSet::new(
        temp.path(),
        initial_id.clone(),
        clock.clone(),
        factory.create(8_000, 80),
    )
    .await
    .unwrap();
    for message in messages {
        match (&message.role, message.content.as_slice()) {
            (::contracts::Role::User, [ContentBlock::Text { text }]) => {
                manager.push_user(text).await
            }
            (::contracts::Role::Assistant, [ContentBlock::Text { text }]) => {
                manager.push_assistant(text).await
            }
            _ => manager.push_message(message.clone()).await,
        }
    }
    let registry = Arc::new(Mutex::new(HashMap::from([(
        initial_id.clone(),
        Arc::new(Mutex::new(manager)),
    )])));
    let canonical_store_impl =
        Arc::new(CanonicalSessionStore::open(temp.path().join("canonical.db")).unwrap());
    let canonical_store =
        aletheon::host::session::test_composition::compose_in_memory_session_store(
            canonical_store_impl,
        );
    let active = Arc::new(runtime::ActiveTurnRegistry::new());
    let canonical = Arc::new(SessionService::new(canonical_store.clone(), active));
    let writer = Arc::new(runtime::RuntimeSessionWriter::new(
        runtime::SessionAuthority::new(Arc::new(runtime::RuntimeJournalShadow::new(Arc::new(
            EmptySpine,
        )))),
        canonical_store,
        clock.clone(),
    ));
    let service = LegacySessionService::new(LegacySessionResources {
        registry,
        created_at: Arc::new(Mutex::new(HashMap::from([(initial_id, clock.mono_now())]))),
        data_dir: temp.path().to_path_buf(),
        context_window: 8_000,
        clock,
        llm: Arc::new(SummaryLlm),
        canonical: canonical.clone(),
        session_commands: writer,
        session_writer: aletheon::config::SessionWriterMode::Legacy,
    });
    (temp, service, canonical)
}

async fn runtime_writer_service() -> (tempfile::TempDir, LegacySessionService, Arc<SessionService>)
{
    let temp = tempfile::tempdir().unwrap();
    let clock: Arc<dyn Clock> = Arc::new(TestClock::default());
    let store: Arc<dyn ::contracts::SessionAppendStore> =
        Arc::new(runtime::session_writer::InMemoryAppendStore::default());
    let canonical = Arc::new(SessionService::new(
        store.clone(),
        Arc::new(runtime::ActiveTurnRegistry::new()),
    ));
    let writer = Arc::new(runtime::RuntimeSessionWriter::new(
        runtime::SessionAuthority::new(Arc::new(runtime::RuntimeJournalShadow::new(Arc::new(
            EmptySpine,
        )))),
        store,
        clock.clone(),
    ));
    let service = LegacySessionService::new(LegacySessionResources {
        registry: Arc::new(Mutex::new(HashMap::new())),
        created_at: Arc::new(Mutex::new(HashMap::new())),
        data_dir: temp.path().to_path_buf(),
        context_window: 8_000,
        clock,
        llm: Arc::new(SummaryLlm),
        canonical: canonical.clone(),
        session_commands: writer,
        session_writer: aletheon::config::SessionWriterMode::Runtime,
    });
    (temp, service, canonical)
}

#[tokio::test]
async fn resume_imports_legacy_journal_into_canonical_history_once() {
    let (_temp, service, canonical) =
        service_with_history(&[Message::user("remember this"), Message::assistant("I will")]).await;

    let snapshot = service.resume("legacy-initial".into()).await.unwrap();
    assert_eq!(snapshot.messages.len(), 2);
    let canonical_history = canonical
        .resume(&SessionId("legacy-initial".into()))
        .await
        .unwrap();
    assert_eq!(canonical_history.messages.len(), 2);

    service.resume("legacy-initial".into()).await.unwrap();
    assert_eq!(
        canonical
            .resume(&SessionId("legacy-initial".into()))
            .await
            .unwrap()
            .messages
            .len(),
        2
    );
}

#[tokio::test]
async fn legacy_list_reads_canonical_sessions_and_historical_rows_without_writing_legacy_db() {
    let (temp, service, _canonical) = service_with_history(&[Message::user("historical")]).await;

    let created = service.create().await.unwrap();
    let listed = service.list().await.unwrap();
    let listed_ids: Vec<_> = listed
        .iter()
        .map(|session| session.session_id.as_str())
        .collect();
    assert!(listed_ids.contains(&"legacy-initial"));
    assert!(listed_ids.contains(&created.session_id.as_str()));
    assert_eq!(
        SessionStore::open_read_only(&temp.path().join("sessions.db"))
            .unwrap()
            .list_sessions()
            .unwrap(),
        vec!["legacy-initial"],
        "legacy list remains read-only"
    );
}

#[tokio::test]
async fn clear_rotates_to_empty_canonical_session() {
    let (temp, service, canonical) = service_with_history(&[Message::user("stale context")]).await;

    let transition = service.clear("legacy-initial").await.unwrap();
    assert_eq!(transition.previous.session_id, "legacy-initial");
    assert_ne!(
        transition.current.session_id,
        transition.previous.session_id
    );
    assert!(canonical
        .resume(&SessionId(transition.current.session_id.clone()))
        .await
        .unwrap()
        .messages
        .is_empty());
    assert_eq!(
        service.current("legacy-initial").await.unwrap().session_id,
        "legacy-initial"
    );
    assert_eq!(
        SessionStore::open_read_only(&temp.path().join("sessions.db"))
            .unwrap()
            .list_sessions()
            .unwrap(),
        vec!["legacy-initial"],
        "legacy sessions.db remains an unchanged read-only audit source"
    );
}

#[tokio::test]
async fn compact_materializes_a_new_canonical_session() {
    let messages: Vec<_> = (0..20)
        .flat_map(|index| {
            [
                Message::user(format!("user {index} {}", "x".repeat(1_000))),
                Message::assistant(format!("assistant {index} {}", "y".repeat(1_000))),
            ]
        })
        .collect();
    let (_temp, service, canonical) = service_with_history(&messages).await;

    let transition = service
        .compact("legacy-initial")
        .await
        .unwrap()
        .expect("long history should compact");
    assert_ne!(
        transition.current.session_id,
        transition.previous.session_id
    );
    let projected = canonical
        .resume(&SessionId(transition.current.session_id.clone()))
        .await
        .unwrap();
    assert!(!projected.messages.is_empty());
    assert!(projected.messages.len() < messages.len());
    assert_eq!(
        service.current("legacy-initial").await.unwrap().session_id,
        "legacy-initial"
    );
}

#[tokio::test]
async fn runtime_mode_facade_creates_only_through_the_runtime_command_port() {
    let (temp, service, canonical) = runtime_writer_service().await;

    let session_id = service
        .route_workspace(temp.path().join("workspace"))
        .await
        .unwrap();
    assert!(session_id.starts_with("session-"));
    assert!(canonical
        .try_resume(&SessionId(session_id.clone()))
        .await
        .unwrap()
        .is_some());
    assert!(!temp.path().join("sessions.db").exists());
}

#[test]
fn session_rpc_and_routing_do_not_construct_concrete_session_stores() {
    let rpc = include_str!("../src/daemon/handler/rpc/rpc_session.rs");
    let handler = include_str!("../src/daemon/handler/mod.rs");
    for forbidden in [
        "SessionStore::new",
        "ContextWorkingSet::new",
        "ContextWorkingSet::recover",
        "get_or_create_session",
        "register_default_session",
    ] {
        assert!(!rpc.contains(forbidden), "RPC contains {forbidden}");
        assert!(!handler.contains(forbidden), "handler contains {forbidden}");
    }
    assert!(!std::path::Path::new("src/impl/daemon/handler/session_routing.rs").exists());
}
