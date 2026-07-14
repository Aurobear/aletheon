# Agent-Google Integration — Implementation Plan

> **For agentic workers:** Use `/workflow feature` to implement this plan phase-by-phase. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Build the 8-phase Aletheon agent-Google integration: Telegram channel, Goal Runtime, DeepSeek/Pi workers, verification pipeline, Google OAuth+sync, Gmail channel+approval, and GBrain backend.

**Architecture:** Integrate into existing crates (executive, corpus, mnemosyne, fabric). Extend existing `ObjectiveStore`/`AgentRuntime`/`ModelRouter` rather than greenfield. Channel abstraction layer routes external messages through existing `DaemonTurnOrchestrator`.

**Tech Stack:** Rust (tokio async), rusqlite (SQLite), reqwest (HTTP), teloxide (Telegram), AES-256-GCM (credential vault), Docker Compose (GBrain+PostgreSQL).

**Spec:** `docs/plans/2026-07-14-agent-google-design.md`

**Phase dependency graph:**
```
A ──→ B ──→ C ──→ D ──→ E
              │
              └──→ F ──→ G
                        │
              └──→ H ──┘
```
---

# Phase A: Channel Core + Telegram (estimated 1-2 weeks, crates: fabric, executive)

## Task A.1: Add channel types to fabric

### A.1.1 Create `crates/fabric/src/types/channel.rs`

- [ ] Create the file with the following complete content.

```rust
//! Channel abstraction types — inbound/outbound message contracts for
//! external communication surfaces (Telegram, Gmail, etc.).
//!
//! These types form the ABI between channel implementations and the
//! daemon routing layer.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for a channel instance (e.g. "telegram", "gmail").
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub String);

impl ChannelId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl fmt::Display for ChannelId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Unique identifier for a message within a channel.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

impl MessageId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl fmt::Display for MessageId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Groups messages into a conversation thread within a channel.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId(pub String);

impl ConversationId {
    pub fn new(id: impl Into<String>) -> Self {
        Self(id.into())
    }
}

impl fmt::Display for ConversationId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

/// Content payload variants for inbound/outbound messages.
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum MessageContent {
    /// Plain text message.
    Text { text: String },
    /// A command issued by the user (e.g. /goal, /status, /pause).
    Command { command: String, args: Vec<String> },
    /// A file attachment (name + MIME type + base64 body).
    File { name: String, mime_type: String, data_base64: String },
    /// Voice message with transcription.
    Voice { transcription: Option<String>, audio_base64: String },
}

/// An action a user can perform on a message sent by the agent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserAction {
    pub action_id: String,
    pub action_type: ActionType,
    pub label: String,
    pub payload: Option<String>,
}

/// Kinds of actions the user can take.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Approve,
    Reject,
    ViewDiff,
    RequestRevision,
    Custom(String),
}

/// An inbound message received from an external channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InboundMessage {
    /// Channel this message came from.
    pub channel_id: ChannelId,
    /// Unique message identifier within the channel.
    pub message_id: MessageId,
    /// Conversation thread this belongs to (optional).
    pub conversation_id: Option<ConversationId>,
    /// The sender identifier (e.g. Telegram user id, email address).
    pub sender_id: String,
    /// The content of the message.
    pub content: MessageContent,
    /// When the message was received (ISO 8601).
    pub timestamp: String,
    /// If this is a reply to an agent action, the action id.
    pub reply_to_action: Option<String>,
}

/// An outbound message to be sent through an external channel.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OutboundMessage {
    /// Target channel.
    pub channel_id: ChannelId,
    /// Optional conversation thread to post in.
    pub conversation_id: Option<ConversationId>,
    /// The message content to send.
    pub content: MessageContent,
    /// Optional actions the recipient can take (buttons).
    pub actions: Vec<UserAction>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn message_content_serde_roundtrip() {
        // Text variant
        let text = MessageContent::Text { text: "hello world".into() };
        let json = serde_json::to_string(&text).unwrap();
        let back: MessageContent = serde_json::from_str(&json).unwrap();
        match back {
            MessageContent::Text { text: t } => assert_eq!(t, "hello world"),
            _ => panic!("expected Text variant"),
        }

        // Command variant
        let cmd = MessageContent::Command {
            command: "goal".into(),
            args: vec!["build auth system".into()],
        };
        let json = serde_json::to_string(&cmd).unwrap();
        let back: MessageContent = serde_json::from_str(&json).unwrap();
        match back {
            MessageContent::Command { command, args } => {
                assert_eq!(command, "goal");
                assert_eq!(args, vec!["build auth system"]);
            }
            _ => panic!("expected Command variant"),
        }

        // File variant
        let file = MessageContent::File {
            name: "diff.patch".into(),
            mime_type: "text/x-patch".into(),
            data_base64: "YXNkZg==".into(),
        };
        let json = serde_json::to_string(&file).unwrap();
        let back: MessageContent = serde_json::from_str(&json).unwrap();
        match back {
            MessageContent::File { name, mime_type, data_base64 } => {
                assert_eq!(name, "diff.patch");
                assert_eq!(mime_type, "text/x-patch");
            }
            _ => panic!("expected File variant"),
        }

        // Voice variant
        let voice = MessageContent::Voice {
            transcription: Some("please review PR #42".into()),
            audio_base64: "bXVzaWM=".into(),
        };
        let json = serde_json::to_string(&voice).unwrap();
        let back: MessageContent = serde_json::from_str(&json).unwrap();
        match back {
            MessageContent::Voice { transcription, .. } => {
                assert_eq!(transcription, Some("please review PR #42".into()));
            }
            _ => panic!("expected Voice variant"),
        }
    }

    #[test]
    fn outbound_actions_serialize() {
        let msg = OutboundMessage {
            channel_id: ChannelId::new("telegram"),
            conversation_id: Some(ConversationId::new("chat-42")),
            content: MessageContent::Text { text: "Shall I deploy?".into() },
            actions: vec![
                UserAction {
                    action_id: "act-1".into(),
                    action_type: ActionType::Approve,
                    label: "Approve".into(),
                    payload: None,
                },
                UserAction {
                    action_id: "act-2".into(),
                    action_type: ActionType::Reject,
                    label: "Reject".into(),
                    payload: Some("needs more tests".into()),
                },
                UserAction {
                    action_id: "act-3".into(),
                    action_type: ActionType::ViewDiff,
                    label: "View Diff".into(),
                    payload: Some("/tmp/pr-42.diff".into()),
                },
            ],
        };

        let json = serde_json::to_string_pretty(&msg).unwrap();
        assert!(json.contains("Approve"));
        assert!(json.contains("Reject"));
        assert!(json.contains("View Diff"));
        assert!(json.contains("Shall I deploy?"));
    }
}
```

