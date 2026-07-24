use async_trait::async_trait;
use executive::approval::{ApprovalCreate, ApprovalRepository};
use executive::goal::ObjectiveStore;
use executive::testing::channel::daemon_adapter::ApprovalRepositoryPort;
use fabric::channel::*;
use fabric::*;
use gateway::dispatcher::{
    ChannelDispatcher, ChannelTransport, ChannelTurnExecutor, ProviderEnvelope,
};
use gateway::ChannelStore;
use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use tempfile::{NamedTempFile, TempDir};

struct NoTurn;
#[async_trait]
impl ChannelTurnExecutor for NoTurn {
    async fn execute(&self, _: &str, _: &str, _: &str) -> anyhow::Result<String> {
        panic!("callback must not invoke LLM")
    }
}
struct Transport {
    fail: AtomicBool,
    calls: AtomicUsize,
    sent: Mutex<Vec<OutboundMessage>>,
}
#[async_trait]
impl ChannelTransport for Transport {
    fn channel_id(&self) -> &str {
        "telegram"
    }
    async fn receive(&self, _: Option<String>) -> anyhow::Result<Vec<ProviderEnvelope>> {
        Ok(vec![])
    }
    async fn send(&self, m: &OutboundMessage) -> anyhow::Result<String> {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.sent.lock().unwrap().push(m.clone());
        if self.fail.swap(false, Ordering::SeqCst) {
            anyhow::bail!("temporary send failure")
        }
        Ok("provider-42".into())
    }
}
struct Fixture {
    _objective: NamedTempFile,
    channels: TempDir,
    approval: ApprovalSnapshot,
    repo: Arc<Mutex<ApprovalRepository>>,
}
impl Fixture {
    fn new(expires: i64) -> Self {
        let objective = NamedTempFile::new().unwrap();
        let store = ObjectiveStore::open(objective.path()).unwrap();
        let goal = store
            .create_goal(
                &PrincipalId("owner".into()),
                "s",
                "project",
                &GoalSpec {
                    original_intent: "approve".into(),
                    desired_state: vec![],
                    constraints: vec![],
                    acceptance_criteria: vec![],
                    budget: GoalBudget {
                        max_input_tokens: 1,
                        max_output_tokens: 1,
                        max_cost_usd: None,
                        max_attempts: 1,
                        deadline_ms: None,
                    },
                },
            )
            .unwrap();
        drop(store);
        let repo = Arc::new(Mutex::new(
            ApprovalRepository::open(objective.path()).unwrap(),
        ));
        let approval = repo
            .lock()
            .unwrap()
            .create(ApprovalCreate {
                subject: ApprovalSubject {
                    category: ApprovalCategory::ApplyCode,
                    goal_id: goal.id,
                    attempt_id: None,
                    job_id: None,
                    attributes: BTreeMap::from([
                        ("changed_file_count".into(), "2".into()),
                        (
                            "verification_summary".into(),
                            "required checks passed".into(),
                        ),
                    ]),
                    allowed_scope: vec![PathBuf::from("src")],
                    apply_target: Some(PathBuf::from(".")),
                },
                risk: ApprovalRisk::High,
                summary: "Apply verified diff".into(),
                artifacts: vec![ApprovalArtifactRef {
                    kind: "diff".into(),
                    relative_path: PathBuf::from("coding-diffs/a.diff"),
                    sha256: "a".repeat(64),
                }],
                created_at_ms: 100,
                expires_at_ms: expires,
            })
            .unwrap();
        Self {
            _objective: objective,
            channels: tempfile::tempdir().unwrap(),
            approval,
            repo,
        }
    }
    fn channel_path(&self) -> PathBuf {
        self.channels.path().join("channels.db")
    }
    fn router(&self) -> ChannelDispatcher {
        let store = ChannelStore::open(&self.channel_path()).unwrap();
        store
            .bind("telegram", "telegram:7", "owner", "active")
            .unwrap();
        ChannelDispatcher::new(store, Arc::new(NoTurn))
            .with_approval_port(Arc::new(ApprovalRepositoryPort::new(self.repo.clone())))
    }
    fn callback(&self, message: &str, sender: &str, action: String, time: i64) -> ProviderEnvelope {
        ProviderEnvelope {
            message: InboundMessage {
                channel_id: ChannelId("telegram".into()),
                message_id: fabric::channel::MessageId(message.into()),
                conversation_id: ConversationId("1001".into()),
                sender_id: ExternalSenderId(sender.into()),
                content: MessageContent::Text {
                    text: String::new(),
                },
                timestamp_ms: time,
                reply_to_action: Some(action),
                correlation_id: format!("callback:{message}"),
            },
            next_cursor: message.into(),
        }
    }
}

