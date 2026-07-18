use corpus::tools::google::oauth::GoogleBinding;
use executive::r#impl::approval::{ApprovalDecision, ApprovalResolutionContext};
use executive::r#impl::artifact::{ArtifactRecord, ArtifactScanStatus};
use executive::r#impl::channel::daemon_adapter::DaemonGmailDraftApprovalExecutor;
use executive::r#impl::channel::gmail::ingest::{
    GmailIngestResult, GmailOriginalReference, IngestedAttachment,
};
use executive::r#impl::channel::gmail::sender_policy::{
    AuthenticationRequirement, GmailHeader, GmailSenderPolicy,
};
use executive::r#impl::channel::gmail::{
    GmailChannelMessage, GmailChannelStore, GmailGoalDraftCoordinator,
};
use executive::r#impl::channel::dispatcher::{
    ChannelDispatcher, ChannelTransport, ChannelTurnExecutor, ProviderEnvelope,
};
use executive::r#impl::channel::registry::ApprovalResolver;
use executive::r#impl::channel::store::ChannelStore;
use executive::r#impl::external::ExternalIdentityRepository;
use executive::r#impl::goal::ObjectiveStore;
use fabric::{
    ApprovalCategory, ApprovalStatus, ExternalIdentityId, ExternalScope, GoalState, PrincipalId,
};
use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};
use tokio::sync::Mutex as AsyncMutex;

struct Fixture {
    _dir: tempfile::TempDir,
    path: PathBuf,
    account: ExternalIdentityId,
    principal: PrincipalId,
    policy: GmailSenderPolicy,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("objectives.db");
        let account = ExternalIdentityId::new();
        let principal = PrincipalId("owner".into());
        ExternalIdentityRepository::open(&path)
            .unwrap()
            .bind_google(
                &principal,
                GoogleBinding {
                    identity_id: account,
                    provider_subject: "subject".into(),
                    email: "owner@example.com".into(),
                    scopes: vec![ExternalScope::GmailReadonly],
                },
                Some("work".into()),
                1,
            )
            .unwrap();
        let policy = GmailSenderPolicy {
            principal: principal.clone(),
            version: 7,
            allowed_addresses: HashSet::from(["alice@example.com".into()]),
            allowed_domains: HashSet::new(),
            trusted_authserv_ids: HashSet::from(["mx.google.com".into()]),
            authentication: AuthenticationRequirement::SpfOrDkim,
        };
        Self {
            _dir: dir,
            path,
            account,
            principal,
            policy,
        }
    }

    fn accepted(&self, id: &str) -> executive::r#impl::channel::gmail::GmailInboxRecord {
        let store = GmailChannelStore::open(&self.path).unwrap();
        store
            .authenticate_and_persist(
                &GmailChannelMessage {
                    account_id: self.account,
                    message_id: id.into(),
                    thread_id: format!("thread-{id}"),
                    headers: vec![
                        GmailHeader {
                            name: "From".into(),
                            value: "Alice <alice@example.com>".into(),
                        },
                        GmailHeader {
                            name: "Subject".into(),
                            value: "[GOAL] ship safely".into(),
                        },
                        GmailHeader {
                            name: "Authentication-Results".into(),
                            value: "mx.google.com; spf=pass smtp.mailfrom=alice@example.com".into(),
                        },
                    ],
                },
                Some(&self.policy),
                10,
            )
            .unwrap()
            .1
    }

    fn ingested(&self, id: &str, intent: &str) -> GmailIngestResult {
        GmailIngestResult {
            body_text: intent.into(),
            original: GmailOriginalReference {
                account_id: self.account,
                message_id: id.into(),
                thread_id: format!("thread-{id}"),
                source_timestamp_ms: 9,
            },
            attachments: vec![
                IngestedAttachment {
                    part_id: "clean".into(),
                    filename: "evidence.txt".into(),
                    mime_type: "text/plain".into(),
                    artifact: Some(ArtifactRecord {
                        artifact_id: format!("sha256:{}", "a".repeat(64)),
                        sha256: "a".repeat(64),
                        size_bytes: 4,
                        mime_type: "text/plain".into(),
                        relative_path: PathBuf::from("aa").join("a".repeat(64)),
                        scan_status: ArtifactScanStatus::Clean,
                    }),
                    unavailable_reason: None,
                },
                IngestedAttachment {
                    part_id: "unsafe".into(),
                    filename: "unsafe.pdf".into(),
                    mime_type: "application/pdf".into(),
                    artifact: Some(ArtifactRecord {
                        artifact_id: format!("sha256:{}", "b".repeat(64)),
                        sha256: "b".repeat(64),
                        size_bytes: 4,
                        mime_type: "application/pdf".into(),
                        relative_path: PathBuf::from("bb").join("b".repeat(64)),
                        scan_status: ArtifactScanStatus::Unscanned,
                    }),
                    unavailable_reason: Some("unscanned".into()),
                },
            ],
        }
    }

    fn resolve(
        &self,
        draft: &executive::r#impl::channel::gmail::GmailGoalDraft,
        principal: &PrincipalId,
        decision: ApprovalDecision,
        now: i64,
    ) -> anyhow::Result<fabric::ApprovalSnapshot> {
        let repo = executive::r#impl::approval::ApprovalRepository::open(&self.path)?;
        Ok(repo.resolve(
            draft.approval.id,
            draft.approval.version,
            &ApprovalResolutionContext {
                principal_id: principal.clone(),
                channel: "telegram".into(),
            },
            decision,
            now,
        )?)
    }
}