### A.1.2 Modify `crates/fabric/src/types/mod.rs`

- [ ] Add `pub mod channel;` after the existing module declarations (e.g., after line 31 `pub mod time;`):

```rust
pub mod channel;
```

### A.1.3 Modify `crates/fabric/src/lib.rs`

- [ ] Add re-export after the existing `pub use types::...` block (e.g., after line 67):

```rust
pub use types::channel;
```

### A.1.4 Commit

- [ ] Run:
```bash
git add crates/fabric/src/types/channel.rs crates/fabric/src/types/mod.rs crates/fabric/src/lib.rs
git commit -m "feat(fabric): add channel abstraction types (ChannelId, MessageContent, InboundMessage, OutboundMessage, UserAction)"
```

## Task A.2: Define Channel trait and ChannelRegistry

### A.2.1 Create `crates/executive/src/impl/channel/mod.rs`

- [ ] Create the directory `crates/executive/src/impl/channel/` and the file `mod.rs`:

```rust
//! Channel abstraction layer — trait and registry for external communication
//! channels (Telegram, Gmail, etc.).
//!
//! The Channel trait defines the lifecycle contract that every external
//! channel must fulfill. ChannelRegistry provides a central multiplexer
//! that the daemon uses to send and receive messages across all channels.

use anyhow::Result;
use async_trait::async_trait;
use fabric::types::channel::{ChannelId, InboundMessage, OutboundMessage};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::sync::Mutex;
use tracing::{error, info, warn};

/// The Channel trait defines the lifecycle contract for external
/// communication surfaces such as Telegram bots, Gmail watchers, etc.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Return the unique ChannelId for this channel instance.
    fn id(&self) -> ChannelId;

    /// Start listening for inbound messages. The channel should begin
    /// forwarding inbound messages to the provided sender.
    async fn start(&self, inbound_tx: mpsc::Sender<InboundMessage>) -> Result<()>;

    /// Send an outbound message through this channel.
    async fn send(&self, msg: OutboundMessage) -> Result<()>;

    /// Gracefully stop the channel (close connections, flush buffers).
    async fn stop(&self) -> Result<()>;
}

/// Central registry that multiplexes outbound messages to the correct
/// channel and funnels inbound messages to the daemon processing loop.
pub struct ChannelRegistry {
    channels: Mutex<HashMap<ChannelId, Arc<dyn Channel>>>,
    /// The sender half used by each registered channel to push inbound
    /// messages. The receiver half is held by the daemon processing loop.
    inbound_tx: Mutex<Option<mpsc::Sender<InboundMessage>>>,
}

impl ChannelRegistry {
    pub fn new() -> Self {
        Self {
            channels: Mutex::new(HashMap::new()),
            inbound_tx: Mutex::new(None),
        }
    }

    /// Register a new channel. Registration does NOT start the channel;
    /// call `start_all` or start individual channels manually.
    pub async fn register(&self, channel: Arc<dyn Channel>) -> Result<()> {
        let id = channel.id();
        let mut guard = self.channels.lock().await;
        if guard.contains_key(&id) {
            warn!(channel_id = %id, "channel already registered, overwriting");
        }
        guard.insert(id.clone(), channel);
        info!(channel_id = %id, "channel registered");
        Ok(())
    }

    /// Set the inbound sender. All registered channels will deliver
    /// inbound messages to this sender after `start` is called.
    pub async fn set_inbound_sender(&self, tx: mpsc::Sender<InboundMessage>) {
        let mut guard = self.inbound_tx.lock().await;
        *guard = Some(tx);
    }

    /// Send an outbound message to the channel specified in the message.
    pub async fn send(&self, msg: OutboundMessage) -> Result<()> {
        let guard = self.channels.lock().await;
        let channel = guard
            .get(&msg.channel_id)
            .ok_or_else(|| {
                anyhow::anyhow!("no channel registered for id: {}", msg.channel_id)
            })?;
        channel.send(msg).await
    }

    /// Broadcast an outbound message to ALL registered channels.
    pub async fn broadcast(&self, msg: &OutboundMessage) {
        let guard = self.channels.lock().await;
        for (id, channel) in guard.iter() {
            let mut per_channel = msg.clone();
            per_channel.channel_id = id.clone();
            if let Err(e) = channel.send(per_channel).await {
                error!(channel_id = %id, error = %e, "broadcast send failed");
            }
        }
    }

    /// Start all registered channels and shut them down gracefully.
    pub async fn shutdown(&self) {
        let guard = self.channels.lock().await;
        for (id, channel) in guard.iter() {
            if let Err(e) = channel.stop().await {
                error!(channel_id = %id, error = %e, "channel stop failed during shutdown");
            }
        }
        info!("all channels shut down");
    }
}

impl Default for ChannelRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    /// Stub channel for unit testing the registry.
    struct StubChannel {
        id: ChannelId,
        sent: Mutex<Vec<OutboundMessage>>,
        should_fail: bool,
    }

    impl StubChannel {
        fn new(id: &str, should_fail: bool) -> Self {
            Self {
                id: ChannelId::new(id),
                sent: Mutex::new(Vec::new()),
                should_fail,
            }
        }
    }

    #[async_trait]
    impl Channel for StubChannel {
        fn id(&self) -> ChannelId {
            self.id.clone()
        }

        async fn start(&self, _inbound_tx: mpsc::Sender<InboundMessage>) -> Result<()> {
            Ok(())
        }

        async fn send(&self, msg: OutboundMessage) -> Result<()> {
            if self.should_fail {
                anyhow::bail!("stub channel {}: simulated send failure", self.id);
            }
            self.sent.lock().await.push(msg);
            Ok(())
        }

        async fn stop(&self) -> Result<()> {
            Ok(())
        }
    }

    #[tokio::test]
    async fn registry_send_routes_to_correct_channel() {
        let registry = ChannelRegistry::new();
        let ch_a = Arc::new(StubChannel::new("ch-a", false));
        let ch_b = Arc::new(StubChannel::new("ch-b", false));
        registry.register(ch_a.clone()).await.unwrap();
        registry.register(ch_b.clone()).await.unwrap();

        let msg = OutboundMessage {
            channel_id: ChannelId::new("ch-a"),
            conversation_id: None,
            content: fabric::types::channel::MessageContent::Text {
                text: "hello from A".into(),
            },
            actions: vec![],
        };
        registry.send(msg).await.unwrap();

        assert_eq!(ch_a.sent.lock().await.len(), 1);
        assert_eq!(ch_b.sent.lock().await.len(), 0);
    }

    #[tokio::test]
    async fn send_to_unknown_channel_errors() {
        let registry = ChannelRegistry::new();
        let msg = OutboundMessage {
            channel_id: ChannelId::new("nonexistent"),
            conversation_id: None,
            content: fabric::types::channel::MessageContent::Text {
                text: "nobody will see this".into(),
            },
            actions: vec![],
        };
        let result = registry.send(msg).await;
        assert!(result.is_err());
        assert!(result.unwrap_err().to_string().contains("nonexistent"));
    }
}
```

