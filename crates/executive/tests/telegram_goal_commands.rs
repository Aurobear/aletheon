use std::sync::Arc;

use executive::r#impl::channel::daemon_adapter::DaemonChannelGoalExecutor;
use executive::r#impl::channel::dispatcher::{
    ChannelDispatcher, ChannelTransport, ChannelTurnExecutor, ProviderEnvelope,
};
use executive::r#impl::channel::store::ChannelStore;
use executive::r#impl::goal::ObjectiveStore;
use fabric::channel::{
    ChannelId, ConversationId, ExternalSenderId, InboundMessage, MessageContent, MessageId,
    OutboundMessage,
};
use tokio::sync::Mutex;

struct NoTurn;
#[async_trait::async_trait]
impl ChannelTurnExecutor for NoTurn {
    async fn execute(&self, _: &str, _: &str, _: &str) -> anyhow::Result<String> {
        anyhow::bail!("unexpected chat turn")
    }
}

#[derive(Default)]
struct CaptureTransport {
    sent: Mutex<Vec<OutboundMessage>>,
}
#[async_trait::async_trait]
impl ChannelTransport for CaptureTransport {
    fn channel_id(&self) -> &str {
        "telegram"
    }
    async fn receive(&self, _: Option<String>) -> anyhow::Result<Vec<ProviderEnvelope>> {
        Ok(vec![])
    }
    async fn send(&self, message: &OutboundMessage) -> anyhow::Result<String> {
        self.sent.lock().await.push(message.clone());
        Ok("sent".into())
    }
}

fn command(id: &str, sender: &str, name: &str, args: &str) -> ProviderEnvelope {
    ProviderEnvelope {
        message: InboundMessage {
            channel_id: ChannelId("telegram".into()),
            message_id: MessageId(id.into()),
            conversation_id: ConversationId("42".into()),
            sender_id: ExternalSenderId(sender.into()),
            content: MessageContent::Command {
                command: name.into(),
                args: args.into(),
            },
            timestamp_ms: 0,
            reply_to_action: None,
            correlation_id: format!("telegram:{id}"),
        },
        next_cursor: id.into(),
    }
}

async fn setup() -> (
    ChannelDispatcher,
    CaptureTransport,
    Arc<Mutex<ObjectiveStore>>,
    tempfile::TempDir,
) {
    let dir = tempfile::tempdir().unwrap();
    let channels = ChannelStore::open(&dir.path().join("channels.db")).unwrap();
    channels
        .bind("telegram", "telegram:owner", "owner", "active")
        .unwrap();
    channels
        .bind("telegram", "telegram:other", "other", "active")
        .unwrap();
    let goals = Arc::new(Mutex::new(
        ObjectiveStore::open(&dir.path().join("objectives.db")).unwrap(),
    ));
    let router = ChannelDispatcher::new(channels, Arc::new(NoTurn))
        .with_goal_executor(Arc::new(DaemonChannelGoalExecutor::new(goals.clone())));
    (router, CaptureTransport::default(), goals, dir)
}

fn text(message: &OutboundMessage) -> &str {
    match &message.content {
        MessageContent::Text { text } => text,
        _ => panic!("expected text"),
    }
}

#[tokio::test]
async fn owner_can_create_confirm_inspect_pause_resume_and_cancel_goal() {
    let (mut router, transport, goals, _dir) = setup().await;
    router
        .process(
            &transport,
            command("1", "telegram:owner", "/goal", "ship feature"),
        )
        .await
        .unwrap();
    assert!(text(&transport.sent.lock().await[0]).contains("created as draft"));
    router
        .process(&transport, command("2", "telegram:owner", "/status", "1"))
        .await
        .unwrap();
    assert!(text(&transport.sent.lock().await[1]).contains("draft"));
    router
        .process(&transport, command("3", "telegram:owner", "/resume", "1"))
        .await
        .unwrap();
    assert!(text(&transport.sent.lock().await[2]).contains("ready"));
    goals
        .lock()
        .await
        .transition_goal(
            fabric::GoalId(1),
            1,
            fabric::GoalState::Running,
            None,
            &serde_json::json!({}),
        )
        .unwrap();
    router
        .process(&transport, command("4", "telegram:owner", "/pause", "1"))
        .await
        .unwrap();
    assert!(text(&transport.sent.lock().await[3]).contains("suspended"));
    router
        .process(&transport, command("5", "telegram:owner", "/resume", "1"))
        .await
        .unwrap();
    assert!(text(&transport.sent.lock().await[4]).contains("ready"));
    router
        .process(&transport, command("6", "telegram:owner", "/cancel", "1"))
        .await
        .unwrap();
    assert!(text(&transport.sent.lock().await[5]).contains("cancelled"));
    router
        .process(&transport, command("7", "telegram:owner", "/goals", ""))
        .await
        .unwrap();
    assert!(text(&transport.sent.lock().await[6]).contains("ship feature"));
}

#[tokio::test]
async fn commands_validate_owner_arguments_replay_and_single_active_goal() {
    let (mut router, transport, _goals, _dir) = setup().await;
    let first = command("1", "telegram:owner", "/goal", "one");
    router.process(&transport, first).await.unwrap();
    router
        .process(&transport, command("1", "telegram:owner", "/goal", "one"))
        .await
        .unwrap();
    assert_eq!(
        transport.sent.lock().await.len(),
        1,
        "replay must not send twice"
    );
    router
        .process(&transport, command("2", "telegram:owner", "/goal", "two"))
        .await
        .unwrap();
    assert!(text(&transport.sent.lock().await[1]).contains("active goal already exists"));
    router
        .process(&transport, command("3", "telegram:other", "/status", "1"))
        .await
        .unwrap();
    assert_eq!(text(&transport.sent.lock().await[2]), "goal not found");
    router
        .process(&transport, command("4", "telegram:owner", "/status", "bad"))
        .await
        .unwrap();
    assert!(text(&transport.sent.lock().await[3]).contains("usage: /status <goal-id>"));
    router
        .process(&transport, command("5", "telegram:owner", "/goal", ""))
        .await
        .unwrap();
    assert!(text(&transport.sent.lock().await[4]).contains("usage: /goal <intent>"));
}
