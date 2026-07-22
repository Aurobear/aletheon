use std::{collections::HashMap, sync::Arc};

use async_trait::async_trait;
use executive::application::session_service::SessionService;
use executive::host::daemon::session_manager::SessionManager;
use executive::host::legacy_session::{
    LegacySessionResources, LegacySessionService, LegacySessionUseCases,
};
use executive::runtime::session::canonical_store::CanonicalSessionStore;
use executive::runtime::session::store::SessionStore;
use fabric::{
    Clock, ContentBlock, LlmProvider, LlmResponse, LlmStream, Message, SessionAppendStore,
    SessionId, StopReason, ToolDefinition, Usage,
};
use kernel::chronos::TestClock;
use tokio::sync::Mutex;

struct SummaryLlm;

#[async_trait]
impl LlmProvider for SummaryLlm {
    async fn complete(
        &self,
        _messages: &[Message],
        _tools: &[ToolDefinition],
    ) -> anyhow::Result<LlmResponse> {
        Ok(LlmResponse {
            content: vec![ContentBlock::Text {
                text: "summary".into(),
            }],
            stop_reason: StopReason::EndTurn,
            usage: Usage::default(),
            cache_hit_tokens: 0,
            cache_miss_tokens: 0,
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
    let mut manager = SessionManager::new(temp.path(), initial_id.clone(), 8_000, clock.clone())
        .await
        .unwrap();
    for message in messages {
        match (&message.role, message.content.as_slice()) {
            (fabric::Role::User, [ContentBlock::Text { text }]) => manager.push_user(text).await,
            (fabric::Role::Assistant, [ContentBlock::Text { text }]) => {
                manager.push_assistant(text).await
            }
            _ => manager.push_message(message.clone()).await,
        }
    }
    let registry = Arc::new(Mutex::new(HashMap::from([(
        initial_id.clone(),
        Arc::new(Mutex::new(manager)),
    )])));
    let canonical_store: Arc<dyn SessionAppendStore> =
        Arc::new(CanonicalSessionStore::open(temp.path().join("canonical.db")).unwrap());
    let active = Arc::new(Mutex::new(HashMap::new()));
    let canonical = Arc::new(SessionService::new(canonical_store, active));
    let service = LegacySessionService::new(LegacySessionResources {
        registry,
        created_at: Arc::new(Mutex::new(HashMap::from([(initial_id, clock.mono_now())]))),
        data_dir: temp.path().to_path_buf(),
        context_window: 8_000,
        clock,
        llm: Arc::new(SummaryLlm),
        canonical: canonical.clone(),
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
async fn clear_rotates_to_empty_canonical_session() {
    let (_temp, service, canonical) = service_with_history(&[Message::user("stale context")]).await;

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

#[test]
fn session_rpc_and_routing_do_not_construct_concrete_session_stores() {
    let rpc = include_str!("../src/host/daemon/handler/rpc/rpc_session.rs");
    let handler = include_str!("../src/host/daemon/handler/mod.rs");
    for forbidden in [
        "SessionStore::new",
        "SessionManager::new",
        "SessionManager::recover",
        "get_or_create_session",
        "register_default_session",
    ] {
        assert!(!rpc.contains(forbidden), "RPC contains {forbidden}");
        assert!(!handler.contains(forbidden), "handler contains {forbidden}");
    }
    assert!(!std::path::Path::new("src/impl/daemon/handler/session_routing.rs").exists());
}