### A.2.2 Modify `crates/executive/src/impl/mod.rs`

- [ ] Add `pub mod channel;` after the existing module declarations (e.g., after line 7 `pub mod goal;`):

```rust
pub mod channel;
```

### A.2.3 Commit

- [ ] Run:
```bash
mkdir -p crates/executive/src/impl/channel
git add crates/executive/src/impl/channel/mod.rs crates/executive/src/impl/mod.rs
git commit -m "feat(executive): add Channel trait and ChannelRegistry for external communication multiplexing"
```


## Task A.3: Implement TelegramChannel

### A.3.1 Create directory structure

- [ ] Run:
```bash
mkdir -p crates/executive/src/impl/channel/telegram
```

### A.3.2 Create `crates/executive/src/impl/channel/telegram/mod.rs`

- [ ] Create the file with the following complete content:

```rust
//! Telegram channel implementation using the teloxide library.
//!
//! Provides a TelegramChannel that implements the Channel trait and
//! connects the Telegram Bot API to the daemon routing layer.

mod binding;
mod formatting;
mod polling;

use anyhow::{Context, Result};
use async_trait::async_trait;
use fabric::types::channel::{ChannelId, InboundMessage, OutboundMessage};
use std::sync::Arc;
use teloxide::Bot;
use tokio::sync::mpsc;
use tracing::info;

use super::Channel;
use self::polling::start_polling;

/// Telegram-specific channel implementation.
pub struct TelegramChannel {
    bot: Bot,
    /// If set, only messages from this Telegram user ID are processed.
    /// When None, any authenticated user can interact.
    owner_telegram_id: Option<i64>,
}

impl TelegramChannel {
    /// Create a new TelegramChannel.
    ///
    /// `token` is the Telegram bot token obtained from @BotFather.
    /// `owner_telegram_id` optionally restricts the bot to a single owner.
    pub fn new(token: &str, owner_telegram_id: Option<i64>) -> Self {
        Self {
            bot: Bot::new(token),
            owner_telegram_id,
        }
    }

    pub fn bot(&self) -> &Bot {
        &self.bot
    }
}

#[async_trait]
impl Channel for TelegramChannel {
    fn id(&self) -> ChannelId {
        ChannelId::new("telegram")
    }

    async fn start(&self, inbound_tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        info!("starting Telegram channel");
        let bot = self.bot.clone();
        let owner_id = self.owner_telegram_id;
        tokio::spawn(async move {
            if let Err(e) = start_polling(bot, inbound_tx, owner_id).await {
                tracing::error!(error = %e, "Telegram polling loop exited");
            }
        });
        Ok(())
    }

    async fn send(&self, msg: OutboundMessage) -> Result<()> {
        // Convert OutboundMessage to Telegram format and send.
        // The conversation_id encodes the Telegram chat_id.
        let chat_id: i64 = msg
            .conversation_id
            .as_ref()
            .map(|c| c.0.parse())
            .transpose()
            .context("invalid conversation_id: must be a Telegram chat_id")?
            .unwrap_or_else(|| {
                // If no conversation_id, we cannot send. In production,
                // the daemon should track active chats per sender.
                panic!("TelegramChannel::send requires conversation_id");
            });

        let (text, markup) = formatting::to_telegram(&msg);

        if let Some(reply_markup) = markup {
            self.bot
                .send_message(chat_id, text)
                .reply_markup(reply_markup)
                .await
                .context("failed to send Telegram message")?;
        } else {
            self.bot
                .send_message(chat_id, text)
                .await
                .context("failed to send Telegram message")?;
        }
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("stopping Telegram channel");
        // The polling loop will exit when the bot is dropped or the
        // cancellation token fires. No explicit shutdown needed.
        Ok(())
    }
}
```

