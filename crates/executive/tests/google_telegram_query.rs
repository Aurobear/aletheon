use async_trait::async_trait;
use corpus::tools::google::oauth::GoogleBinding;
use corpus::tools::google::oauth::{GoogleCapability, GoogleOAuthProvider, OAuthClientConfig};
use corpus::tools::mcp::token_store::{TokenEntry, TokenKey, TokenStore};
use executive::r#impl::external::{ExternalIdentityRepository, GoogleIntegration};
use fabric::channel::{
    ChannelId, ConversationId, ExternalSenderId, InboundMessage, MessageContent, MessageId,
    OutboundMessage,
};
use fabric::{ExternalCapabilityId, ExternalIdentityId, ExternalProviderId, PrincipalId};
use gateway::dispatcher::{
    ChannelDispatcher, ChannelTransport, ChannelTurnExecutor, ProviderEnvelope,
};
use gateway::handlers::chat::ChatHandler;
use gateway::handlers::external_read::{ExternalAccountDirectory, ExternalReadPreprocessor};
use gateway::handlers::greeting::GreetingHandler;
use gateway::registry::CapabilityRegistry;
use gateway::ChannelStore;
use http_body_util::Full;
use hyper::body::Bytes;
use hyper::service::service_fn;
use hyper::{Request, Response, StatusCode};
use hyper_util::rt::TokioIo;
use kernel::chronos::TestClock;
use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;

const TOKEN_SENTINEL: &str = "ya29.refresh-result-must-not-leak";

#[derive(Default)]
struct Turn {
    calls: AsyncMutex<Vec<(String, String)>>,
}

#[async_trait]
impl ChannelTurnExecutor for Turn {
    async fn execute(
        &self,
        principal: &str,
        message: &str,
        _correlation_id: &str,
    ) -> anyhow::Result<String> {
        self.calls
            .lock()
            .await
            .push((principal.to_owned(), message.to_owned()));
        Ok("normal-react-response".into())
    }
}

struct Accounts(Vec<String>);

#[async_trait]
impl ExternalAccountDirectory for Accounts {
    async fn active_account_labels(&self, _principal: &str) -> anyhow::Result<Vec<String>> {
        Ok(self.0.clone())
    }
}

#[derive(Default)]
struct Transport {
    sent: AsyncMutex<Vec<OutboundMessage>>,
}

#[async_trait]
impl ChannelTransport for Transport {
    fn channel_id(&self) -> &str {
        "telegram"
    }
    async fn receive(&self, _cursor: Option<String>) -> anyhow::Result<Vec<ProviderEnvelope>> {
        Ok(Vec::new())
    }
    async fn send(&self, message: &OutboundMessage) -> anyhow::Result<String> {
        self.sent.lock().await.push(message.clone());
        Ok("sent-1".into())
    }
}

fn inbound(id: &str, sender: &str, text: &str) -> ProviderEnvelope {
    ProviderEnvelope {
        message: InboundMessage {
            channel_id: ChannelId("telegram".into()),
            message_id: MessageId(id.into()),
            conversation_id: ConversationId("42".into()),
            sender_id: ExternalSenderId(sender.into()),
            content: MessageContent::Text { text: text.into() },
            timestamp_ms: 1,
            reply_to_action: None,
            correlation_id: format!("telegram:{id}"),
        },
        next_cursor: format!("cursor-{id}"),
    }
}

fn router(path: &std::path::Path, turn: Arc<Turn>, accounts: Vec<String>) -> ChannelDispatcher {
    let store = ChannelStore::open(path).unwrap();
    store
        .bind("telegram", "telegram:7", "principal-7", "active")
        .unwrap();
    let mut registry = CapabilityRegistry::new();
    registry.register(Arc::new(ChatHandler::new(
        turn,
        Some(Arc::new(ExternalReadPreprocessor::new(Arc::new(Accounts(
            accounts,
        ))))),
    )));
    registry.register(Arc::new(GreetingHandler));
    ChannelDispatcher::with_registry(store, registry)
}

#[tokio::test]
async fn multiple_accounts_prompt_without_guessing_or_provider_bypass() {
    let dir = tempfile::tempdir().unwrap();
    let turn = Arc::new(Turn::default());
    let mut router = router(
        &dir.path().join("channels.db"),
        turn.clone(),
        vec!["work".into(), "personal".into()],
    );
    let transport = Transport::default();
    router
        .process(
            &transport,
            inbound("1", "telegram:7", "what are today's events?"),
        )
        .await
        .unwrap();

    assert!(turn.calls.lock().await.is_empty());
    let sent = transport.sent.lock().await;
    let MessageContent::Text { text } = &sent[0].content else {
        panic!("expected text")
    };
    assert!(text.contains("choose an external account"));
    assert!(text.contains("work"));
    assert!(text.contains("personal"));
    assert!(!text.contains(TOKEN_SENTINEL));
}