#[tokio::test]
async fn notification_is_bounded_persisted_and_retried_after_restart() {
    let f = Fixture::new(1000);
    let transport = Transport {
        fail: AtomicBool::new(true),
        calls: AtomicUsize::new(0),
        sent: Mutex::new(vec![]),
    };
    let mut router = f.router();
    assert!(!router
        .notify_approval(&transport, ConversationId("1001".into()), &f.approval, 100)
        .await
        .unwrap());
    let first = transport.sent.lock().unwrap()[0].clone();
    let MessageContent::Text { text } = &first.content else {
        panic!()
    };
    assert!(text.contains("Goal"));
    assert!(text.contains("Changed files: 2"));
    assert!(text.contains("required checks passed"));
    assert!(!text.contains("diff --git"));
    assert_eq!(first.actions.len(), 4);
    for a in &first.actions {
        assert!(a.action_id.starts_with(&f.approval.id.to_string()));
        assert!(!a.action_id.contains("sha256"));
    }
    assert_eq!(
        f.repo
            .lock()
            .unwrap()
            .delivery_for_correlation(&first.correlation_id)
            .unwrap()
            .unwrap()
            .status,
        "failed"
    );
    drop(router);
    let mut restarted = f.router();
    assert_eq!(
        restarted
            .flush_pending_outbox(&transport, 10)
            .await
            .unwrap(),
        1
    );
    let delivery = f
        .repo
        .lock()
        .unwrap()
        .delivery_for_correlation(&first.correlation_id)
        .unwrap()
        .unwrap();
    assert_eq!(delivery.status, "sent");
    assert_eq!(delivery.provider_message_id.as_deref(), Some("provider-42"));
}

#[tokio::test]
async fn owner_callback_is_authoritative_and_duplicate_is_idempotent() {
    let f = Fixture::new(1000);
    let transport = Transport {
        fail: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        sent: Mutex::new(vec![]),
    };
    let mut router = f.router();
    let action = format!("{}:apply", f.approval.id);
    router
        .process(
            &transport,
            f.callback("1", "telegram:7", action.clone(), 150),
        )
        .await
        .unwrap();
    router
        .process(&transport, f.callback("2", "telegram:7", action, 151))
        .await
        .unwrap();
    let resolved = f.repo.lock().unwrap().get(f.approval.id).unwrap().unwrap();
    assert_eq!(resolved.status, ApprovalStatus::Approved);
    assert_eq!(resolved.version, 1);
    assert_eq!(transport.calls.load(Ordering::SeqCst), 2);
}

#[tokio::test]
async fn unknown_expired_and_forged_callbacks_never_approve() {
    let f = Fixture::new(200);
    let transport = Transport {
        fail: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        sent: Mutex::new(vec![]),
    };
    let mut router = f.router();
    router
        .process(
            &transport,
            f.callback("u", "telegram:999", format!("{}:apply", f.approval.id), 150),
        )
        .await
        .unwrap();
    assert_eq!(
        f.repo
            .lock()
            .unwrap()
            .get(f.approval.id)
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Pending
    );
    router
        .process(
            &transport,
            f.callback(
                "x",
                "telegram:7",
                format!("{}:apply", ApprovalId::new()),
                150,
            ),
        )
        .await
        .unwrap();
    assert_eq!(
        f.repo
            .lock()
            .unwrap()
            .get(f.approval.id)
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Pending
    );
    router
        .process(
            &transport,
            f.callback("e", "telegram:7", format!("{}:apply", f.approval.id), 200),
        )
        .await
        .unwrap();
    assert_eq!(
        f.repo
            .lock()
            .unwrap()
            .get(f.approval.id)
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Expired
    );
}

#[tokio::test]
async fn view_diff_returns_only_bounded_trusted_reference() {
    let f = Fixture::new(1000);
    let transport = Transport {
        fail: AtomicBool::new(false),
        calls: AtomicUsize::new(0),
        sent: Mutex::new(vec![]),
    };
    let mut router = f.router();
    router
        .process(
            &transport,
            f.callback(
                "v",
                "telegram:7",
                format!("{}:view_diff", f.approval.id),
                150,
            ),
        )
        .await
        .unwrap();
    let sent = transport.sent.lock().unwrap();
    let MessageContent::Text { text } = &sent[0].content else {
        panic!()
    };
    assert!(text.contains("coding-diffs/a.diff"));
    assert!(text.contains(&"a".repeat(64)));
    assert!(!text.contains("diff --git"));
    assert_eq!(
        f.repo
            .lock()
            .unwrap()
            .get(f.approval.id)
            .unwrap()
            .unwrap()
            .status,
        ApprovalStatus::Pending
    );
}