### A.3.3 Create `crates/executive/src/impl/channel/telegram/polling.rs`

- [ ] Create the file with the following complete content:

```rust
//! Telegram long-polling loop using teloxide Dispatcher.

use anyhow::Result;
use fabric::types::channel::{ChannelId, ConversationId, InboundMessage, MessageContent, MessageId};
use teloxide::{
    dispatching::{dialogue::InMemStorage, Dispatcher, UpdateFilterExt, UpdateHandler},
    prelude::*,
    types::{CallbackQuery, Message, Update},
};
use tokio::sync::mpsc;
use tracing::{debug, warn};

/// Start the Telegram long-polling loop.
///
/// Inbound messages are forwarded through `inbound_tx`. When `owner_id` is
/// set, messages from other users are silently ignored.
pub async fn start_polling(
    bot: Bot,
    inbound_tx: mpsc::Sender<InboundMessage>,
    owner_id: Option<i64>,
) -> Result<()> {
    let handler = UpdateHandler::new()
        .branch(Update::filter_message().endpoint({
            let tx = inbound_tx.clone();
            move |bot: Bot, msg: Message| {
                let tx = tx.clone();
                async move {
                    handle_message(bot, msg, tx.clone(), owner_id).await;
                }
            }
        }))
        .branch(Update::filter_callback_query().endpoint({
            let tx = inbound_tx.clone();
            move |bot: Bot, query: CallbackQuery| {
                let tx = tx.clone();
                async move {
                    handle_callback(bot, query, tx.clone(), owner_id).await;
                }
            }
        }));

    let mut dispatcher = Dispatcher::builder(bot, handler)
        .default_handler(|_| async {})
        .storage(InMemStorage::new())
        .build();

    dispatcher.dispatch().await;
    Ok(())
}

/// Convert a Telegram text message into an InboundMessage and forward it.
async fn handle_message(
    _bot: Bot,
    msg: Message,
    tx: mpsc::Sender<InboundMessage>,
    owner_id: Option<i64>,
) {
    let from = match msg.from() {
        Some(user) => user,
        None => {
            warn!("Telegram message without sender, ignoring");
            return;
        }
    };

    let telegram_user_id = from.id.0 as i64;

    // Check owner binding if configured.
    if let Some(owner) = owner_id {
        if telegram_user_id != owner {
            debug!("ignoring message from non-owner user {}", telegram_user_id);
            return;
        }
    }

    let text = msg.text().unwrap_or("").to_string();
    let content = if text.starts_with('/') {
        let parts: Vec<&str> = text[1..].split_whitespace().collect();
        let command = parts.first().map(|s| s.to_string()).unwrap_or_default();
        let args: Vec<String> = parts.iter().skip(1).map(|s| s.to_string()).collect();
        MessageContent::Command { command, args }
    } else {
        MessageContent::Text { text }
    };

    let inbound = InboundMessage {
        channel_id: ChannelId::new("telegram"),
        message_id: MessageId::new(format!("{}", msg.id.0)),
        conversation_id: Some(ConversationId::new(format!("{}", msg.chat.id.0))),
        sender_id: format!("telegram:{}", telegram_user_id),
        content,
        timestamp: chrono::Utc::now().to_rfc3339(),
        reply_to_action: None,
    };

    if let Err(e) = tx.send(inbound).await {
        warn!("failed to forward Telegram message to daemon: {}", e);
    }
}

/// Convert a Telegram callback query (inline button press) into an InboundMessage.
async fn handle_callback(
    _bot: Bot,
    query: CallbackQuery,
    tx: mpsc::Sender<InboundMessage>,
    owner_id: Option<i64>,
) {
    let from = match &query.from {
        user => user,
    };

    let telegram_user_id = from.id.0 as i64;

    if let Some(owner) = owner_id {
        if telegram_user_id != owner {
            debug!("ignoring callback from non-owner user {}", telegram_user_id);
            return;
        }
    }

    let data = query.data.unwrap_or_default();
    // Callback data format: "action_id[:payload]",
    let (action_id, payload) = match data.split_once(':') {
        Some((aid, rest)) => (aid.to_string(), Some(rest.to_string())),
        None => (data, None),
    };

    let inbound = InboundMessage {
        channel_id: ChannelId::new("telegram"),
        message_id: MessageId::new(format!("cb-{}", query.id)),
        conversation_id: query.message.as_ref().map(|m| {
            ConversationId::new(format!("{}", m.chat.id.0))
        }),
        sender_id: format!("telegram:{}", telegram_user_id),
        content: MessageContent::Text {
            text: format!("action:{} payload:{}", action_id, payload.unwrap_or_default()),
        },
        timestamp: chrono::Utc::now().to_rfc3339(),
        reply_to_action: Some(action_id),
    };

    if let Err(e) = tx.send(inbound).await {
        warn!("failed to forward Telegram callback to daemon: {}", e);
    }
}
```

### A.3.4 Create `crates/executive/src/impl/channel/telegram/binding.rs`

- [ ] Create the file with the following complete content:

```rust
//! Telegram user-to-principal binding for owner-restricted bots.

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};

/// Links a Telegram user ID to an Aletheon principal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TelegramBinding {
    /// The Telegram user ID (numeric).
    pub telegram_user_id: i64,
    /// The Aletheon principal (agent) ID this user represents.
    pub principal_id: String,
    /// The current status of this binding.
    pub status: BindingStatus,
    /// When the binding was created (or last status change).
    pub bound_at: DateTime<Utc>,
}

/// Status of a Telegram-to-principal binding.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum BindingStatus {
    /// Awaiting admin approval.
    Pending,
    /// Binding is active — messages are routed.
    Active,
    /// Binding was rejected by admin.
    Rejected,
}

impl TelegramBinding {
    pub fn new(telegram_user_id: i64, principal_id: String) -> Self {
        Self {
            telegram_user_id,
            principal_id,
            status: BindingStatus::Pending,
            bound_at: Utc::now(),
        }
    }
}
```

