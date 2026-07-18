use corpus::tools::google::oauth::GoogleBinding;
use gateway::store::ChannelStore;
use executive::r#impl::external::ExternalIdentityRepository;
use executive::r#impl::goal::coordinator::{GoalCoordinator, GoogleEventWaitCondition};
use executive::r#impl::goal::ObjectiveStore;
use executive::r#impl::google::{
    DurableGoogleNotificationSink, GoogleCurrentTaskProjection, GoogleEventDispatcher,
    GoogleEventRouter, GoogleMemoryProposalSink, GoogleSubscription, GoogleSubscriptionQuery,
    GoogleSyncStore, ProjectionWrite, SyncCommit, SyncStream,
};
use fabric::goal::{GoalBudget, GoalSpec, GoalState, GoalWaitReason};
use fabric::{
    ExternalEventDraft, ExternalEventEnvelope, ExternalEventId, ExternalIdentityId,
    ExternalObjectRef, ExternalScope, GmailMessageSummary, GoogleEvent, IdentityProvider,
    MailChange, PrincipalId, ProviderRecordRef,
};
use std::collections::HashSet;
use std::sync::{Arc, Mutex};
use tokio_util::sync::CancellationToken;

#[derive(Default)]
struct TaskProjection {
    seen: Mutex<HashSet<ExternalEventId>>,
}

impl GoogleCurrentTaskProjection for TaskProjection {
    fn project_current_task(
        &self,
        _principal: &PrincipalId,
        _task_id: &str,
        event: &ExternalEventEnvelope,
    ) -> Result<(), String> {
        self.seen.lock().unwrap().insert(event.id);
        Ok(())
    }
}

#[derive(Default)]
struct MemoryProposals {
    seen: Mutex<HashSet<ExternalEventId>>,
    provenance: Mutex<Vec<ProviderRecordRef>>,
}

impl GoogleMemoryProposalSink for MemoryProposals {
    fn propose_with_provenance(
        &self,
        _principal: &PrincipalId,
        event: &ExternalEventEnvelope,
    ) -> Result<(), String> {
        if self.seen.lock().unwrap().insert(event.id) {
            self.provenance
                .lock()
                .unwrap()
                .push(event.provenance.clone());
        }
        Ok(())
    }
}

struct Fixture {
    _dir: tempfile::TempDir,
    google_path: std::path::PathBuf,
    channel_path: std::path::PathBuf,
    account: ExternalIdentityId,
    principal: PrincipalId,
}

impl Fixture {
    fn new() -> Self {
        let dir = tempfile::tempdir().unwrap();
        let google_path = dir.path().join("objectives.db");
        let channel_path = dir.path().join("channels.db");
        let account = ExternalIdentityId::new();
        let principal = PrincipalId("owner".into());
        ExternalIdentityRepository::open(&google_path)
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
        Self {
            _dir: dir,
            google_path,
            channel_path,
            account,
            principal,
        }
    }

    fn event(&self, version: &str, source_ms: i64) -> ExternalEventEnvelope {
        let object = ExternalObjectRef {
            provider: IdentityProvider::Google,
            account_id: self.account,
            object_id: "message-1".into(),
            object_version: version.into(),
        };
        let source = ProviderRecordRef {
            account_id: self.account,
            provider_object_id: "message-1".into(),
            fetched_at_ms: source_ms + 1,
            source_timestamp_ms: source_ms,
            etag_or_history: Some(version.into()),
        };
        ExternalEventEnvelope::from_draft(ExternalEventDraft {
            provider: IdentityProvider::Google,
            account_id: self.account,
            provider_event_id: Some(format!("history-{version}")),
            object,
            observed_at_ms: source_ms + 1,
            source_timestamp_ms: source_ms,
            provenance: source.clone(),
            event: GoogleEvent::MailReceived(MailChange {
                message: GmailMessageSummary {
                    source,
                    thread_id: "thread".into(),
                    subject: "important subject".into(),
                    from: "sender@example.com".into(),
                    snippet: "snippet".into(),
                    unread: true,
                    important: true,
                },
                content: None,
            }),
        })
        .unwrap()
    }