#[test]
fn verified_goal_is_exactly_once_draft_with_bounded_review_and_no_job() {
    let f = Fixture::new();
    let inbox = f.accepted("m1");
    let mut coordinator = GmailGoalDraftCoordinator::open(&f.path).unwrap();
    let first = coordinator
        .create_draft(
            &inbox,
            &f.policy,
            &f.ingested("m1", "ship feature"),
            "event-1",
            100,
            1_000,
        )
        .unwrap();
    let duplicate = coordinator
        .create_draft(
            &inbox,
            &f.policy,
            &f.ingested("m1", "changed replay"),
            "event-1",
            101,
            1_000,
        )
        .unwrap();
    assert_eq!(first.goal.id, duplicate.goal.id);
    assert_eq!(first.approval.id, duplicate.approval.id);
    assert_eq!(first.goal.state, GoalState::Draft);
    assert_eq!(first.goal.spec.original_intent, "ship feature");
    assert_eq!(first.approval.artifacts.len(), 1);
    assert!(first.approval.summary.contains("1 clean, 1 unavailable"));
    assert!(first.approval.summary.len() < 4_096);
    let store = ObjectiveStore::open(&f.path).unwrap();
    assert_eq!(store.list_goals(&[], 100).unwrap().len(), 1);
    let db = rusqlite::Connection::open(&f.path).unwrap();
    let jobs: i64 = db
        .query_row("SELECT count(*) FROM goal_attempts", [], |row| row.get(0))
        .unwrap();
    assert_eq!(jobs, 0);
}

#[test]
fn telegram_owner_confirmation_is_durable_and_wrong_or_email_identity_cannot_activate() {
    let f = Fixture::new();
    let inbox = f.accepted("m2");
    let mut coordinator = GmailGoalDraftCoordinator::open(&f.path).unwrap();
    let draft = coordinator
        .create_draft(
            &inbox,
            &f.policy,
            &f.ingested("m2", "confirm me"),
            "event-2",
            100,
            1_000,
        )
        .unwrap();
    assert!(f
        .resolve(
            &draft,
            &PrincipalId("attacker".into()),
            ApprovalDecision::Approve,
            110,
        )
        .is_err());
    let repo = executive::r#impl::approval::ApprovalRepository::open(&f.path).unwrap();
    assert!(repo
        .resolve(
            draft.approval.id,
            0,
            &ApprovalResolutionContext {
                principal_id: f.principal.clone(),
                channel: "gmail".into(),
            },
            ApprovalDecision::Approve,
            111,
        )
        .is_err());
    let approved = f
        .resolve(&draft, &f.principal, ApprovalDecision::Approve, 120)
        .unwrap();
    drop(coordinator);
    let restarted = GmailGoalDraftCoordinator::open(&f.path).unwrap();
    let goal = restarted.confirm(&approved, 121).unwrap();
    assert_eq!(goal.state, GoalState::Ready);
    assert_eq!(approved.status, ApprovalStatus::Approved);
    assert_eq!(
        restarted.confirm(&approved, 122).unwrap().state,
        GoalState::Ready
    );
}