### A.3.5 Create `crates/executive/src/impl/channel/telegram/formatting.rs`

- [ ] Create the file with the following complete content including tests:

```rust
//! Convert OutboundMessage to Telegram format (text + optional inline keyboard).

use fabric::types::channel::{ActionType, MessageContent, OutboundMessage};
use teloxide::types::{InlineKeyboardButton, InlineKeyboardMarkup};

/// Convert an OutboundMessage to Telegram-compatible text and optional
/// inline keyboard markup.
pub fn to_telegram(msg: &OutboundMessage) -> (String, Option<InlineKeyboardMarkup>) {
    let text = match &msg.content {
        MessageContent::Text { text } => text.clone(),
        MessageContent::Command { command, args } => {
            format!("/{} {}", command, args.join(" "))
        }
        MessageContent::File { name, .. } => {
            format!("[File] {}", name)
        }
        MessageContent::Voice { transcription, .. } => {
            match transcription {
                Some(t) => format!("[Voice] {}", t),
                None => "[Voice message]".to_string(),
            }
        }
    };

    let markup = if msg.actions.is_empty() {
        None
    } else {
        Some(build_keyboard(&msg.actions))
    };

    (text, markup)
}

/// Build an inline keyboard from action buttons.
fn build_keyboard(actions: &[fabric::types::channel::UserAction]) -> InlineKeyboardMarkup {
    let buttons: Vec<Vec<InlineKeyboardButton>> = actions
        .iter()
        .map(|action| {
            let callback_data = match &action.payload {
                Some(p) => format!("{}:{}", action.action_id, p),
                None => action.action_id.clone(),
            };
            let button = InlineKeyboardButton::callback(
                action.label.clone(),
                callback_data,
            );
            vec![button]
        })
        .collect();
    InlineKeyboardMarkup::new(buttons)
}

/// Escape Telegram MarkdownV2 special characters: _ * [ ] ( ) ~ ` > # + - = | { } . !
pub fn escape_md(text: &str) -> String {
    let special = ['_', '*', '[', ']', '(', ')', '~', '`', '>', '#', '+', '-', '=', '|', '{', '}', '.', '!'];
    text.chars()
        .map(|c| {
            if special.contains(&c) {
                format!("\\{}", c)
            } else {
                c.to_string()
            }
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::types::channel::{ChannelId, UserAction};

    #[test]
    fn text_message_no_actions() {
        let msg = OutboundMessage {
            channel_id: ChannelId::new("telegram"),
            conversation_id: None,
            content: MessageContent::Text { text: "hello".into() },
            actions: vec![],
        };
        let (text, markup) = to_telegram(&msg);
        assert_eq!(text, "hello");
        assert!(markup.is_none());
    }

    #[test]
    fn message_with_approval_buttons() {
        let msg = OutboundMessage {
            channel_id: ChannelId::new("telegram"),
            conversation_id: None,
            content: MessageContent::Text { text: "Deploy to production?".into() },
            actions: vec![
                UserAction {
                    action_id: "a1".into(),
                    action_type: ActionType::Approve,
                    label: "Approve".into(),
                    payload: None,
                },
                UserAction {
                    action_id: "a2".into(),
                    action_type: ActionType::Reject,
                    label: "Reject".into(),
                    payload: Some("too risky".into()),
                },
            ],
        };
        let (text, markup) = to_telegram(&msg);
        assert_eq!(text, "Deploy to production?");
        let kb = markup.expect("should have keyboard");
        assert_eq!(kb.inline_keyboard.len(), 2);
    }

    #[test]
    fn escape_markdown_special_chars() {
        assert_eq!(escape_md("hello_world"), "hello\\_world");
        assert_eq!(escape_md("a*b"), "a\\*b");
        assert_eq!(escape_md("[link](url)"), "\\[link\\]\\(url\\)");
        assert_eq!(escape_md("normal text"), "normal text");
        assert_eq!(escape_md("`code`"), "\\`code\\`");
    }
}
```

### A.3.6 Modify `crates/executive/Cargo.toml`

- [ ] Add the teloxide dependency. Insert after the existing dependencies (e.g., after line 36 `nix = { workspace = true }`):

```toml
teloxide = { version = "0.13", default-features = false, features = ["ctrlc_handler", "macros", "rustls"] }
```

### A.3.7 Commit

- [ ] Run:
```bash
git add crates/executive/src/impl/channel/telegram/ crates/executive/Cargo.toml
git commit -m "feat(executive): implement TelegramChannel with long-polling, owner binding, and formatting"
```


## Task A.4: Integrate ChannelRouter into daemon initialization

### A.4.1 Modify `crates/executive/src/impl/daemon/handler/init.rs`

- [ ] Add the `spawn_channel_processor` method to `RequestHandler`. Add it after the existing `new()` method body. Insert the following code before the closing `}` of the `impl RequestHandler` block (around line 900+):

```rust
    /// Spawn the channel processor loop.
    ///
    /// This method starts a Tokio task that reads inbound messages from the
    /// ChannelRegistry and routes them to the appropriate command handler.
    pub fn spawn_channel_processor(
        &self,
        mut inbound_rx: mpsc::Receiver<fabric::types::channel::InboundMessage>,
    ) {
        let systems = self.core_systems.clone();
        let cancel_token = self.cancel_token.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = cancel_token.cancelled() => {
                        info!("channel processor shutting down");
                        break;
                    }
                    msg = inbound_rx.recv() => {
                        match msg {
                            Some(inbound) => {
                                Self::process_channel_message(&systems, inbound).await;
                            }
                            None => {
                                info!("channel inbound channel closed");
                                break;
                            }
                        }
                    }
                }
            }
        });
    }

    /// Route an inbound channel message to the correct handler.
    async fn process_channel_message(
        _systems: &std::sync::Arc<CoreSystems>,
        msg: fabric::types::channel::InboundMessage,
    ) {
        use fabric::types::channel::MessageContent;
        use tracing::info;

        info!(
            channel = %msg.channel_id,
            sender = %msg.sender_id,
            "processing channel message"
        );

        match &msg.content {
            MessageContent::Command { command, args } => {
                match command.as_str() {
                    "chat" => {
                        info!("chat command: {:?}", args);
                        // Stub: will be implemented in Phase A.4 (real routing)
                        Self::handle_chat_command(&msg, args).await;
                    }
                    "goal" => {
                        info!("goal command: {:?}", args);
                        // Phase B: route to GoalSupervisor
                        Self::handle_goal_command(&msg, args).await;
                    }
                    "goals" | "list_goals" => {
                        info!("list goals command");
                        // Phase B: route to GoalSupervisor
                        Self::handle_list_goals(&msg).await;
                    }
                    "status" => {
                        info!("status command");
                        // Phase B: route to GoalSupervisor
                        Self::handle_goal_status(&msg, args).await;
                    }
                    "pause" | "resume" | "cancel" => {
                        info!("{} command: {:?}", command, args);
                        // Phase B: route to GoalSupervisor lifecycle
                        Self::handle_goal_lifecycle(&msg, command, args).await;
                    }
                    "approve" | "reject" => {
                        info!("{} command", command);
                        // Phase B: route to GoalSupervisor approval
                        Self::handle_approval_command(&msg, command, args).await;
                    }
                    other => {
                        tracing::warn!("unknown command: /{}", other);
                    }
                }
            }
            MessageContent::Text { text } if text.starts_with("action:") => {
                // Callback from inline keyboard — parse action_id from text
                // and route to approval handler.
                Self::handle_action_callback(&msg, text).await;
            }
            _ => {
                // Generic text message — treat as chat.
                Self::handle_chat_command(&msg, &[]).await;
            }
        }
    }

    /// Stub: process /chat commands.
    async fn handle_chat_command(
        msg: &fabric::types::channel::InboundMessage,
        _args: &[String],
    ) {
        info!(
            sender = %msg.sender_id,
            "chat command received (stub)"
        );
    }

    /// Stub: process /goal commands (Phase B).
    async fn handle_goal_command(
        _msg: &fabric::types::channel::InboundMessage,
        _args: &[String],
    ) {
        info!("goal command stub — Phase B will wire GoalSupervisor");
    }

    /// Stub: list goals (Phase B).
    async fn handle_list_goals(
        _msg: &fabric::types::channel::InboundMessage,
    ) {
        info!("list goals stub — Phase B will wire GoalSupervisor");
    }

    /// Stub: goal status (Phase B).
    async fn handle_goal_status(
        _msg: &fabric::types::channel::InboundMessage,
        _args: &[String],
    ) {
        info!("goal status stub — Phase B will wire GoalSupervisor");
    }

    /// Stub: pause/resume/cancel lifecycle (Phase B).
    async fn handle_goal_lifecycle(
        _msg: &fabric::types::channel::InboundMessage,
        command: &str,
        _args: &[String],
    ) {
        info!("goal lifecycle stub ({}) — Phase B will wire GoalSupervisor", command);
    }

    /// Stub: approve/reject command handling (Phase B).
    async fn handle_approval_command(
        _msg: &fabric::types::channel::InboundMessage,
        command: &str,
        _args: &[String],
    ) {
        info!("approval stub ({}) — Phase B will wire GoalSupervisor", command);
    }

    /// Stub: inline keyboard callback handling (Phase B).
    async fn handle_action_callback(
        _msg: &fabric::types::channel::InboundMessage,
        _text: &str,
    ) {
        info!("action callback stub — Phase B will wire GoalSupervisor");
    }