#[tokio::test]
async fn one_account_and_authenticated_principal_use_normal_react_path() {
    let dir = tempfile::tempdir().unwrap();
    let turn = Arc::new(Turn::default());
    let mut router = router(
        &dir.path().join("channels.db"),
        turn.clone(),
        vec!["work".into()],
    );
    let transport = Transport::default();
    router
        .process(
            &transport,
            inbound("2", "telegram:7", "show important unread mail"),
        )
        .await
        .unwrap();
    let calls = turn.calls.lock().await;
    assert_eq!(calls.len(), 1);
    assert_eq!(calls[0].0, "principal-7");
    assert!(calls[0].1.contains("<trusted-external-account>work"));
    assert!(calls[0].1.contains("important unread mail"));
    assert!(!calls[0].1.contains(TOKEN_SENTINEL));
}

#[tokio::test]
async fn unauthorized_telegram_identity_never_reaches_account_or_turn_flow() {
    let dir = tempfile::tempdir().unwrap();
    let turn = Arc::new(Turn::default());
    let mut router = router(
        &dir.path().join("channels.db"),
        turn.clone(),
        vec!["work".into()],
    );
    router
        .process(
            &Transport::default(),
            inbound("3", "telegram:attacker", "today's events"),
        )
        .await
        .unwrap();
    assert!(turn.calls.lock().await.is_empty());
    let store = ChannelStore::open(&dir.path().join("channels.db")).unwrap();
    assert_eq!(
        store.inbox_status("telegram", "3").unwrap().as_deref(),
        Some("rejected")
    );
}

#[test]
fn account_binding_and_revocation_survive_repository_restart() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("objectives.db");
    let owner = PrincipalId("principal-7".into());
    let identity_id = ExternalIdentityId::new();
    let repository = ExternalIdentityRepository::open(&path).unwrap();
    let (identity, _) = repository
        .bind_google(
            &owner,
            GoogleBinding {
                identity_id,
                provider_subject: "subject-7".into(),
                email: "owner@example.com".into(),
                scopes: vec![ExternalCapabilityId::new("mail.read").unwrap()],
            },
            Some("work".into()),
            10,
        )
        .unwrap();
    drop(repository);

    let repository = ExternalIdentityRepository::open(&path).unwrap();
    assert_eq!(
        repository.resolve_account(&owner, "work").unwrap(),
        Some(identity_id)
    );
    repository
        .revoke_local(&owner, identity_id, identity.version, 20)
        .unwrap();
    drop(repository);

    let repository = ExternalIdentityRepository::open(&path).unwrap();
    let (restarted, grant) = repository.get(&owner, identity_id).unwrap().unwrap();
    assert_eq!(restarted.state, fabric::ExternalIdentityState::Revoked);
    assert_eq!(grant.state, fabric::GrantState::Revoked);
    assert_eq!(repository.resolve_account(&owner, "work").unwrap(), None);
}