#[test]
fn edit_versions_intent_and_requires_a_fresh_confirmation_while_reject_cancels() {
    let f = Fixture::new();
    let inbox = f.accepted("m3");
    let mut coordinator = GmailGoalDraftCoordinator::open(&f.path).unwrap();
    let draft = coordinator
        .create_draft(
            &inbox,
            &f.policy,
            &f.ingested("m3", "old intent"),
            "event-3",
            100,
            1_000,
        )
        .unwrap();
    let edit = f
        .resolve(
            &draft,
            &f.principal,
            ApprovalDecision::Reject {
                reason: Some("owner requested revision".into()),
            },
            110,
        )
        .unwrap();
    coordinator.reject_or_edit(&edit, true, 111).unwrap();
    let revised = coordinator
        .revise(draft.goal.id, &f.principal, "new intent", 120, 1_200)
        .unwrap();
    assert_eq!(revised.revision, 2);
    assert_ne!(revised.approval.id, draft.approval.id);
    assert_eq!(revised.goal.spec.original_intent, "new intent");
    let rejected = f
        .resolve(
            &revised,
            &f.principal,
            ApprovalDecision::Reject { reason: None },
            130,
        )
        .unwrap();
    assert_eq!(
        coordinator
            .reject_or_edit(&rejected, false, 131)
            .unwrap()
            .state,
        GoalState::Cancelled
    );
}

#[test]
fn revoked_account_and_confirmation_racing_deletion_fail_closed() {
    let f = Fixture::new();
    let inbox = f.accepted("m4");
    let mut coordinator = GmailGoalDraftCoordinator::open(&f.path).unwrap();
    let draft = coordinator
        .create_draft(
            &inbox,
            &f.policy,
            &f.ingested("m4", "never run"),
            "event-4",
            100,
            1_000,
        )
        .unwrap();
    ExternalIdentityRepository::open(&f.path)
        .unwrap()
        .revoke_local(&f.principal, f.account, 0, 105)
        .unwrap();
    let approved = f
        .resolve(&draft, &f.principal, ApprovalDecision::Approve, 110)
        .unwrap();
    assert!(coordinator.confirm(&approved, 111).is_err());
    assert_eq!(
        ObjectiveStore::open(&f.path)
            .unwrap()
            .get_goal(draft.goal.id)
            .unwrap()
            .unwrap()
            .state,
        GoalState::Draft
    );

    let f2 = Fixture::new();
    let inbox = f2.accepted("m5");
    let mut coordinator = GmailGoalDraftCoordinator::open(&f2.path).unwrap();
    let draft = coordinator
        .create_draft(
            &inbox,
            &f2.policy,
            &f2.ingested("m5", "race"),
            "event-5",
            100,
            1_000,
        )
        .unwrap();
    let store = ObjectiveStore::open(&f2.path).unwrap();
    store
        .transition_goal(
            draft.goal.id,
            0,
            GoalState::Cancelled,
            None,
            &serde_json::json!({}),
        )
        .unwrap();
    let approved = f2
        .resolve(&draft, &f2.principal, ApprovalDecision::Approve, 110)
        .unwrap();
    assert!(coordinator.confirm(&approved, 111).is_err());
    assert_eq!(
        store.get_goal(draft.goal.id).unwrap().unwrap().state,
        GoalState::Cancelled
    );
}