    fn subscription(
        &self,
        id: &str,
        object_id: Option<&str>,
        conversation: Option<&str>,
        task: Option<&str>,
        memory: bool,
    ) -> GoogleSubscription {
        GoogleSubscription {
            subscription_id: id.into(),
            principal_id: self.principal.clone(),
            account_id: self.account,
            stream: SyncStream::GmailHistory,
            event_kinds: vec!["mail_received".into()],
            query: GoogleSubscriptionQuery {
                object_id: object_id.map(str::to_owned),
                important_only: true,
                source_after_ms: Some(500),
                source_before_ms: Some(2_000),
                telegram_conversation_id: conversation.map(str::to_owned),
                current_task_id: task.map(str::to_owned),
                propose_memory: memory,
            },
            cursor_generation: 1,
            state: "active".into(),
            version: 0,
            created_at_ms: 1,
            updated_at_ms: 1,
        }
    }
}

fn goal_spec() -> GoalSpec {
    GoalSpec {
        original_intent: "wait for mail".into(),
        desired_state: vec!["mail arrived".into()],
        constraints: Vec::new(),
        acceptance_criteria: Vec::new(),
        budget: GoalBudget::default(),
    }
}

#[tokio::test]
async fn subscriptions_route_once_wake_explicit_goals_and_keep_memory_as_proposal() {
    let fixture = Fixture::new();
    let objective_store = Arc::new(Mutex::new(
        ObjectiveStore::open(&fixture.google_path).unwrap(),
    ));
    let goal = objective_store
        .lock()
        .unwrap()
        .create_goal(&fixture.principal, "session", "project", &goal_spec())
        .unwrap();
    let running = objective_store
        .lock()
        .unwrap()
        .transition_goal(
            goal.id,
            goal.version,
            GoalState::Running,
            None,
            &serde_json::json!({"test":true}),
        )
        .unwrap();
    let condition = GoogleEventWaitCondition {
        account_id: fixture.account,
        event_id: None,
        object_id: Some("message-1".into()),
        source_after_ms: Some(500),
        source_before_ms: Some(2_000),
    };
    objective_store
        .lock()
        .unwrap()
        .transition_goal(
            goal.id,
            running.version,
            GoalState::Suspended,
            Some(&GoalWaitReason::ExternalEvent {
                key: condition.key().unwrap(),
            }),
            &serde_json::json!({"test":true}),
        )
        .unwrap();
    let terminal = objective_store
        .lock()
        .unwrap()
        .create_goal(&fixture.principal, "session", "project", &goal_spec())
        .unwrap();
    let terminal_running = objective_store
        .lock()
        .unwrap()
        .transition_goal(
            terminal.id,
            terminal.version,
            GoalState::Running,
            None,
            &serde_json::json!({"test":true}),
        )
        .unwrap();
    objective_store
        .lock()
        .unwrap()
        .transition_goal(
            terminal.id,
            terminal_running.version,
            GoalState::Completed,
            Some(&GoalWaitReason::ExternalEvent {
                key: condition.key().unwrap(),
            }),
            &serde_json::json!({"test":true}),
        )
        .unwrap();

    let google_store = Arc::new(Mutex::new(
        GoogleSyncStore::open(&fixture.google_path).unwrap(),
    ));
    let cursor = google_store
        .lock()
        .unwrap()
        .initialize_cursor(fixture.account, SyncStream::GmailHistory, Some("h1"), 1)
        .unwrap();
    for subscription in [
        fixture.subscription("unmatched", Some("other"), Some("chat"), None, false),
        fixture.subscription("notify-task", None, Some("chat"), Some("task-1"), false),
        fixture.subscription("memory", None, None, None, true),
    ] {
        google_store
            .lock()
            .unwrap()
            .put_subscription(&subscription, None)
            .unwrap();
    }
    let event = fixture.event("v1", 1_000);
    google_store
        .lock()
        .unwrap()
        .commit(SyncCommit {
            account_id: fixture.account,
            stream: SyncStream::GmailHistory,
            expected_cursor_token: cursor.token,
            expected_cursor_version: cursor.version,
            successor_cursor_token: "h2".into(),
            cursor_generation: 1,
            events: vec![(
                event.clone(),
                ProjectionWrite {
                    json: serde_json::json!({"version":"v1"}),
                    tombstone: false,
                },
            )],
            committed_at_ms: 1_001,
        })
        .unwrap();

    let notifications = Arc::new(DurableGoogleNotificationSink::open(&fixture.channel_path).unwrap());
    let tasks = Arc::new(TaskProjection::default());
    let memories = Arc::new(MemoryProposals::default());
    let router = Arc::new(
        GoogleEventRouter::new_with_notifications(
            google_store.clone(),
            Arc::new(GoalCoordinator::new(objective_store.clone())),
            notifications,
        )
        .with_current_tasks(tasks.clone())
        .with_memory_proposals(memories.clone()),
    );
    let dispatcher =
        GoogleEventDispatcher::new(google_store.clone(), router, "dispatcher".into(), 1_000)
            .unwrap();
    assert_eq!(
        dispatcher
            .dispatch_due(2_000, 10, &CancellationToken::new())
            .await
            .unwrap()
            .delivered,
        1
    );
    assert_eq!(
        objective_store
            .lock()
            .unwrap()
            .get_goal(goal.id)
            .unwrap()
            .unwrap()
            .state,
        GoalState::Ready
    );
    assert_eq!(
        objective_store
            .lock()
            .unwrap()
            .get_goal(terminal.id)
            .unwrap()
            .unwrap()
            .state,
        GoalState::Completed
    );
    assert_eq!(
        ChannelStore::open(&fixture.channel_path)
            .unwrap()
            .outbox_count("telegram")
            .unwrap(),
        1
    );
    assert_eq!(tasks.seen.lock().unwrap().len(), 1);
    assert_eq!(memories.seen.lock().unwrap().len(), 1);
    assert_eq!(memories.provenance.lock().unwrap()[0], event.provenance);

    let channel = ChannelStore::open(&fixture.channel_path).unwrap();
    channel
        .mark_outbound_failed(&event.id.to_string(), "offline")
        .unwrap();
    assert_eq!(channel.pending_outbox("telegram", 10).unwrap().len(), 1);

    // Delivered outbox rows and channel correlation IDs make retries harmless.
    assert_eq!(
        dispatcher
            .dispatch_due(3_000, 10, &CancellationToken::new())
            .await
            .unwrap()
            .claimed,
        0
    );
}