```

- [ ] Add necessary imports at the top of `init.rs`. Add alongside existing imports:

```rust
use crate::r#impl::channel::ChannelRegistry;
use crate::core::core_systems::CoreSystems;
```

- [ ] Inside `RequestHandler::new()`, after the existing ChannelRegistry initialization logic, add:

```rust
        // --- Channel subsystem ---
        let (inbound_tx, inbound_rx) = mpsc::channel::<fabric::types::channel::InboundMessage>(256);
        let channel_registry = std::sync::Arc::new(ChannelRegistry::new());
        channel_registry.set_inbound_sender(inbound_tx).await;
        handler.spawn_channel_processor(inbound_rx);
```

### A.4.2 Modify `crates/executive/src/impl/daemon/server.rs`

- [ ] In the `UnixServer::new()` function, after the handler is built, add optional channel startup logic:

```rust
        // If a Telegram bot token is configured, start the Telegram channel.
        if let Ok(token) = std::env::var("ALETHEON_TELEGRAM_TOKEN") {
            use crate::r#impl::channel::telegram::TelegramChannel;
            let owner_id = std::env::var("ALETHEON_TELEGRAM_OWNER_ID")
                .ok()
                .and_then(|s| s.parse().ok());
            let telegram = std::sync::Arc::new(TelegramChannel::new(&token, owner_id));
            if let Err(e) = handler.register_channel(telegram).await {
                error!(error = %e, "failed to register Telegram channel");
            }
        }