struct NoTurn;

#[async_trait::async_trait]
impl ChannelTurnExecutor for NoTurn {
    async fn execute(&self, _: &str, _: &str, _: &str) -> anyhow::Result<String> {
        anyhow::bail!("approval must not invoke a model")
    }
}

#[derive(Default)]
struct CaptureTransport {
    sent: AsyncMutex<Vec<fabric::channel::OutboundMessage>>,
}

#[async_trait::async_trait]
impl ChannelTransport for CaptureTransport {
    fn channel_id(&self) -> &str {
        "telegram"
    }

    async fn receive(&self, _: Option<String>) -> anyhow::Result<Vec<ProviderEnvelope>> {
        Ok(vec![])
    }

    async fn send(&self, message: &fabric::channel::OutboundMessage) -> anyhow::Result<String> {
        self.sent.lock().await.push(message.clone());
        Ok("telegram-message".into())
    }
}

fn callback(id: &str, sender: &str, action: String, now_ms: i64) -> ProviderEnvelope {
    ProviderEnvelope {
        message: fabric::channel::InboundMessage {
            channel_id: fabric::channel::ChannelId("telegram".into()),
            message_id: fabric::channel::MessageId(id.into()),
            conversation_id: fabric::channel::ConversationId("42".into()),
            sender_id: fabric::channel::ExternalSenderId(sender.into()),
            content: fabric::channel::MessageContent::Text {
                text: String::new(),
            },
            timestamp_ms: now_ms,
            reply_to_action: Some(action),
            correlation_id: format!("callback:{id}"),
        },
        next_cursor: id.into(),
    }
}

#[tokio::test]
async fn telegram_review_has_confirm_edit_reject_and_replayed_confirm_is_idempotent() {
    let f = Fixture::new();
    let inbox = f.accepted("m6");
    let coordinator = Arc::new(Mutex::new(
        GmailGoalDraftCoordinator::open(&f.path).unwrap(),
    ));
    let draft = coordinator
        .lock()
        .unwrap()
        .create_draft(
            &inbox,
            &f.policy,
            &f.ingested("m6", "router confirmation"),
            "event-6",
            100,
            1_000,
        )
        .unwrap();
    let channel_path = f._dir.path().join("channels.db");
    let channels = ChannelStore::open(&channel_path).unwrap();
    channels
        .bind("telegram", "telegram:7", "owner", "active")
        .unwrap();
    let approval_repo = coordinator.lock().unwrap().approval_repository();
    let gmail_resolver: Arc<dyn ApprovalResolver> =
        Arc::new(DaemonGmailDraftApprovalExecutor::new(coordinator));
    let mut router = ChannelDispatcher::new(channels, Arc::new(NoTurn))
        .with_approval_repository(approval_repo)
        .with_approval_resolver(ApprovalCategory::ActivateGoal, gmail_resolver);
    let transport = CaptureTransport::default();
    assert!(router
        .notify_approval(
            &transport,
            fabric::channel::ConversationId("42".into()),
            &draft.approval,
            101,
        )
        .await
        .unwrap());
    let sent = transport.sent.lock().await;
    assert_eq!(sent[0].actions.len(), 3);
    assert_eq!(sent[0].actions[0].label, "Confirm");
    assert_eq!(sent[0].actions[1].label, "Edit");
    assert_eq!(sent[0].actions[2].label, "Reject");
    drop(sent);

    let action = format!("{}:confirm", draft.approval.id);
    router
        .process(
            &transport,
            callback("cb1", "telegram:7", action.clone(), 110),
        )
        .await
        .unwrap();
    router
        .process(&transport, callback("cb2", "telegram:7", action, 111))
        .await
        .unwrap();
    assert_eq!(
        ObjectiveStore::open(&f.path)
            .unwrap()
            .get_goal(draft.goal.id)
            .unwrap()
            .unwrap()
            .state,
        GoalState::Ready
    );
}