#[tokio::test]
async fn expired_token_refresh_is_singleflight_and_transcript_safe() {
    let (endpoint, requests) = mock_server(vec![(
        StatusCode::OK,
        format!(
            r#"{{"access_token":"{TOKEN_SENTINEL}","expires_in":3600,"token_type":"Bearer","scope":"{}"}}"#,
            GoogleCapability::MailRead.oauth_scope()
        ),
    )])
    .await;
    let dir = tempfile::tempdir().unwrap();
    let identity = ExternalIdentityId::new();
    let mut tokens = TokenStore::new(dir.path().join("tokens.json")).unwrap();
    tokens.set_key(
        TokenKey::external(ExternalProviderId::new("google").unwrap(), identity),
        TokenEntry {
            access_token: "expired-access".into(),
            refresh_token: Some("refresh-secret".into()),
            expires_at: 0,
            scopes: vec![GoogleCapability::MailRead.oauth_scope().into()],
            token_type: "Bearer".into(),
        },
    );
    tokens.save().unwrap();
    let provider = GoogleOAuthProvider::with_local_endpoints(
        OAuthClientConfig {
            client_id: "client".into(),
            client_secret: None,
            redirect_uri: "http://localhost/callback".into(),
            auth_url: format!("{endpoint}/authorize"),
            token_url: format!("{endpoint}/token"),
            revocation_url: Some(format!("{endpoint}/revoke")),
            userinfo_url: Some(format!("{endpoint}/userinfo")),
            client_auth_method: corpus::tools::google::oauth::OAuthClientAuthMethod::None,
        },
        vec![ExternalCapabilityId::new("mail.read").unwrap()],
        tokens,
        Arc::new(TestClock::default()),
    )
    .unwrap();
    assert_eq!(
        provider
            .access_credential(identity)
            .unwrap_err()
            .to_string(),
        "google_reauthorization_required"
    );
    let repository = Arc::new(Mutex::new(
        ExternalIdentityRepository::open(&dir.path().join("objectives.db")).unwrap(),
    ));
    let integration = Arc::new(GoogleIntegration::new(
        repository,
        Arc::new(AsyncMutex::new(provider)),
    ));
    let (first, second) = tokio::join!(
        integration.refresh_singleflight(identity),
        integration.refresh_singleflight(identity)
    );
    assert!(!format!("{:?}", first.unwrap()).contains(TOKEN_SENTINEL));
    assert!(!format!("{:?}", second.unwrap()).contains(TOKEN_SENTINEL));
    assert_eq!(requests.lock().unwrap().len(), 1);
}

#[tokio::test]
async fn refresh_failure_returns_only_reauthorization_status() {
    let (endpoint, _) = mock_server(vec![(StatusCode::BAD_REQUEST, "{}".into())]).await;
    let dir = tempfile::tempdir().unwrap();
    let identity = ExternalIdentityId::new();
    let mut tokens = TokenStore::new(dir.path().join("tokens.json")).unwrap();
    tokens.set_key(
        TokenKey::external(ExternalProviderId::new("google").unwrap(), identity),
        TokenEntry {
            access_token: "expired".into(),
            refresh_token: Some("refresh-do-not-print".into()),
            expires_at: 0,
            scopes: vec![GoogleCapability::MailRead.oauth_scope().into()],
            token_type: "Bearer".into(),
        },
    );
    let mut provider = GoogleOAuthProvider::with_local_endpoints(
        OAuthClientConfig {
            client_id: "client".into(),
            client_secret: None,
            redirect_uri: "http://localhost/callback".into(),
            auth_url: format!("{endpoint}/authorize"),
            token_url: format!("{endpoint}/token"),
            revocation_url: None,
            userinfo_url: None,
            client_auth_method: corpus::tools::google::oauth::OAuthClientAuthMethod::None,
        },
        vec![ExternalCapabilityId::new("mail.read").unwrap()],
        tokens,
        Arc::new(TestClock::default()),
    )
    .unwrap();
    let error = provider.refresh_credential(identity).await.unwrap_err();
    assert_eq!(error.to_string(), "google_reauthorization_required");
    assert!(!error.to_string().contains("refresh-do-not-print"));
}

async fn mock_server(responses: Vec<(StatusCode, String)>) -> (String, Arc<Mutex<Vec<String>>>) {
    let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let responses = Arc::new(Mutex::new(VecDeque::from(responses)));
    let requests = Arc::new(Mutex::new(Vec::new()));
    let request_log = requests.clone();
    tokio::spawn(async move {
        loop {
            let Ok((stream, _)) = listener.accept().await else {
                break;
            };
            let responses = responses.clone();
            let request_log = request_log.clone();
            tokio::spawn(async move {
                let service = service_fn(move |request: Request<hyper::body::Incoming>| {
                    let responses = responses.clone();
                    let request_log = request_log.clone();
                    async move {
                        request_log
                            .lock()
                            .unwrap()
                            .push(request.uri().path().to_owned());
                        let (status, body) = responses
                            .lock()
                            .unwrap()
                            .pop_front()
                            .unwrap_or((StatusCode::INTERNAL_SERVER_ERROR, "{}".into()));
                        Ok::<_, hyper::Error>(
                            Response::builder()
                                .status(status)
                                .header("content-type", "application/json")
                                .body(Full::new(Bytes::from(body)))
                                .unwrap(),
                        )
                    }
                });
                let _ = hyper::server::conn::http1::Builder::new()
                    .serve_connection(TokioIo::new(stream), service)
                    .await;
            });
        }
    });
    (format!("http://{address}"), requests)
}