```

- [ ] Add a `register_channel` method to `RequestHandler` (in `init.rs`):

```rust
    pub async fn register_channel(
        &self,
        channel: std::sync::Arc<dyn crate::r#impl::channel::Channel>,
    ) -> anyhow::Result<()> {
        let inbound_tx = self.channel_registry.inbound_tx.lock().await.clone()
            .ok_or_else(|| anyhow::anyhow!("inbound sender not initialized"))?;
        channel.start(inbound_tx).await?;
        self.channel_registry.register(channel).await?;
        Ok(())
    }
```

### A.4.3 Add `channel_registry` field to `RequestHandler` struct

- [ ] In `crates/executive/src/impl/daemon/handler/mod.rs`, add the field to the `RequestHandler` struct (after existing fields):

```rust
    pub channel_registry: std::sync::Arc<ChannelRegistry>,
```

### A.4.4 Commit

- [ ] Run:
```bash
git add crates/executive/src/impl/daemon/handler/init.rs crates/executive/src/impl/daemon/handler/mod.rs crates/executive/src/impl/daemon/server.rs
git commit -m "feat(executive): integrate ChannelRouter into daemon with stub command handlers"
```


## Task A.5: End-to-end compilation and test

### A.5.1 Compile the workspace

- [ ] Run and verify it succeeds with no errors:
```bash
cargo check --workspace 2>&1
```

**Expected output:** `Checking fabric ...`, `Checking executive ...`, then `Finished dev [unoptimized + debuginfo] target(s)`. No error lines related to channel types or Telegram imports.

### A.5.2 Run fabric unit tests

- [ ] Run and verify all tests pass:
```bash
cargo test -p fabric --lib -- types::channel 2>&1
```

**Expected output:**
```
running 2 tests
test types::channel::tests::message_content_serde_roundtrip ... ok
test types::channel::tests::outbound_actions_serialize ... ok
test result: ok. 2 passed; 0 failed; 0 ignored
```

### A.5.3 Run executive channel unit tests

- [ ] Run and verify all tests pass:
```bash
cargo test -p executive --lib -- impl::channel 2>&1
```

**Expected output:**
```
running 5 tests
test impl::channel::tests::registry_send_routes_to_correct_channel ... ok
test impl::channel::tests::send_to_unknown_channel_errors ... ok
test impl::channel::telegram::formatting::tests::text_message_no_actions ... ok
test impl::channel::telegram::formatting::tests::message_with_approval_buttons ... ok
test impl::channel::telegram::formatting::tests::escape_markdown_special_chars ... ok
test result: ok. 5 passed; 0 failed; 0 ignored
```

### A.5.4 Commit

- [ ] Run (only if there are formatting/lint changes):
```bash
cargo fmt --all
git add -u
git commit -m "chore: cargo fmt after Phase A channel integration"
```


---

# Phase B: Goal Runtime v1 (estimated 2-3 weeks, depends on Phase A)

## Task B.1: Define Goal types in fabric

### B.1.1 Create `crates/fabric/src/types/goal.rs`

- [ ] Create the file with the following complete content including all 5 tests:

```rust
//! Goal-oriented task types — budget, workers, failure classification, and
//! state-machine primitives shared across the goal runtime layer.

use serde::{Deserialize, Serialize};
use std::fmt;

/// Unique identifier for a goal (maps to objective_id in the DB).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct GoalId(pub i64);

impl GoalId {
    pub fn new(id: i64) -> Self {
        Self(id)
    }

    pub fn as_i64(&self) -> i64 {
        self.0
    }
}

impl fmt::Display for GoalId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "goal:{}", self.0)
    }
}

/// Resource budget for a single goal attempt.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalBudget {
    /// Maximum tokens the worker may consume in one attempt.
    pub max_tokens: u64,
    /// Tokens already consumed in this goal.
    pub tokens_used: u64,
    /// Maximum wall-clock duration for one attempt (seconds).
    pub max_duration_secs: u64,
    /// Maximum number of retry attempts allowed.
    pub max_attempts: u32,
    /// Number of attempts already made.
    pub attempt_count: u32,
}

impl Default for GoalBudget {
    fn default() -> Self {
        Self {
            max_tokens: 8192,
            tokens_used: 0,
            max_duration_secs: 300,
            max_attempts: 3,
            attempt_count: 0,
        }
    }
}

impl GoalBudget {
    /// Returns true if the budget is exhausted (tokens OR attempts).
    pub fn is_exhausted(&self) -> bool {
        self.tokens_used >= self.max_tokens || self.attempt_count >= self.max_attempts
    }

    /// Remaining tokens in this budget.
    pub fn remaining_tokens(&self) -> u64 {
        self.max_tokens.saturating_sub(self.tokens_used)
    }
}

/// Kinds of workers that can execute goals.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum WorkerKind {
    /// DeepSeek model via LLM provider.
    DeepSeek,
    /// Pi coding subagent.
    Pi,
    /// Anthropic Opus model (escalation target).
    Opus,
    /// OpenAI GPT model (escalation target).
    Gpt,
    /// Native Cognit reasoning (no external API).
    NativeCognit,
}

impl fmt::Display for WorkerKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            WorkerKind::DeepSeek => write!(f, "deepseek"),
            WorkerKind::Pi => write!(f, "pi"),
            WorkerKind::Opus => write!(f, "opus"),
            WorkerKind::Gpt => write!(f, "gpt"),
            WorkerKind::NativeCognit => write!(f, "native_cognit"),
        }
    }
}

/// Classification of why an attempt failed.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum FailureClass {
    /// Code failed to compile.
    Compilation,
    /// Tests failed.
    TestFailure,
    /// A required permission was denied.
    PermissionDenied,
    /// The attempt timed out.
    Timeout,
    /// A dependency (crate, tool, file) was missing.
    MissingDependency,
    /// The worker made an invalid assumption about the environment.
    InvalidAssumption,
    /// The change violated an architecture constraint.
    ArchitectureViolation,
    /// A tool invocation failed.
    ToolFailure,
    /// The LLM context window was insufficient.
    ContextInsufficient,
    /// The same failure occurred repeatedly across attempts.
    RepeatedFailure,
}