#[tokio::test]
async fn stale_updates_and_revoked_accounts_do_not_rewrite_or_route() {
    let fixture = Fixture::new();
    let store = GoogleSyncStore::open(&fixture.google_path).unwrap();
    let first = store
        .initialize_cursor(fixture.account, SyncStream::GmailHistory, Some("h1"), 1)
        .unwrap();
    let newer = fixture.event("v2", 2_000);
    let next = store
        .commit(SyncCommit {
            account_id: fixture.account,
            stream: SyncStream::GmailHistory,
            expected_cursor_token: first.token,
            expected_cursor_version: first.version,
            successor_cursor_token: "h2".into(),
            cursor_generation: 1,
            events: vec![(
                newer,
                ProjectionWrite {
                    json: serde_json::json!({"version":"v2"}),
                    tombstone: false,
                },
            )],
            committed_at_ms: 2_001,
        })
        .unwrap();
    store
        .commit(SyncCommit {
            account_id: fixture.account,
            stream: SyncStream::GmailHistory,
            expected_cursor_token: next.cursor.token,
            expected_cursor_version: next.cursor.version,
            successor_cursor_token: "h3".into(),
            cursor_generation: 1,
            events: vec![(
                fixture.event("v1", 1_000),
                ProjectionWrite {
                    json: serde_json::json!({"version":"v1"}),
                    tombstone: false,
                },
            )],
            committed_at_ms: 2_002,
        })
        .unwrap();
    let projection = store
        .projection(fixture.account, SyncStream::GmailHistory, "message-1")
        .unwrap()
        .unwrap();
    assert!(projection.0.contains("v2"));

    let identity = ExternalIdentityRepository::open(&fixture.google_path)
        .unwrap()
        .get(&fixture.principal, fixture.account)
        .unwrap()
        .unwrap()
        .0;
    ExternalIdentityRepository::open(&fixture.google_path)
        .unwrap()
        .revoke_local(&fixture.principal, fixture.account, identity.version, 3_000)
        .unwrap();
    assert!(!store.account_is_active(fixture.account).unwrap());
}