impl fmt::Display for FailureClass {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            FailureClass::Compilation => write!(f, "compilation_error"),
            FailureClass::TestFailure => write!(f, "test_failure"),
            FailureClass::PermissionDenied => write!(f, "permission_denied"),
            FailureClass::Timeout => write!(f, "timeout"),
            FailureClass::MissingDependency => write!(f, "missing_dependency"),
            FailureClass::InvalidAssumption => write!(f, "invalid_assumption"),
            FailureClass::ArchitectureViolation => write!(f, "architecture_violation"),
            FailureClass::ToolFailure => write!(f, "tool_failure"),
            FailureClass::ContextInsufficient => write!(f, "context_insufficient"),
            FailureClass::RepeatedFailure => write!(f, "repeated_failure"),
        }
    }
}

/// Unique identifier for a goal execution attempt.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct AttemptId(pub String);

impl AttemptId {
    pub fn new(goal_id: GoalId, attempt_number: u32) -> Self {
        Self(format!("g{}-a{}", goal_id.0, attempt_number))
    }
}

/// Summary of a single goal attempt for the goal frame.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AttemptSummary {
    pub attempt_id: AttemptId,
    pub worker: WorkerKind,
    pub outcome: String,
    pub failure_class: Option<FailureClass>,
}

/// Projection of relevant memories for the goal context.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MemoryProjection {
    /// Key facts relevant to this goal.
    pub relevant_facts: Vec<String>,
    /// Past experiences that may be useful.
    pub past_experiences: Vec<String>,
}

/// A snapshot of the current goal state passed to workers.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoalFrame {
    /// The goal this frame represents.
    pub goal_id: GoalId,
    /// The original user intent (immutable).
    pub original_intent: String,
    /// The current task description (may evolve across attempts).
    pub current_task: String,
    /// Criteria that must be met for success.
    pub acceptance_criteria: Vec<String>,
    /// Summaries of previous attempts on this goal.
    pub recent_attempts: Vec<AttemptSummary>,
    /// Remaining resource budget.
    pub remaining_budget: GoalBudget,
    /// Relevant memories projected by Mnemosyne.
    pub relevant_memories: Vec<MemoryProjection>,
}

/// Possible transitions after a goal tick.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoalTransition {
    /// Nothing changed (no work done, or work yielded same state).
    NoOp,
    /// The goal state changed (attempts incremented, task refined, etc.).
    StateChanged,
    /// The goal needs human approval before proceeding.
    AwaitingApproval,
    /// The goal has been completed successfully.
    Completed,
    /// The goal has failed (budget exhausted or unrecoverable error).
    Failed,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn budget_exhausted_by_tokens() {
        let budget = GoalBudget {
            max_tokens: 100,
            tokens_used: 100,
            max_duration_secs: 60,
            max_attempts: 5,
            attempt_count: 0,
        };
        assert!(budget.is_exhausted());
        assert_eq!(budget.remaining_tokens(), 0);
    }

    #[test]
    fn budget_exhausted_by_attempts() {
        let budget = GoalBudget {
            max_tokens: 1000,
            tokens_used: 10,
            max_duration_secs: 60,
            max_attempts: 2,
            attempt_count: 2,
        };
        assert!(budget.is_exhausted());
    }

    #[test]
    fn budget_not_exhausted_initially() {
        let budget = GoalBudget::default();
        assert!(!budget.is_exhausted());
        assert_eq!(budget.remaining_tokens(), 8192);
    }

    #[test]
    fn goal_frame_serde_roundtrip() {
        let frame = GoalFrame {
            goal_id: GoalId::new(1),
            original_intent: "add OAuth2 login".into(),
            current_task: "implement token refresh flow".into(),
            acceptance_criteria: vec!["tests pass".into(), "no clippy warnings".into()],
            recent_attempts: vec![],
            remaining_budget: GoalBudget::default(),
            relevant_memories: vec![MemoryProjection {
                relevant_facts: vec!["oauth crate v0.5 used".into()],
                past_experiences: vec!["similar work in PR #23".into()],
            }],
        };
        let json = serde_json::to_string(&frame).unwrap();
        let back: GoalFrame = serde_json::from_str(&json).unwrap();
        assert_eq!(back.goal_id.0, 1);
        assert_eq!(back.original_intent, "add OAuth2 login");
        assert_eq!(back.acceptance_criteria.len(), 2);
    }

    #[test]
    fn failure_class_display() {
        assert_eq!(FailureClass::Compilation.to_string(), "compilation_error");
        assert_eq!(FailureClass::TestFailure.to_string(), "test_failure");
        assert_eq!(FailureClass::Timeout.to_string(), "timeout");
        assert_eq!(FailureClass::MissingDependency.to_string(), "missing_dependency");
        assert_eq!(FailureClass::RepeatedFailure.to_string(), "repeated_failure");
    }
}
```

### B.1.2 Modify `crates/fabric/src/types/mod.rs`

- [ ] Add `pub mod goal;` after the existing module declarations:

```rust
pub mod goal;
```

### B.1.3 Modify `crates/fabric/src/lib.rs`

- [ ] Add re-export after existing `pub use types::...` block:

```rust
pub use types::goal;
```

### B.1.4 Commit

- [ ] Run:
```bash
git add crates/fabric/src/types/goal.rs crates/fabric/src/types/mod.rs crates/fabric/src/lib.rs
git commit -m "feat(fabric): add goal types — GoalBudget, WorkerKind, FailureClass, GoalFrame, GoalTransition"
```

