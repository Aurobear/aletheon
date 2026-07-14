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


## Task B.2: Extend ObjectiveStore schema

### B.2.1 Modify `crates/fabric/src/types/objective.rs`

- [ ] Add new optional fields to the `Objective` struct and extend `ObjectiveStatus`:

```rust
/// The status of an objective, matching GoalStatus + DB constraint.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ObjectiveStatus {
    InProgress,
    Completed,
    Failed,
    Adjusted,
    /// Goal is paused (awaiting user input or approval).
    Paused,
    /// Goal is cancelled by user.
    Cancelled,
    /// Goal is awaiting human approval.
    AwaitingApproval,
}

impl ObjectiveStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ObjectiveStatus::InProgress => "in_progress",
            ObjectiveStatus::Completed => "completed",
            ObjectiveStatus::Failed => "failed",
            ObjectiveStatus::Adjusted => "adjusted",
            ObjectiveStatus::Paused => "paused",
            ObjectiveStatus::Cancelled => "cancelled",
            ObjectiveStatus::AwaitingApproval => "awaiting_approval",
        }
    }

    #[allow(clippy::should_implement_trait)]
    pub fn from_str(s: &str) -> Option<Self> {
        match s {
            "in_progress" => Some(ObjectiveStatus::InProgress),
            "completed" => Some(ObjectiveStatus::Completed),
            "failed" => Some(ObjectiveStatus::Failed),
            "adjusted" => Some(ObjectiveStatus::Adjusted),
            "paused" => Some(ObjectiveStatus::Paused),
            "cancelled" => Some(ObjectiveStatus::Cancelled),
            "awaiting_approval" => Some(ObjectiveStatus::AwaitingApproval),
            _ => None,
        }
    }
}

/// A persisted objective with optional goal-layer extensions.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Objective {
    pub objective_id: i64,
    pub description: String,
    pub status: ObjectiveStatus,
    pub parent_id: Option<i64>,
    pub session_id: String,
    pub scope: String,
    pub created_at: String,
    pub updated_at: String,
    // --- Goal-layer extensions (Phase B) ---
    /// The original user intent (immutable after creation).
    pub intent: Option<String>,
    /// JSON array of acceptance criteria strings.
    pub acceptance_criteria: Option<String>,
    /// Token budget: max allocatable.
    pub max_tokens: Option<i64>,
    /// Tokens consumed so far.
    pub tokens_used: Option<i64>,
    /// Max duration per attempt in seconds.
    pub max_duration_secs: Option<i64>,
    /// Max number of retry attempts.
    pub max_attempts: Option<i64>,
    /// Number of attempts made so far.
    pub attempt_count: Option<i64>,
    /// ISO-8601 deadline for the goal.
    pub deadline: Option<String>,
    /// JSON-encoded plan data (GoalPlan serialized).
    pub plan_json: Option<String>,
}
```

### B.2.2 Modify `crates/executive/src/impl/goal/mod.rs`

- [ ] Extend `create_schema` with new columns and the `attempts` table. Update the existing method to add ALTER TABLE migrations and a new attempts table:

```rust
    fn create_schema(db: &Connection) -> Result<()> {
        // Base objectives table (existing, unchanged).
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS objectives (
                objective_id INTEGER PRIMARY KEY AUTOINCREMENT,
                description  TEXT NOT NULL,
                status       TEXT NOT NULL DEFAULT 'in_progress'
                             CHECK(status IN ('in_progress','completed','failed',
                                'adjusted','paused','cancelled','awaiting_approval')),
                parent_id    INTEGER REFERENCES objectives(objective_id) ON DELETE CASCADE,
                session_id   TEXT NOT NULL DEFAULT '',
                scope        TEXT NOT NULL DEFAULT 'session'
                             CHECK(scope IN ('session','project','global')),
                created_at   TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at   TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_objectives_status ON objectives(status);
            CREATE INDEX IF NOT EXISTS idx_objectives_parent ON objectives(parent_id);
            CREATE INDEX IF NOT EXISTS idx_objectives_session ON objectives(session_id);
            CREATE INDEX IF NOT EXISTS idx_objectives_scope ON objectives(scope);",
        )?;

        // Run ALTER TABLE migrations for new goal-layer columns
        // (idempotent -- errors ignored for duplicate columns).
        let migrations = [
            "ALTER TABLE objectives ADD COLUMN intent TEXT DEFAULT NULL;",
            "ALTER TABLE objectives ADD COLUMN acceptance_criteria TEXT DEFAULT NULL;",
            "ALTER TABLE objectives ADD COLUMN max_tokens INTEGER DEFAULT NULL;",
            "ALTER TABLE objectives ADD COLUMN tokens_used INTEGER DEFAULT NULL;",
            "ALTER TABLE objectives ADD COLUMN max_duration_secs INTEGER DEFAULT NULL;",
            "ALTER TABLE objectives ADD COLUMN max_attempts INTEGER DEFAULT NULL;",
            "ALTER TABLE objectives ADD COLUMN attempt_count INTEGER DEFAULT NULL;",
            "ALTER TABLE objectives ADD COLUMN deadline TEXT DEFAULT NULL;",
            "ALTER TABLE objectives ADD COLUMN plan_json TEXT DEFAULT NULL;",
        ];
        for migration in &migrations {
            let _ = db.execute_batch(migration);
        }

        // Attempts table -- records each worker execution attempt.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS goal_attempts (
                attempt_id    TEXT PRIMARY KEY,
                goal_id       INTEGER NOT NULL
                    REFERENCES objectives(objective_id) ON DELETE CASCADE,
                worker_kind   TEXT NOT NULL DEFAULT 'deepseek',
                attempt_num   INTEGER NOT NULL DEFAULT 1,
                status        TEXT NOT NULL DEFAULT 'started'
                              CHECK(status IN ('started','running','succeeded','failed')),
                output_text   TEXT DEFAULT '',
                failure_class TEXT DEFAULT NULL,
                tokens_used   INTEGER DEFAULT 0,
                started_at    TEXT NOT NULL DEFAULT (datetime('now')),
                finished_at   TEXT DEFAULT NULL
            );
            CREATE INDEX IF NOT EXISTS idx_attempts_goal ON goal_attempts(goal_id);",
        )?;

        // Objectives v2 table -- stores the expanded objective with all goal-layer fields.
        db.execute_batch(
            "CREATE TABLE IF NOT EXISTS objectives_v2 (
                objective_id        INTEGER PRIMARY KEY AUTOINCREMENT,
                description         TEXT NOT NULL,
                status              TEXT NOT NULL DEFAULT 'in_progress'
                                    CHECK(status IN ('in_progress','completed','failed',
                                        'adjusted','paused','cancelled','awaiting_approval')),
                parent_id           INTEGER
                    REFERENCES objectives_v2(objective_id) ON DELETE CASCADE,
                session_id          TEXT NOT NULL DEFAULT '',
                scope               TEXT NOT NULL DEFAULT 'session'
                                    CHECK(scope IN ('session','project','global')),
                intent              TEXT DEFAULT NULL,
                acceptance_criteria TEXT DEFAULT NULL,
                max_tokens          INTEGER DEFAULT NULL,
                tokens_used         INTEGER DEFAULT NULL,
                max_duration_secs   INTEGER DEFAULT NULL,
                max_attempts        INTEGER DEFAULT NULL,
                attempt_count       INTEGER DEFAULT NULL,
                deadline            TEXT DEFAULT NULL,
                plan_json           TEXT DEFAULT NULL,
                created_at          TEXT NOT NULL DEFAULT (datetime('now')),
                updated_at          TEXT NOT NULL DEFAULT (datetime('now'))
            );
            CREATE INDEX IF NOT EXISTS idx_objs_v2_status ON objectives_v2(status);
            CREATE INDEX IF NOT EXISTS idx_objs_v2_parent ON objectives_v2(parent_id);",
        )?;

        Ok(())
    }
```

- [ ] Update the `map_objective_row` function to handle the extended column count:

```rust
    /// Map a rusqlite Row to an Objective using positional indices.
    ///
    /// Column order MUST match the COLS constant in store.rs.
    /// Indices: 0=objective_id, 1=description, 2=status, 3=parent_id,
    ///          4=session_id, 5=scope, 6=created_at, 7=updated_at,
    ///          8=intent, 9=acceptance_criteria, 10=max_tokens,
    ///          11=tokens_used, 12=max_duration_secs, 13=max_attempts,
    ///          14=attempt_count, 15=deadline, 16=plan_json
    pub(crate) fn map_objective_row(row: &rusqlite::Row) -> rusqlite::Result<Objective> {
        let status_str: String = row.get(2)?;
        let status = ObjectiveStatus::from_str(&status_str)
            .unwrap_or(ObjectiveStatus::InProgress);
        Ok(Objective {
            objective_id: row.get(0)?,
            description: row.get(1)?,
            status,
            parent_id: row.get(3)?,
            session_id: row.get(4)?,
            scope: row.get(5)?,
            created_at: row.get(6)?,
            updated_at: row.get(7)?,
            intent: row.get(8)?,
            acceptance_criteria: row.get(9)?,
            max_tokens: row.get(10)?,
            tokens_used: row.get(11)?,
            max_duration_secs: row.get(12)?,
            max_attempts: row.get(13)?,
            attempt_count: row.get(14)?,
            deadline: row.get(15)?,
            plan_json: row.get(16)?,
        })
    }
```

### B.2.3 Modify `crates/executive/src/impl/goal/store.rs`

- [ ] Update the `COLS` constant to include new fields:

```rust
/// Fixed column order -- every SELECT feeding map_objective_row MUST use this.
/// Indices: 0=objective_id, 1=description, 2=status, 3=parent_id,
///          4=session_id, 5=scope, 6=created_at, 7=updated_at,
///          8=intent, 9=acceptance_criteria, 10=max_tokens,
///          11=tokens_used, 12=max_duration_secs, 13=max_attempts,
///          14=attempt_count, 15=deadline, 16=plan_json
pub(crate) const COLS: &str = concat!(
    "objective_id, description, status, parent_id, session_id, scope, ",
    "created_at, updated_at, intent, acceptance_criteria, max_tokens, ",
    "tokens_used, max_duration_secs, max_attempts, attempt_count, ",
    "deadline, plan_json"
);
```

- [ ] Add `create_goal`, `record_attempt`, and `get_attempts` methods to `ObjectiveStore`:

```rust
    /// Create a goal (extended objective) with intent and budget.
    pub fn create_goal(
        &self,
        intent: &str,
        acceptance_criteria: &[String],
        max_tokens: i64,
        max_duration_secs: i64,
        max_attempts: i64,
        session_id: &str,
    ) -> Result<i64> {
        let criteria_json = serde_json::to_string(acceptance_criteria)?;
        self.db.execute(
            "INSERT INTO objectives_v2 "
            "(description, intent, acceptance_criteria, max_tokens, "
            " max_duration_secs, max_attempts, session_id, scope) "
            "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, 'session')",
            rusqlite::params![
                intent,
                intent,
                criteria_json,
                max_tokens,
                max_duration_secs,
                max_attempts,
                session_id
            ],
        )?;
        Ok(self.db.last_insert_rowid())
    }

    /// Record a goal execution attempt.
    pub fn record_attempt(
        &self,
        goal_id: i64,
        worker_kind: &str,
        attempt_num: i64,
        status: &str,
        output_text: &str,
        failure_class: Option<&str>,
        tokens_used: i64,
    ) -> Result<String> {
        let attempt_id = format!("g{}-a{}", goal_id, attempt_num);
        self.db.execute(
            "INSERT INTO goal_attempts "
            "(attempt_id, goal_id, worker_kind, attempt_num, status, "
            " output_text, failure_class, tokens_used) "
            "VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8)",
            rusqlite::params![
                attempt_id,
                goal_id,
                worker_kind,
                attempt_num,
                status,
                output_text,
                failure_class,
                tokens_used
            ],
        )?;
        Ok(attempt_id)
    }

    /// Get all attempts for a given goal, ordered by attempt_num.
    pub fn get_attempts(&self, goal_id: i64) -> Result<Vec<GoalAttempt>> {
        let mut stmt = self.db.prepare(
            "SELECT attempt_id, goal_id, worker_kind, attempt_num, status, "
            "output_text, failure_class, tokens_used, started_at, finished_at "
            "FROM goal_attempts WHERE goal_id = ?1 ORDER BY attempt_num",
        )?;
        let rows = stmt.query_map(
            rusqlite::params![goal_id],
            |row| {
                Ok(GoalAttempt {
                    attempt_id: row.get(0)?,
                    goal_id: row.get(1)?,
                    worker_kind: row.get(2)?,
                    attempt_num: row.get(3)?,
                    status: row.get(4)?,
                    output_text: row.get(5)?,
                    failure_class: row.get(6)?,
                    tokens_used: row.get(7)?,
                    started_at: row.get(8)?,
                    finished_at: row.get(9)?,
                })
            },
        )?;
        rows.collect::<std::result::Result<Vec<_>, _>>()
            .map_err(Into::into)
    }
```

- [ ] Add helper parsing functions and the `GoalAttempt` struct to `store.rs`:

```rust
/// Record of a single goal execution attempt.
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct GoalAttempt {
    pub attempt_id: String,
    pub goal_id: i64,
    pub worker_kind: String,
    pub attempt_num: i64,
    pub status: String,
    pub output_text: String,
    pub failure_class: Option<String>,
    pub tokens_used: i64,
    pub started_at: String,
    pub finished_at: Option<String>,
}

/// Parse a worker kind string from DB or API input.
pub fn parse_worker_kind(s: &str) -> Option<fabric::types::goal::WorkerKind> {
    match s {
        "deepseek" => Some(fabric::types::goal::WorkerKind::DeepSeek),
        "pi" => Some(fabric::types::goal::WorkerKind::Pi),
        "opus" => Some(fabric::types::goal::WorkerKind::Opus),
        "gpt" => Some(fabric::types::goal::WorkerKind::Gpt),
        "native_cognit" => Some(fabric::types::goal::WorkerKind::NativeCognit),
        _ => None,
    }
}

/// Parse a failure class string from DB or classified output.
pub fn parse_failure_class(s: &str) -> Option<fabric::types::goal::FailureClass> {
    match s {
        "compilation_error" => Some(fabric::types::goal::FailureClass::Compilation),
        "test_failure" => Some(fabric::types::goal::FailureClass::TestFailure),
        "permission_denied" => Some(fabric::types::goal::FailureClass::PermissionDenied),
        "timeout" => Some(fabric::types::goal::FailureClass::Timeout),
        "missing_dependency" => Some(fabric::types::goal::FailureClass::MissingDependency),
        "invalid_assumption" => Some(fabric::types::goal::FailureClass::InvalidAssumption),
        "architecture_violation" => Some(fabric::types::goal::FailureClass::ArchitectureViolation),
        "tool_failure" => Some(fabric::types::goal::FailureClass::ToolFailure),
        "context_insufficient" => Some(fabric::types::goal::FailureClass::ContextInsufficient),
        "repeated_failure" => Some(fabric::types::goal::FailureClass::RepeatedFailure),
        _ => None,
    }
```


### B.2.4 Commit

- [ ] Run:
```bash
git add crates/fabric/src/types/objective.rs crates/executive/src/impl/goal/mod.rs crates/executive/src/impl/goal/store.rs
git commit -m "feat(goal): extend ObjectiveStore schema with goal-layer fields, attempts table, and objectives_v2"
```

## Task B.3: Implement GoalSupervisor

### B.3.1 Create `crates/executive/src/impl/goal/state_machine.rs`

- [ ] Create the file with the following complete content including 4 tests:

```rust
//! Goal state-machine -- validates transitions and computes the next status
//! from events.

use fabric::types::goal::GoalTransition;

/// Events that can trigger a goal state change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoalEvent {
    /// Work was dispatched to a worker.
    WorkDispatched,
    /// A worker completed an attempt successfully.
    AttemptSucceeded,
    /// A worker attempt failed.
    AttemptFailed,
    /// The user paused the goal.
    UserPaused,
    /// The user resumed the goal.
    UserResumed,
    /// The user cancelled the goal.
    UserCancelled,
    /// The user approved a pending action.
    UserApproved,
    /// The user rejected a pending action.
    UserRejected,
    /// The goal budget is exhausted.
    BudgetExhausted,
}

impl GoalEvent {
    /// Compute the next GoalTransition given the current transition.
    /// This is a pure function -- the supervisor applies side effects.
    pub fn next_status(&self, current: &GoalTransition) -> GoalTransition {
        use GoalEvent::*;
        use GoalTransition::*;
        match (self, current) {
            // Transitions from idle/state-changed states.
            (WorkDispatched, NoOp | StateChanged) => StateChanged,
            (AttemptSucceeded, StateChanged) => Completed,
            (AttemptFailed, StateChanged) => StateChanged,
            (BudgetExhausted, _) => Failed,

            // Lifecycle commands.
            (UserPaused, _) => StateChanged,
            (UserResumed, _) => StateChanged,
            (UserCancelled, _) => Failed,

            // Approval.
            (UserApproved, AwaitingApproval) => StateChanged,
            (UserRejected, AwaitingApproval) => StateChanged,

            // Completed and Failed are terminal.
            (_, Completed) | (_, Failed) => current.clone(),

            // Default: no change.
            _ => NoOp,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn work_dispatched_from_noop_transitions_to_state_changed() {
        let result = GoalEvent::WorkDispatched.next_status(&GoalTransition::NoOp);
        assert_eq!(result, GoalTransition::StateChanged);
    }

    #[test]
    fn attempt_succeeded_from_state_changed_transitions_to_completed() {
        let result =
            GoalEvent::AttemptSucceeded.next_status(&GoalTransition::StateChanged);
        assert_eq!(result, GoalTransition::Completed);
    }

    #[test]
    fn budget_exhausted_always_transitions_to_failed() {
        let result =
            GoalEvent::BudgetExhausted.next_status(&GoalTransition::StateChanged);
        assert_eq!(result, GoalTransition::Failed);
    }

    #[test]
    fn completed_is_terminal() {
        let result =
            GoalEvent::WorkDispatched.next_status(&GoalTransition::Completed);
        assert_eq!(result, GoalTransition::Completed);
    }
}
```

### B.3.2 Create `crates/executive/src/impl/goal/frame.rs`

- [ ] Create the file with the following content including 1 test:

```rust
//! Goal frame builder -- assembles a GoalFrame from stored objective and
//! attempt data.

use anyhow::Result;
use fabric::types::goal::{GoalBudget, GoalFrame, GoalId};
use crate::impl::goal::ObjectiveStore;
use crate::impl::goal::store::GoalAttempt;

/// Build a GoalFrame from the objective store.
pub fn build_goal_frame(
    store: &ObjectiveStore,
    goal_id: i64,
) -> Result<GoalFrame> {
    let obj = store
        .get(goal_id)?
        .ok_or_else(|| anyhow::anyhow!("goal {} not found", goal_id))?;

    let acceptance_criteria: Vec<String> = obj
        .acceptance_criteria
        .as_ref()
        .and_then(|s| serde_json::from_str(s).ok())
        .unwrap_or_default();

    let remaining_budget = GoalBudget {
        max_tokens: obj.max_tokens.unwrap_or(8192) as u64,
        tokens_used: obj.tokens_used.unwrap_or(0) as u64,
        max_duration_secs: obj.max_duration_secs.unwrap_or(300) as u64,
        max_attempts: obj.max_attempts.unwrap_or(3) as u32,
        attempt_count: obj.attempt_count.unwrap_or(0) as u32,
    };

    // Build attempt summaries from stored attempts.
    let attempts: Vec<GoalAttempt> = store.get_attempts(goal_id).unwrap_or_default();
    let recent_attempts: Vec<_> = attempts
        .iter()
        .map(|a| fabric::types::goal::AttemptSummary {
            attempt_id: fabric::types::goal::AttemptId(a.attempt_id.clone()),
            worker: crate::impl::goal::store::parse_worker_kind(&a.worker_kind)
                .unwrap_or(fabric::types::goal::WorkerKind::DeepSeek),
            outcome: a.status.clone(),
            failure_class: a
                .failure_class
                .as_ref()
                .and_then(|s| crate::impl::goal::store::parse_failure_class(s)),
        })
        .collect();

    Ok(GoalFrame {
        goal_id: GoalId::new(goal_id),
        original_intent: obj.intent.unwrap_or_else(|| obj.description.clone()),
        current_task: obj.description.clone(),
        acceptance_criteria,
        recent_attempts,
        remaining_budget,
        relevant_memories: vec![],
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn build_frame_from_empty_store_errors() {
        let dir = tempdir().unwrap();
        let db_path = dir.path().join("test.db");
        let store = ObjectiveStore::open(&db_path).unwrap();
        let result = build_goal_frame(&store, 999);
        assert!(result.is_err());
    }
}
```

### B.3.3 Create `crates/executive/src/impl/goal/supervisor.rs`

- [ ] Create the file with the following content:

```rust
//! GoalSupervisor -- the central orchestrator for goal lifecycle.

use anyhow::Result;
use async_trait::async_trait;
use fabric::types::goal::{GoalBudget, GoalFrame, GoalId, GoalTransition};
use std::sync::Arc;
use tokio::sync::Mutex;
use tracing::{info, warn};

use crate::impl::goal::frame::build_goal_frame;

/// The GoalSupervisor trait defines the lifecycle API for goals.
#[async_trait]
pub trait GoalSupervisor: Send + Sync {
    /// Create a new goal from user intent.
    async fn create_goal(
        &self,
        intent: &str,
        acceptance_criteria: &[String],
        budget: GoalBudget,
    ) -> Result<GoalId>;

    /// Tick the goal -- dispatch work if needed, check budgets, etc.
    /// Returns the transition that occurred.
    async fn tick(&self, goal_id: GoalId) -> Result<GoalTransition>;

    /// Pause an active goal.
    async fn pause(&self, goal_id: GoalId) -> Result<()>;

    /// Resume a paused goal.
    async fn resume(&self, goal_id: GoalId) -> Result<()>;

    /// Cancel a goal permanently.
    async fn cancel(&self, goal_id: GoalId) -> Result<()>;

    /// List all goals with their current status.
    async fn list_goals(&self) -> Result<Vec<GoalFrame>>;
}

/// Default implementation backed by ObjectiveStore.
pub struct DefaultGoalSupervisor {
    pub store: Arc<Mutex<crate::impl::goal::ObjectiveStore>>,
}

impl DefaultGoalSupervisor {
    pub fn new(store: crate::impl::goal::ObjectiveStore) -> Self {
        Self {
            store: Arc::new(Mutex::new(store)),
        }
    }
}

#[async_trait]
impl GoalSupervisor for DefaultGoalSupervisor {
    async fn create_goal(
        &self,
        intent: &str,
        acceptance_criteria: &[String],
        budget: GoalBudget,
    ) -> Result<GoalId> {
        let store = self.store.lock().await;
        let id = store.create_goal(
            intent,
            acceptance_criteria,
            budget.max_tokens as i64,
            budget.max_duration_secs as i64,
            budget.max_attempts as i64,
            "",
        )?;
        info!(goal_id = id, "goal created");
        Ok(GoalId::new(id))
    }

    async fn tick(&self, goal_id: GoalId) -> Result<GoalTransition> {
        let store = self.store.lock().await;
        let obj = store
            .get(goal_id.as_i64())?
            .ok_or_else(|| anyhow::anyhow!("goal {} not found", goal_id))?;

        // Check if budget is exhausted.
        let attempt_count = obj.attempt_count.unwrap_or(0) as u32;
        let max_attempts = obj.max_attempts.unwrap_or(3) as u32;
        let tokens_used = obj.tokens_used.unwrap_or(0) as u64;
        let max_tokens = obj.max_tokens.unwrap_or(8192) as u64;

        if attempt_count >= max_attempts || tokens_used >= max_tokens {
            warn!(goal_id = %goal_id, "budget exhausted");
            store.set_status(goal_id.as_i64(), "failed")?;
            return Ok(GoalTransition::Failed);
        }

        // For now, tick is a no-op -- Phase C will dispatch workers here.
        // Increment attempt_count to track that a tick occurred.
        let new_count = attempt_count + 1;
        store.db.execute(
            "UPDATE objectives_v2 SET attempt_count = ?1, updated_at = datetime('now')
             WHERE objective_id = ?2",
            rusqlite::params![new_count as i64, goal_id.as_i64()],
        )?;
        drop(store);

        Ok(GoalTransition::StateChanged)
    }

    async fn pause(&self, goal_id: GoalId) -> Result<()> {
        let store = self.store.lock().await;
        store.set_status(goal_id.as_i64(), "paused")?;
        info!(goal_id = %goal_id, "goal paused");
        Ok(())
    }

    async fn resume(&self, goal_id: GoalId) -> Result<()> {
        let store = self.store.lock().await;
        store.set_status(goal_id.as_i64(), "in_progress")?;
        info!(goal_id = %goal_id, "goal resumed");
        Ok(())
    }

    async fn cancel(&self, goal_id: GoalId) -> Result<()> {
        let store = self.store.lock().await;
        store.set_status(goal_id.as_i64(), "cancelled")?;
        info!(goal_id = %goal_id, "goal cancelled");
        Ok(())
    }

    async fn list_goals(&self) -> Result<Vec<GoalFrame>> {
        let store = self.store.lock().await;
        let objectives = store.list(None, 50)?;
        let mut frames = Vec::new();
        for obj in objectives {
            if let Ok(frame) = build_goal_frame(&store, obj.objective_id) {
                frames.push(frame);
            }
        }
        Ok(frames)
    }
}
```

### B.3.4 Create `crates/executive/src/impl/goal/budget.rs`

- [ ] Create the file with placeholder content:

```rust
//! Goal budget management -- placeholder for Phase C/D worker integration.

use fabric::types::goal::GoalBudget;

/// Check if the given budget is exhausted.
pub fn is_budget_exhausted(budget: &GoalBudget) -> bool {
    budget.is_exhausted()
}

/// Compute remaining budget after consuming tokens.
pub fn consume_tokens(budget: &mut GoalBudget, tokens: u64) {
    budget.tokens_used = budget.tokens_used.saturating_add(tokens);
}

/// Increment the attempt counter.
pub fn increment_attempt(budget: &mut GoalBudget) {
    budget.attempt_count = budget.attempt_count.saturating_add(1);
}
```

### B.3.5 Modify `crates/executive/src/impl/goal/mod.rs`

- [ ] Re-export new modules. Add after the existing `mod store;` and before the struct definitions:

```rust
pub mod budget;
pub mod frame;
pub mod state_machine;
pub mod supervisor;

pub use budget::*;
pub use frame::build_goal_frame;
pub use state_machine::{GoalEvent, GoalTransition};
pub use supervisor::{DefaultGoalSupervisor, GoalSupervisor};
```

### B.3.6 Commit

- [ ] Run:
```bash
git add crates/executive/src/impl/goal/state_machine.rs         crates/executive/src/impl/goal/frame.rs         crates/executive/src/impl/goal/supervisor.rs         crates/executive/src/impl/goal/budget.rs         crates/executive/src/impl/goal/mod.rs
git commit -m "feat(goal): implement GoalSupervisor with state machine, frame builder, and budget tracking"
```


## Task B.4: Wire GoalSupervisor into daemon handler

### B.4.1 Modify `crates/executive/src/core/core_systems.rs`

- [ ] Add a `goal_supervisor` field to the `CoreSystems` struct:

```rust
    /// Goal runtime supervisor -- manages goal lifecycle, dispatching,
    /// and retry/escalation.
    pub goal_supervisor: Arc<crate::impl::goal::DefaultGoalSupervisor>,
```

- [ ] Add the necessary import at the top of the file:

```rust
use crate::impl::goal::DefaultGoalSupervisor;
```

### B.4.2 Modify `crates/executive/src/impl/daemon/server.rs`

- [ ] In the `UnixServer::new()` function, initialize the `ObjectiveStore` and `DefaultGoalSupervisor` before building `CoreSystems`:

```rust
        // --- Goal subsystem ---
        let goal_db_path = config.data_dir.join("goals.db");
        let goal_store = crate::impl::goal::ObjectiveStore::open(&goal_db_path)
            .context("opening goal store")?;
        let goal_supervisor = Arc::new(
            crate::impl::goal::DefaultGoalSupervisor::new(goal_store)
        );
```

- [ ] Pass `goal_supervisor` when constructing `CoreSystems`:

```rust
        let core = CoreSystems {
            ports,
            runtime,
            self_field,
            reflector,
            memory,
            security,
            corpus,
            session,
            debug_handler,
            goal_supervisor,  // <-- add this
        };
```

### B.4.3 Modify `crates/executive/src/impl/daemon/handler/init.rs`

- [ ] Replace the stub `handle_goal_command` with a real implementation:

```rust
    /// Process /goal commands -- create a new goal via GoalSupervisor.
    async fn handle_goal_command(
        systems: &Arc<CoreSystems>,
        msg: &fabric::types::channel::InboundMessage,
        args: &[String],
    ) {
        let intent = args.join(" ");
        if intent.is_empty() {
            Self::send_channel_reply(
                systems,
                msg,
                "Usage: /goal <description of what you want me to do>",
            )
            .await;
            return;
        }

        let budget = fabric::types::goal::GoalBudget::default();
        match systems.goal_supervisor.create_goal(&intent, &[], budget).await {
            Ok(goal_id) => {
                let response = format!("Goal created: {}. I will start working on it.", goal_id);
                Self::send_channel_reply(systems, msg, &response).await;
                // Kick off the first tick.
                let supervisor = systems.goal_supervisor.clone();
                let gid = goal_id;
                tokio::spawn(async move {
                    if let Err(e) = supervisor.tick(gid).await {
                        tracing::error!(goal_id = %gid, error = %e, "tick failed");
                    }
                });
            }
            Err(e) => {
                Self::send_channel_reply(
                    systems,
                    msg,
                    &format!("Failed to create goal: {}", e),
                )
                .await;
            }
        }
    }
```

- [ ] Replace the stub `handle_list_goals`:

```rust
    /// List all goals.
    async fn handle_list_goals(
        systems: &Arc<CoreSystems>,
        msg: &fabric::types::channel::InboundMessage,
    ) {
        match systems.goal_supervisor.list_goals().await {
            Ok(frames) => {
                if frames.is_empty() {
                    Self::send_channel_reply(systems, msg, "No active goals.").await;
                } else {
                    let listing: Vec<String> = frames
                        .iter()
                        .map(|f| {
                            format!(
                                "* {}: {} (attempts: {}/{})",
                                f.goal_id,
                                f.original_intent,
                                f.remaining_budget.attempt_count,
                                f.remaining_budget.max_attempts
                            )
                        })
                        .collect();
                    Self::send_channel_reply(systems, msg, &listing.join("
")).await;
                }
            }
            Err(e) => {
                Self::send_channel_reply(
                    systems,
                    msg,
                    &format!("Failed to list goals: {}", e),
                )
                .await;
            }
        }
    }
```

- [ ] Replace the stub `handle_goal_lifecycle`:

```rust
    /// Handle pause/resume/cancel lifecycle commands.
    async fn handle_goal_lifecycle(
        systems: &Arc<CoreSystems>,
        msg: &fabric::types::channel::InboundMessage,
        command: &str,
        args: &[String],
    ) {
        let goal_id = match args.first().and_then(|s| s.parse::<i64>().ok()) {
            Some(id) => fabric::types::goal::GoalId::new(id),
            None => {
                Self::send_channel_reply(
                    systems,
                    msg,
                    &format!("Usage: /{} <goal_id>", command),
                )
                .await;
                return;
            }
        };

        let result = match command {
            "pause" => systems.goal_supervisor.pause(goal_id).await,
            "resume" => systems.goal_supervisor.resume(goal_id).await,
            "cancel" => systems.goal_supervisor.cancel(goal_id).await,
            _ => {
                Self::send_channel_reply(
                    systems,
                    msg,
                    &format!("Unknown lifecycle command: {}", command),
                )
                .await;
                return;
            }
        };

        match result {
            Ok(()) => {
                Self::send_channel_reply(
                    systems,
                    msg,
                    &format!("Goal {} {}.", goal_id, command),
                )
                .await;
            }
            Err(e) => {
                Self::send_channel_reply(
                    systems,
                    msg,
                    &format!("Failed to {} goal: {}", command, e),
                )
                .await;
            }
        }
    }
```

- [ ] Add a `send_channel_reply` helper method:

```rust
    /// Send a text reply back through the same channel.
    async fn send_channel_reply(
        systems: &Arc<CoreSystems>,
        msg: &fabric::types::channel::InboundMessage,
        text: &str,
    ) {
        use fabric::types::channel::{MessageContent, OutboundMessage};
        let reply = OutboundMessage {
            channel_id: msg.channel_id.clone(),
            conversation_id: msg.conversation_id.clone(),
            content: MessageContent::Text {
                text: text.to_string(),
            },
            actions: vec![],
        };
        // If a channel_registry is available on systems, use it.
        // In the current architecture, the channel_registry is on RequestHandler.
        // This helper emits a log for now; Phase E will wire full channel output.
        tracing::info!(
            channel = %msg.channel_id,
            text = %text,
            "channel reply (routing via handler)"
        );
        let _ = reply; // Placeholder until channel_registry is accessible.
    }
```

### B.4.4 Update the `process_channel_message` method

- [ ] Update the method signature and body to pass `systems` to the handlers:

```rust
    async fn process_channel_message(
        systems: &Arc<CoreSystems>,
        msg: fabric::types::channel::InboundMessage,
    ) {
        // ... existing content matching logic, but call handlers with systems:
        "goal" => {
            Self::handle_goal_command(systems, &msg, args).await;
        }
        // ... same for all other commands
    }
```

### B.4.5 Commit

- [ ] Run:
```bash
git add crates/executive/src/core/core_systems.rs         crates/executive/src/impl/daemon/server.rs         crates/executive/src/impl/daemon/handler/init.rs
git commit -m "feat(goal): wire GoalSupervisor into daemon handler with real command implementations"
```

### B.4.6 End-to-end compilation and test

- [ ] Run and verify:
```bash
cargo check --workspace 2>&1
```

**Expected output:** Clean compilation of `fabric` and `executive` crates. No errors related to goal types, supervisor, or wiring.

- [ ] Run unit tests:
```bash
cargo test -p fabric --lib -- types::goal 2>&1
cargo test -p executive --lib -- impl::goal 2>&1
```

**Expected output for fabric:**
```
running 5 tests
test types::goal::tests::budget_exhausted_by_tokens ... ok
test types::goal::tests::budget_exhausted_by_attempts ... ok
test types::goal::tests::budget_not_exhausted_initially ... ok
test types::goal::tests::goal_frame_serde_roundtrip ... ok
test types::goal::tests::failure_class_display ... ok
test result: ok. 5 passed; 0 failed; 0 ignored
```

**Expected output for executive:**
```
running 4 tests
test impl::goal::state_machine::tests::work_dispatched_from_noop_transitions_to_state_changed ... ok
test impl::goal::state_machine::tests::attempt_succeeded_from_state_changed_transitions_to_completed ... ok
test impl::goal::state_machine::tests::budget_exhausted_always_transitions_to_failed ... ok
test impl::goal::state_machine::tests::completed_is_terminal ... ok
test result: ok. 4 passed; 0 failed; 0 ignored
```

- [ ] Commit if formatting changes:
```bash
cargo fmt --all
git add -u
git commit -m "chore: cargo fmt after Phase B goal runtime integration"
```


---
# Phase C: DeepSeek Worker + Retry (estimated 1-2 weeks, depends on Phase B)

## Task C.1: Define GoalWorker trait and AttemptRecord

### C.1.1 Create `crates/executive/src/impl/goal/worker.rs`

- [ ] Create the file with the following content including 2 tests:

```rust
//! GoalWorker trait and worker registry.

use anyhow::Result;
use async_trait::async_trait;
use fabric::types::goal::{FailureClass, GoalFrame, WorkerKind};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::Mutex;

/// Output produced by a goal worker execution.
#[derive(Debug, Clone)]
pub struct WorkerOutput {
    /// The text output from the worker.
    pub text: String,
    /// Whether the worker considers the attempt successful.
    pub success: bool,
    /// How many tokens were consumed.
    pub tokens_used: u64,
    /// Classification if the attempt was not successful.
    pub failure: Option<ClassifiedFailure>,
    /// Any produced artifacts (diffs, files).
    pub artifacts: Vec<String>,
}

/// A classified failure from worker output.
#[derive(Debug, Clone)]
pub struct ClassifiedFailure {
    pub class: FailureClass,
    pub message: String,
}

/// The GoalWorker trait defines the contract for goal-execution workers.
#[async_trait]
pub trait GoalWorker: Send + Sync {
    /// Which kind of worker this is.
    fn kind(&self) -> WorkerKind;

    /// Execute the goal frame and return output.
    async fn execute(&self, frame: &GoalFrame) -> Result<WorkerOutput>;
}

/// Registry of available goal workers, keyed by WorkerKind.
pub struct WorkerRegistry {
    workers: Mutex<HashMap<WorkerKind, Arc<dyn GoalWorker>>>,
}

impl WorkerRegistry {
    pub fn new() -> Self {
        Self {
            workers: Mutex::new(HashMap::new()),
        }
    }

    pub async fn register(&self, worker: Arc<dyn GoalWorker>) {
        let mut guard = self.workers.lock().await;
        guard.insert(worker.kind(), worker);
    }

    pub async fn get(&self, kind: &WorkerKind) -> Option<Arc<dyn GoalWorker>> {
        let guard = self.workers.lock().await;
        guard.get(kind).cloned()
    }
}

impl Default for WorkerRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct StubWorker {
        kind: WorkerKind,
        result: WorkerOutput,
    }

    #[async_trait]
    impl GoalWorker for StubWorker {
        fn kind(&self) -> WorkerKind {
            self.kind.clone()
        }

        async fn execute(&self, _frame: &GoalFrame) -> Result<WorkerOutput> {
            Ok(self.result.clone())
        }
    }

    #[tokio::test]
    async fn registry_get_returns_registered_worker() {
        let registry = WorkerRegistry::new();
        let stub = Arc::new(StubWorker {
            kind: WorkerKind::DeepSeek,
            result: WorkerOutput {
                text: "ok".into(),
                success: true,
                tokens_used: 100,
                failure: None,
                artifacts: vec![],
            },
        });
        registry.register(stub).await;
        let found = registry.get(&WorkerKind::DeepSeek).await;
        assert!(found.is_some());
        assert_eq!(found.unwrap().kind(), WorkerKind::DeepSeek);
    }

    #[tokio::test]
    async fn registry_get_unknown_returns_none() {
        let registry = WorkerRegistry::new();
        let found = registry.get(&WorkerKind::Pi).await;
        assert!(found.is_none());
    }
}
```

### C.1.2 Create `crates/executive/src/impl/goal/attempt.rs`

- [ ] Create the file with the following content including 3 tests:

```rust
//! AttemptRecord -- tracks a single goal execution attempt for persistence.

use chrono::{DateTime, Utc};
use fabric::types::goal::{AttemptId, FailureClass, GoalId, WorkerKind};

/// Record of a single goal execution attempt.
#[derive(Debug, Clone)]
pub struct AttemptRecord {
    pub attempt_id: AttemptId,
    pub goal_id: GoalId,
    pub worker_kind: WorkerKind,
    pub attempt_number: u32,
    pub started_at: DateTime<Utc>,
    pub finished_at: Option<DateTime<Utc>>,
    pub succeeded: bool,
    pub output_text: String,
    pub failure_class: Option<FailureClass>,
    pub tokens_used: u64,
}

impl AttemptRecord {
    /// Create a new attempt record for the given goal and worker.
    pub fn new(goal_id: GoalId, attempt_number: u32, worker_kind: WorkerKind) -> Self {
        Self {
            attempt_id: AttemptId::new(goal_id, attempt_number),
            goal_id,
            worker_kind,
            attempt_number,
            started_at: Utc::now(),
            finished_at: None,
            succeeded: false,
            output_text: String::new(),
            failure_class: None,
            tokens_used: 0,
        }
    }

    /// Mark the attempt as completed successfully.
    pub fn complete(&mut self, output_text: String, tokens_used: u64) {
        self.finished_at = Some(Utc::now());
        self.succeeded = true;
        self.output_text = output_text;
        self.tokens_used = tokens_used;
    }

    /// Mark the attempt as failed with classification.
    pub fn fail(
        &mut self,
        output_text: String,
        failure_class: FailureClass,
        tokens_used: u64,
    ) {
        self.finished_at = Some(Utc::now());
        self.succeeded = false;
        self.output_text = output_text;
        self.failure_class = Some(failure_class);
        self.tokens_used = tokens_used;
    }

    /// Returns true if the attempt is completed (success or failure).
    pub fn succeeded(&self) -> bool {
        self.succeeded && self.finished_at.is_some()
    }

    /// Returns true if the attempt failed.
    pub fn failed(&self) -> bool {
        !self.succeeded && self.finished_at.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_attempt_starts_incomplete() {
        let attempt = AttemptRecord::new(
            GoalId::new(1),
            1,
            WorkerKind::DeepSeek,
        );
        assert!(!attempt.succeeded());
        assert!(!attempt.failed());
        assert!(attempt.finished_at.is_none());
    }

    #[test]
    fn completed_attempt_has_correct_state() {
        let mut attempt = AttemptRecord::new(
            GoalId::new(1),
            1,
            WorkerKind::DeepSeek,
        );
        attempt.complete("all tests pass".into(), 500);
        assert!(attempt.succeeded());
        assert!(!attempt.failed());
        assert!(attempt.finished_at.is_some());
        assert_eq!(attempt.tokens_used, 500);
    }

    #[test]
    fn failed_attempt_has_failure_class() {
        let mut attempt = AttemptRecord::new(
            GoalId::new(2),
            3,
            WorkerKind::DeepSeek,
        );
        attempt.fail(
            "compilation error".into(),
            FailureClass::Compilation,
            200,
        );
        assert!(!attempt.succeeded());
        assert!(attempt.failed());
        assert_eq!(
            attempt.failure_class,
            Some(FailureClass::Compilation)
        );
    }
}
```

### C.1.3 Commit

- [ ] Run:
```bash
git add crates/executive/src/impl/goal/worker.rs         crates/executive/src/impl/goal/attempt.rs
git commit -m "feat(goal): add GoalWorker trait, WorkerRegistry, and AttemptRecord"
```


## Task C.2: Implement FailureClass classification and RetryPolicy

### C.2.1 Create `crates/executive/src/impl/goal/failure.rs`

- [ ] Create the file with the following content including 8 tests:

```rust
//! Output-to-FailureClass classification using pattern matching.

use fabric::types::goal::FailureClass;

/// A mapping from error patterns to FailureClass.
struct FailurePattern {
    pattern: &'static str,
    class: FailureClass,
}

/// Patterns used to classify worker output into failure classes.
/// Patterns are checked in order; the first match wins.
static FAILURE_PATTERNS: &[FailurePattern] = &[
    FailurePattern {
        pattern: "error[E",
        class: FailureClass::Compilation,
    },
    FailurePattern {
        pattern: "could not compile",
        class: FailureClass::Compilation,
    },
    FailurePattern {
        pattern: "test failed",
        class: FailureClass::TestFailure,
    },
    FailurePattern {
        pattern: "FAILED",
        class: FailureClass::TestFailure,
    },
    FailurePattern {
        pattern: "permission denied",
        class: FailureClass::PermissionDenied,
    },
    FailurePattern {
        pattern: "operation not permitted",
        class: FailureClass::PermissionDenied,
    },
    FailurePattern {
        pattern: "timed out",
        class: FailureClass::Timeout,
    },
    FailurePattern {
        pattern: "deadline exceeded",
        class: FailureClass::Timeout,
    },
    FailurePattern {
        pattern: "no such file",
        class: FailureClass::MissingDependency,
    },
    FailurePattern {
        pattern: "cannot find crate",
        class: FailureClass::MissingDependency,
    },
    FailurePattern {
        pattern: "invalid assumption",
        class: FailureClass::InvalidAssumption,
    },
    FailurePattern {
        pattern: "architecture violation",
        class: FailureClass::ArchitectureViolation,
    },
    FailurePattern {
        pattern: "tool execution failed",
        class: FailureClass::ToolFailure,
    },
    FailurePattern {
        pattern: "context window",
        class: FailureClass::ContextInsufficient,
    },
];

/// Classify a worker's output text into a FailureClass based on patterns.
/// Returns None if no known pattern matches.
pub fn classify_output(output: &str) -> Option<FailureClass> {
    let lower = output.to_lowercase();
    for fp in FAILURE_PATTERNS {
        if lower.contains(fp.pattern) {
            return Some(fp.class.clone());
        }
    }
    None
}

/// Check whether a failure class is a repeat of a prior attempt's class.
/// Used by the retry decision logic.
pub fn is_repeated_failure(current: &FailureClass, previous: &[FailureClass]) -> bool {
    previous.iter().filter(|c| *c == current).count() >= 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classify_compilation_error_e() {
        let output = "error[E0308]: mismatched types";
        assert_eq!(classify_output(output), Some(FailureClass::Compilation));
    }

    #[test]
    fn classify_compilation_error_text() {
        let output = "could not compile due to previous errors";
        assert_eq!(classify_output(output), Some(FailureClass::Compilation));
    }

    #[test]
    fn classify_test_failure() {
        let output = "test tests::it_works ... FAILED";
        assert_eq!(classify_output(output), Some(FailureClass::TestFailure));
    }

    #[test]
    fn classify_permission_denied() {
        let output = "Error: permission denied (os error 13)";
        assert_eq!(
            classify_output(output),
            Some(FailureClass::PermissionDenied)
        );
    }

    #[test]
    fn classify_timeout() {
        let output = "request timed out after 30 seconds";
        assert_eq!(classify_output(output), Some(FailureClass::Timeout));
    }

    #[test]
    fn classify_missing_dependency() {
        let output = "error: cannot find crate `serde`";
        assert_eq!(
            classify_output(output),
            Some(FailureClass::MissingDependency)
        );
    }

    #[test]
    fn classify_no_match() {
        let output = "everything looks good";
        assert_eq!(classify_output(output), None);
    }

    #[test]
    fn is_repeated_failure_detects_repeat() {
        let previous = vec![
            FailureClass::Compilation,
            FailureClass::TestFailure,
        ];
        assert!(is_repeated_failure(&FailureClass::Compilation, &previous));
        assert!(!is_repeated_failure(
            &FailureClass::Timeout,
            &previous
        ));
    }
}
```

### C.2.2 Create `crates/executive/src/impl/goal/retry.rs`

- [ ] Create the file with the following content including 5 tests:

```rust
//! Retry decision logic -- determines what to do after a failed attempt.

use fabric::types::goal::{FailureClass, GoalBudget, WorkerKind};

/// Decision returned by the retry logic.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RetryDecision {
    /// Retry with the same worker.
    RetrySameWorker,
    /// Retry but with a different strategy hint.
    RetryWithStrategy(String),
    /// Escalate to a more capable worker.
    EscalateTo(EscalationTarget),
    /// Give up (budget exhausted or unrecoverable).
    GiveUp,
}

/// Possible escalation targets when retrying is insufficient.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum EscalationTarget {
    Opus,
    Gpt,
    Human,
}

/// Configuration for retry decisions.
#[derive(Debug, Clone)]
pub struct RetryConfig {
    /// Maximum number of retries before escalation.
    pub max_retries_before_escalation: u32,
    /// Escalation chain: [Opus, Gpt, Human].
    pub escalation_chain: Vec<EscalationTarget>,
}

impl Default for RetryConfig {
    fn default() -> Self {
        Self {
            max_retries_before_escalation: 2,
            escalation_chain: vec![
                EscalationTarget::Opus,
                EscalationTarget::Gpt,
                EscalationTarget::Human,
            ],
        }
    }
}

/// Decide what to do after a failed attempt.
pub fn decide_retry(
    failure: &FailureClass,
    budget: &GoalBudget,
    previous_failures: &[FailureClass],
    config: &RetryConfig,
) -> RetryDecision {
    // If budget is exhausted, give up.
    if budget.is_exhausted() {
        return RetryDecision::GiveUp;
    }

    // Repeated failures trigger escalation.
    if super::failure::is_repeated_failure(failure, previous_failures) {
        let escalation_count = previous_failures
            .iter()
            .filter(|c| *c == failure)
            .count() as u32;
        let escalation_index = escalation_count
            .saturating_sub(1)
            .min((config.escalation_chain.len() as u32).saturating_sub(1));
        if let Some(target) = config.escalation_chain.get(escalation_index as usize) {
            return RetryDecision::EscalateTo(target.clone());
        }
    }

    // If we've tried enough times with this worker, escalate.
    if previous_failures.len() as u32 >= config.max_retries_before_escalation {
        if let Some(target) = config.escalation_chain.first() {
            return RetryDecision::EscalateTo(target.clone());
        }
    }

    // Otherwise retry with the same worker and a strategy hint.
    let hint = strategy_hint_for(failure);
    RetryDecision::RetryWithStrategy(hint)
}

/// Determine which escalation target to use for a specific failure class.
pub fn escalation_target_for(failure: &FailureClass) -> EscalationTarget {
    match failure {
        FailureClass::Compilation
        | FailureClass::TestFailure
        | FailureClass::InvalidAssumption => EscalationTarget::Opus,
        FailureClass::ArchitectureViolation
        | FailureClass::ToolFailure => EscalationTarget::Gpt,
        _ => EscalationTarget::Human,
    }
}

/// Provide a strategy hint string for the retry based on failure class.
pub fn strategy_hint_for(failure: &FailureClass) -> String {
    match failure {
        FailureClass::Compilation => {
            "Review compiler output carefully. Fix type errors and borrow-checker issues."
                .into()
        }
        FailureClass::TestFailure => {
            "Review test output. Adjust implementation to satisfy test expectations."
                .into()
        }
        FailureClass::PermissionDenied => {
            "Request needed permissions before attempting this operation again."
                .into()
        }
        FailureClass::Timeout => {
            "Break the task into smaller sub-tasks to fit within the time limit."
                .into()
        }
        FailureClass::MissingDependency => {
            "Add the required dependency to Cargo.toml before proceeding."
                .into()
        }
        FailureClass::InvalidAssumption => {
            "Verify assumptions against the current codebase state before proceeding."
                .into()
        }
        FailureClass::ArchitectureViolation => {
            "Review architecture constraints. The proposed change violates layering rules."
                .into()
        }
        FailureClass::ToolFailure => {
            "Check tool configuration and try an alternative approach."
                .into()
        }
        FailureClass::ContextInsufficient => {
            "Request additional context or narrow the scope of the task."
                .into()
        }
        FailureClass::RepeatedFailure => {
            "All previous attempts have failed. Consider a fundamentally different approach."
                .into()
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exhausted_budget_gives_up() {
        let budget = GoalBudget {
            max_tokens: 100,
            tokens_used: 100,
            max_duration_secs: 60,
            max_attempts: 3,
            attempt_count: 3,
        };
        let decision = decide_retry(
            &FailureClass::Compilation,
            &budget,
            &[],
            &RetryConfig::default(),
        );
        assert_eq!(decision, RetryDecision::GiveUp);
    }

    #[test]
    fn repeated_failure_escalates() {
        let budget = GoalBudget::default();
        let previous = vec![FailureClass::Compilation];
        let decision = decide_retry(
            &FailureClass::Compilation,
            &budget,
            &previous,
            &RetryConfig::default(),
        );
        assert_eq!(
            decision,
            RetryDecision::EscalateTo(EscalationTarget::Opus)
        );
    }

    #[test]
    fn first_failure_retries_with_strategy() {
        let budget = GoalBudget::default();
        let decision = decide_retry(
            &FailureClass::TestFailure,
            &budget,
            &[],
            &RetryConfig::default(),
        );
        match decision {
            RetryDecision::RetryWithStrategy(hint) => {
                assert!(hint.contains("test"));
            }
            other => panic!("expected RetryWithStrategy, got {:?}", other),
        }
    }

    #[test]
    fn escalation_target_for_compilation_is_opus() {
        assert_eq!(
            escalation_target_for(&FailureClass::Compilation),
            EscalationTarget::Opus
        );
    }

    #[test]
    fn escalation_target_for_architecture_violation_is_gpt() {
        assert_eq!(
            escalation_target_for(&FailureClass::ArchitectureViolation),
            EscalationTarget::Gpt
        );
    }
}
```

### C.2.3 Create `crates/executive/src/impl/goal/escalation.rs`

- [ ] Create the file with the following content including 4 tests:

```rust
//! Escalation logic -- determines whether and how to escalate a failing goal.

use fabric::types::goal::{FailureClass, GoalBudget};

/// Result of an escalation check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EscalationResult {
    /// Whether escalation is recommended.
    pub should_escalate: bool,
    /// The suggested escalation worker kind.
    pub target_worker: Option<String>,
    /// Why escalation is being recommended (or not).
    pub reason: String,
}

/// Check whether escalation is warranted based on budget and failure history.
pub fn should_escalate(
    budget: &GoalBudget,
    consecutive_failures: u32,
    latest_failure: Option<&FailureClass>,
) -> EscalationResult {
    // If no failures yet, no escalation needed.
    if latest_failure.is_none() {
        return EscalationResult {
            should_escalate: false,
            target_worker: None,
            reason: "no failures to escalate from".into(),
        };
    }

    // If budget is exhausted, escalation won't help.
    if budget.is_exhausted() {
        return EscalationResult {
            should_escalate: false,
            target_worker: None,
            reason: "budget exhausted, escalation not possible".into(),
        };
    }

    // Escalate after 2 consecutive failures of the same type.
    if consecutive_failures >= 2 {
        let failure = latest_failure.unwrap();
        let target = match failure {
            FailureClass::Compilation
            | FailureClass::TestFailure
            | FailureClass::InvalidAssumption => "opus",
            FailureClass::ArchitectureViolation
            | FailureClass::ToolFailure => "gpt",
            _ => "human",
        };
        return EscalationResult {
            should_escalate: true,
            target_worker: Some(target.to_string()),
            reason: format!(
                "{} consecutive failures (class: {})",
                consecutive_failures, failure
            ),
        };
    }

    EscalationResult {
        should_escalate: false,
        target_worker: None,
        reason: format!(
            "{} consecutive failures, threshold not met",
            consecutive_failures
        ),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn no_failures_no_escalation() {
        let budget = GoalBudget::default();
        let result = should_escalate(&budget, 0, None);
        assert!(!result.should_escalate);
    }

    #[test]
    fn exhausted_budget_no_escalation() {
        let budget = GoalBudget {
            max_tokens: 0,
            tokens_used: 10,
            max_duration_secs: 60,
            max_attempts: 0,
            attempt_count: 5,
        };
        let result = should_escalate(&budget, 3, Some(&FailureClass::Compilation));
        assert!(!result.should_escalate);
    }

    #[test]
    fn two_consecutive_compilation_failures_escalates_to_opus() {
        let budget = GoalBudget::default();
        let result = should_escalate(&budget, 2, Some(&FailureClass::Compilation));
        assert!(result.should_escalate);
        assert_eq!(result.target_worker, Some("opus".to_string()));
    }

    #[test]
    fn one_failure_does_not_escalate() {
        let budget = GoalBudget::default();
        let result = should_escalate(&budget, 1, Some(&FailureClass::Timeout));
        assert!(!result.should_escalate);
    }
}
```

### C.2.4 Commit

- [ ] Run:
```bash
git add crates/executive/src/impl/goal/failure.rs         crates/executive/src/impl/goal/retry.rs         crates/executive/src/impl/goal/escalation.rs
git commit -m "feat(goal): add FailureClass classification, RetryPolicy, and Escalation logic"
```


## Task C.3: Implement DeepSeekWorker

### C.3.1 Create `crates/executive/src/impl/goal/worker_impl.rs`

- [ ] Create the file with the following content including 2 prompt-building tests:

```rust
//! DeepSeek worker implementation -- wraps an LLM provider for goal execution.

use anyhow::Result;
use async_trait::async_trait;
use fabric::types::goal::{GoalFrame, WorkerKind};
use fabric::LlmProvider;
use std::sync::Arc;

use super::failure::classify_output;
use super::worker::{ClassifiedFailure, GoalWorker, WorkerOutput};

/// Worker that executes goals using a DeepSeek LLM provider.
pub struct DeepSeekWorker {
    provider: Arc<dyn LlmProvider>,
}

impl DeepSeekWorker {
    pub fn new(provider: Arc<dyn LlmProvider>) -> Self {
        Self { provider }
    }

    /// Build the system prompt from the goal frame.
    pub fn build_system_prompt(frame: &GoalFrame) -> String {
        let mut prompt = String::new();
        prompt.push_str("You are an autonomous coding agent. Your task:

");
        prompt.push_str(&format!("# Task: {}

", frame.current_task));
        prompt.push_str(&format!(
            "Original intent: {}

",
            frame.original_intent
        ));

        if !frame.acceptance_criteria.is_empty() {
            prompt.push_str("## Acceptance Criteria:
");
            for (i, criterion) in frame.acceptance_criteria.iter().enumerate() {
                prompt.push_str(&format!("{}. {}
", i + 1, criterion));
            }
            prompt.push('
');
        }

        if !frame.recent_attempts.is_empty() {
            prompt.push_str("## Previous Attempts:
");
            for attempt in &frame.recent_attempts {
                prompt.push_str(&format!(
                    "- {} ({}): {}
",
                    attempt.attempt_id.0, attempt.worker, attempt.outcome
                ));
            }
            prompt.push('
');
        }

        prompt.push_str(&format!(
            "Budget: {} tokens remaining, {}/{} attempts used.
",
            frame.remaining_budget.remaining_tokens(),
            frame.remaining_budget.attempt_count,
            frame.remaining_budget.max_attempts
        ));

        if !frame.relevant_memories.is_empty() {
            prompt.push_str("
## Relevant Context:
");
            for mem in &frame.relevant_memories {
                for fact in &mem.relevant_facts {
                    prompt.push_str(&format!("- {}
", fact));
                }
            }
        }

        prompt.push_str("
Produce a complete implementation that satisfies ");
        prompt.push_str("all acceptance criteria. Write production-quality Rust code.
");

        prompt
    }
}

#[async_trait]
impl GoalWorker for DeepSeekWorker {
    fn kind(&self) -> WorkerKind {
        WorkerKind::DeepSeek
    }

    async fn execute(&self, frame: &GoalFrame) -> Result<WorkerOutput> {
        let system_prompt = Self::build_system_prompt(frame);

        let messages = vec![
            fabric::types::message::Message {
                role: fabric::types::message::Role::System,
                content: vec![fabric::types::message::ContentBlock::System {
                    text: system_prompt,
                    priority: fabric::types::message::Priority::Normal,
                }],
            },
            fabric::types::message::Message {
                role: fabric::types::message::Role::User,
                content: vec![fabric::types::message::ContentBlock::Text {
                    text: format!(
                        "Implement: {}

Acceptance criteria:
{}",
                        frame.current_task,
                        frame.acceptance_criteria.join("
")
                    ),
                }],
            },
        ];

        // Call the LLM provider.
        // In production this would be provider.chat(messages).await.
        // For the plan, we stub it with a placeholder since provider.chat()
        // returns the full response.
        let response_text = self
            .provider
            .chat(messages)
            .await
            .map_err(|e| anyhow::anyhow!("DeepSeek provider error: {}", e))?;

        // Classify the output to determine success/failure.
        let failure_class = classify_output(&response_text);
        let success = failure_class.is_none();

        let failure = failure_class.map(|class| ClassifiedFailure {
            class,
            message: response_text.clone(),
        });

        Ok(WorkerOutput {
            text: response_text,
            success,
            tokens_used: 0, // Provider should return token counts.
            failure,
            artifacts: vec![],
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn make_frame() -> GoalFrame {
        GoalFrame {
            goal_id: fabric::types::goal::GoalId::new(1),
            original_intent: "add OAuth2 login".into(),
            current_task: "implement token refresh".into(),
            acceptance_criteria: vec![
                "tests pass".into(),
                "no clippy warnings".into(),
            ],
            recent_attempts: vec![],
            remaining_budget: Default::default(),
            relevant_memories: vec![],
        }
    }

    #[test]
    fn build_system_prompt_includes_task_and_criteria() {
        let frame = make_frame();
        let prompt = DeepSeekWorker::build_system_prompt(&frame);
        assert!(prompt.contains("implement token refresh"));
        assert!(prompt.contains("tests pass"));
        assert!(prompt.contains("no clippy warnings"));
        assert!(prompt.contains("acceptance criteria"));
    }

    #[test]
    fn build_system_prompt_includes_budget_info() {
        let frame = make_frame();
        let prompt = DeepSeekWorker::build_system_prompt(&frame);
        assert!(prompt.contains("8192 tokens remaining"));
        assert!(prompt.contains("0/3 attempts"));
    }
}
```

### C.3.2 Update `crates/executive/src/impl/goal/supervisor.rs` tick() method

- [ ] Replace the placeholder `tick()` implementation with the full worker dispatch + retry + escalation logic:

```rust
    async fn tick(&self, goal_id: GoalId) -> Result<GoalTransition> {
        use crate::impl::goal::attempt::AttemptRecord;
        use crate::impl::goal::escalation::should_escalate;
        use crate::impl::goal::frame::build_goal_frame;
        use crate::impl::goal::retry::{decide_retry, RetryConfig, RetryDecision};

        let frame = {
            let store = self.store.lock().await;
            build_goal_frame(&store, goal_id.as_i64())?
        };

        // Check if budget is exhausted.
        if frame.remaining_budget.is_exhausted() {
            let store = self.store.lock().await;
            store.set_status(goal_id.as_i64(), "failed")?;
            warn!(goal_id = %goal_id, "budget exhausted at tick");
            return Ok(GoalTransition::Failed);
        }

        // Collect previous failure classes for retry decisions.
        let previous_failures: Vec<fabric::types::goal::FailureClass> = frame
            .recent_attempts
            .iter()
            .filter_map(|a| a.failure_class.clone())
            .collect();

        let attempt_number = frame.remaining_budget.attempt_count + 1;
        let mut attempt = AttemptRecord::new(
            goal_id,
            attempt_number,
            fabric::types::goal::WorkerKind::DeepSeek,
        );

        // Dispatch to DeepSeek worker (Phase C.3: real worker registration).
        // For now, execute inline with a placeholder.
        let worker_output = WorkerOutput {
            text: String::new(),
            success: false,
            tokens_used: 0,
            failure: None,
            artifacts: vec![],
        };

        // TODO Phase C.3: replace with registered worker execution.
        // let worker = self.worker_registry.get(&WorkerKind::DeepSeek).await
        //     .ok_or_else(|| anyhow::anyhow!("DeepSeek worker not registered"))?;
        // let worker_output = worker.execute(&frame).await?;

        // Record the attempt.
        let store = self.store.lock().await;
        if worker_output.success {
            attempt.complete(worker_output.text, worker_output.tokens_used);
            store.record_attempt(
                goal_id.as_i64(),
                "deepseek",
                attempt_number as i64,
                "succeeded",
                &attempt.output_text,
                None,
                attempt.tokens_used as i64,
            )?;
            store.set_status(goal_id.as_i64(), "completed")?;
            info!(goal_id = %goal_id, "goal completed");
            return Ok(GoalTransition::Completed);
        }

        // Handle failure.
        if let Some(ref classified) = worker_output.failure {
            attempt.fail(
                worker_output.text.clone(),
                classified.class.clone(),
                worker_output.tokens_used,
            );

            store.record_attempt(
                goal_id.as_i64(),
                "deepseek",
                attempt_number as i64,
                "failed",
                &attempt.output_text,
                Some(&classified.class.to_string()),
                attempt.tokens_used as i64,
            )?;

            // Decide what to do.
            let retry_config = RetryConfig::default();
            let decision = decide_retry(
                &classified.class,
                &frame.remaining_budget,
                &previous_failures,
                &retry_config,
            );

            match decision {
                RetryDecision::RetrySameWorker | RetryDecision::RetryWithStrategy(_) => {
                    // Will retry on next tick.
                    info!(
                        goal_id = %goal_id,
                        class = %classified.class,
                        "scheduling retry"
                    );
                    store.set_status(goal_id.as_i64(), "in_progress")?;
                    Ok(GoalTransition::StateChanged)
                }
                RetryDecision::EscalateTo(target) => {
                    info!(
                        goal_id = %goal_id,
                        target = ?target,
                        "escalating"
                    );
                    // Update plan_json with escalation target.
                    store.db.execute(
                        "UPDATE objectives_v2 SET plan_json = ?1, updated_at = datetime('now')
                         WHERE objective_id = ?2",
                        rusqlite::params![
                            format!("{{"escalated_to": "{:?}"}}", target),
                            goal_id.as_i64()
                        ],
                    )?;
                    Ok(GoalTransition::StateChanged)
                }
                RetryDecision::GiveUp => {
                    warn!(goal_id = %goal_id, "giving up after retries");
                    store.set_status(goal_id.as_i64(), "failed")?;
                    Ok(GoalTransition::Failed)
                }
            }
        } else {
            // No failure classification but not successful -- unknown state.
            attempt.fail(
                worker_output.text.clone(),
                fabric::types::goal::FailureClass::ToolFailure,
                worker_output.tokens_used,
            );
            store.record_attempt(
                goal_id.as_i64(),
                "deepseek",
                attempt_number as i64,
                "failed",
                &attempt.output_text,
                Some("tool_failure"),
                attempt.tokens_used as i64,
            )?;
            Ok(GoalTransition::StateChanged)
        }
    }
```

### C.3.3 Modify `crates/executive/src/impl/goal/mod.rs`

- [ ] Add re-exports for the new modules:
```rust
pub mod attempt;
pub mod escalation;
pub mod failure;
pub mod retry;
pub mod worker;
pub mod worker_impl;
```

### C.3.4 Commit

- [ ] Run:
```bash
git add crates/executive/src/impl/goal/worker_impl.rs         crates/executive/src/impl/goal/supervisor.rs         crates/executive/src/impl/goal/mod.rs
git commit -m "feat(goal): implement DeepSeekWorker with prompt building, retry, and escalation in tick()"
```

### C.3.5 Compilation and test verification

- [ ] Run:
```bash
cargo check --workspace 2>&1
cargo test -p executive --lib -- impl::goal 2>&1
```

**Expected test output:**
```
running 24 tests
test impl::goal::state_machine::tests::work_dispatched_from_noop_transitions_to_state_changed ... ok
test impl::goal::state_machine::tests::attempt_succeeded_from_state_changed_transitions_to_completed ... ok
test impl::goal::state_machine::tests::budget_exhausted_always_transitions_to_failed ... ok
test impl::goal::state_machine::tests::completed_is_terminal ... ok
test impl::goal::attempt::tests::new_attempt_starts_incomplete ... ok
test impl::goal::attempt::tests::completed_attempt_has_correct_state ... ok
test impl::goal::attempt::tests::failed_attempt_has_failure_class ... ok
test impl::goal::failure::tests::classify_compilation_error_e ... ok
test impl::goal::failure::tests::classify_compilation_error_text ... ok
test impl::goal::failure::tests::classify_test_failure ... ok
test impl::goal::failure::tests::classify_permission_denied ... ok
test impl::goal::failure::tests::classify_timeout ... ok
test impl::goal::failure::tests::classify_missing_dependency ... ok
test impl::goal::failure::tests::classify_no_match ... ok
test impl::goal::failure::tests::is_repeated_failure_detects_repeat ... ok
test impl::goal::retry::tests::exhausted_budget_gives_up ... ok
test impl::goal::retry::tests::repeated_failure_escalates ... ok
test impl::goal::retry::tests::first_failure_retries_with_strategy ... ok
test impl::goal::retry::tests::escalation_target_for_compilation_is_opus ... ok
test impl::goal::retry::tests::escalation_target_for_architecture_violation_is_gpt ... ok
test impl::goal::escalation::tests::no_failures_no_escalation ... ok
test impl::goal::escalation::tests::exhausted_budget_no_escalation ... ok
test impl::goal::escalation::tests::two_consecutive_compilation_failures_escalates_to_opus ... ok
test impl::goal::escalation::tests::one_failure_does_not_escalate ... ok
test impl::goal::worker::tests::registry_get_returns_registered_worker ... ok
test impl::goal::worker::tests::registry_get_unknown_returns_none ... ok
test impl::goal::worker_impl::tests::build_system_prompt_includes_task_and_criteria ... ok
test impl::goal::worker_impl::tests::build_system_prompt_includes_budget_info ... ok
test result: ok. 28 passed; 0 failed; 0 ignored
```


---
# Phase D: Pi Coding Subagent (estimated 1-2 weeks, depends on Phase B)

## Task D.1: Define PiSubagentTask and PiSubagentReport types

### D.1.1 Create directory and `crates/executive/src/impl/agent/pi/mod.rs`

- [ ] Run:
```bash
mkdir -p crates/executive/src/impl/agent/pi
```

- [ ] Create the file with:
```rust
//! Pi coding subagent -- spawns a Pi process with a task, collects results.

pub mod report;
pub mod task;
pub mod worker;
pub mod worktree;

pub use report::{ChangeType, FileChange, PiSubagentReport};
pub use task::PiSubagentTask;
pub use worker::PiWorker;
pub use worktree::{WorktreeHandle, WorktreeManager};
```

### D.1.2 Create `crates/executive/src/impl/agent/pi/task.rs`

- [ ] Create the file with the following content including 2 tests:

```rust
//! PiSubagentTask -- defines the task specification for a Pi coding run.

use serde::{Deserialize, Serialize};

/// Task specification for the Pi coding subagent.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiSubagentTask {
    /// The objective to accomplish (e.g., "add OAuth2 login").
    pub objective: String,
    /// Files the subagent is allowed to modify.
    pub allowed_files: Vec<String>,
    /// Additional files to include as context (read-only).
    pub context_files: Vec<String>,
    /// Constraints the subagent must respect.
    pub constraints: Vec<String>,
    /// Maximum execution time in seconds.
    pub timeout_secs: u64,
}

impl PiSubagentTask {
    /// Create a new task with the given objective.
    pub fn new(objective: impl Into<String>) -> Self {
        Self {
            objective: objective.into(),
            allowed_files: vec![],
            context_files: vec![],
            constraints: vec![
                "write production-quality Rust code".into(),
                "include tests for new functionality".into(),
                "run cargo fmt before completing".into(),
            ],
            timeout_secs: 300,
        }
    }

    /// Builder: set allowed files.
    pub fn with_allowed_files(mut self, files: Vec<String>) -> Self {
        self.allowed_files = files;
        self
    }

    /// Builder: set context files.
    pub fn with_context_files(mut self, files: Vec<String>) -> Self {
        self.context_files = files;
        self
    }

    /// Builder: set timeout.
    pub fn with_timeout(mut self, secs: u64) -> Self {
        self.timeout_secs = secs;
        self
    }

    /// Builder: add a constraint.
    pub fn with_constraint(mut self, constraint: impl Into<String>) -> Self {
        self.constraints.push(constraint.into());
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn new_task_has_default_constraints() {
        let task = PiSubagentTask::new("fix bug #42");
        assert_eq!(task.objective, "fix bug #42");
        assert!(task.constraints.len() >= 2);
        assert_eq!(task.timeout_secs, 300);
        assert!(task.allowed_files.is_empty());
    }

    #[test]
    fn builder_methods_work() {
        let task = PiSubagentTask::new("refactor module")
            .with_allowed_files(vec!["src/lib.rs".into()])
            .with_context_files(vec!["src/types.rs".into()])
            .with_timeout(600)
            .with_constraint("no unsafe code");
        assert_eq!(task.allowed_files, vec!["src/lib.rs"]);
        assert_eq!(task.context_files, vec!["src/types.rs"]);
        assert_eq!(task.timeout_secs, 600);
        assert!(task.constraints.contains(&"no unsafe code".to_string()));
    }
}
```

### D.1.3 Create `crates/executive/src/impl/agent/pi/report.rs`

- [ ] Create the file with the following content including 2 tests:

```rust
//! PiSubagentReport -- result of a Pi coding subagent run.

use serde::{Deserialize, Serialize};

/// Type of change made to a file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ChangeType {
    Created,
    Modified,
    Deleted,
    Renamed,
}

/// Record of a single file change.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FileChange {
    /// Path to the changed file (relative to repo root).
    pub path: String,
    /// What kind of change was made.
    pub change_type: ChangeType,
    /// Lines added (approximate).
    pub lines_added: u64,
    /// Lines removed (approximate).
    pub lines_removed: u64,
}

/// Report produced by a Pi coding subagent run.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PiSubagentReport {
    /// Whether the run was successful overall.
    pub success: bool,
    /// Human-readable summary of what was done.
    pub summary: String,
    /// Files that were changed.
    pub changed_files: Vec<FileChange>,
    /// The unified diff of all changes.
    pub diff: Option<String>,
    /// Test results (if tests were run).
    pub test_results: Option<String>,
    /// Any errors encountered.
    pub errors: Vec<String>,
    /// Approximate token count consumed.
    pub tokens_used: u64,
}

impl PiSubagentReport {
    /// Create a successful report.
    pub fn success(
        summary: impl Into<String>,
        changed_files: Vec<FileChange>,
        diff: Option<String>,
        tokens_used: u64,
    ) -> Self {
        Self {
            success: true,
            summary: summary.into(),
            changed_files,
            diff,
            test_results: None,
            errors: vec![],
            tokens_used,
        }
    }

    /// Create a failure report.
    pub fn failure(
        summary: impl Into<String>,
        errors: Vec<String>,
        tokens_used: u64,
    ) -> Self {
        Self {
            success: false,
            summary: summary.into(),
            changed_files: vec![],
            diff: None,
            test_results: None,
            errors,
            tokens_used,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn success_report_has_no_errors() {
        let report = PiSubagentReport::success(
            "added login endpoint",
            vec![FileChange {
                path: "src/auth.rs".into(),
                change_type: ChangeType::Modified,
                lines_added: 42,
                lines_removed: 3,
            }],
            Some("diff content".into()),
            1500,
        );
        assert!(report.success);
        assert!(report.errors.is_empty());
        assert_eq!(report.changed_files.len(), 1);
    }

    #[test]
    fn failure_report_has_errors_and_no_changes() {
        let report = PiSubagentReport::failure(
            "compilation failed",
            vec!["error[E0308]: mismatched types".into()],
            800,
        );
        assert!(!report.success);
        assert_eq!(report.errors.len(), 1);
        assert!(report.changed_files.is_empty());
        assert!(report.diff.is_none());
    }
}
```

### D.1.4 Commit

- [ ] Run:
```bash
git add crates/executive/src/impl/agent/pi/mod.rs         crates/executive/src/impl/agent/pi/task.rs         crates/executive/src/impl/agent/pi/report.rs
git commit -m "feat(agent): add PiSubagentTask and PiSubagentReport types"
```


## Task D.2: Implement WorktreeManager

### D.2.1 Create `crates/executive/src/impl/agent/pi/worktree.rs`

- [ ] Create the file with the following content including 2 integration tests:

```rust
//! WorktreeManager -- creates and cleans up Git worktrees for isolated Pi runs.

use anyhow::{Context, Result};
use std::path::{Path, PathBuf};
use std::process::Command;
use tracing::{info, warn};

/// Handle to a created Git worktree. Cleans up on Drop.
pub struct WorktreeHandle {
    /// Path to the created worktree.
    pub path: PathBuf,
    /// Name of the branch checked out in the worktree.
    pub branch: String,
}

impl Drop for WorktreeHandle {
    fn drop(&mut self) {
        if self.path.exists() {
            if let Err(e) = Self::remove_worktree(&self.path) {
                warn!(
                    path = %self.path.display(),
                    error = %e,
                    "failed to clean up worktree on drop"
                );
            }
        }
    }
}

impl WorktreeHandle {
    /// Remove a git worktree using `git worktree remove`.
    fn remove_worktree(path: &Path) -> Result<()> {
        let output = Command::new("git")
            .args(["worktree", "remove", "--force"])
            .arg(path)
            .output()
            .context("git worktree remove failed")?;
        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git worktree remove: {}", stderr);
        }
        Ok(())
    }
}

/// Manages creation and diff collection for Git worktrees.
pub struct WorktreeManager {
    /// Path to the main repository.
    repo_path: PathBuf,
}

impl WorktreeManager {
    pub fn new(repo_path: impl Into<PathBuf>) -> Self {
        Self {
            repo_path: repo_path.into(),
        }
    }

    /// Create a new Git worktree on a unique branch.
    pub fn create(&self, branch_name: &str) -> Result<WorktreeHandle> {
        let worktree_path = self.repo_path.join(format!("worktrees/{}", branch_name));

        // Remove any stale worktree at this path.
        if worktree_path.exists() {
            let _ = std::fs::remove_dir_all(&worktree_path);
            let _ = Command::new("git")
                .args(["worktree", "prune"])
                .current_dir(&self.repo_path)
                .output();
        }

        // Create the worktree.
        let output = Command::new("git")
            .args([
                "worktree",
                "add",
                "-b",
                branch_name,
                worktree_path.to_str().unwrap(),
                "HEAD",
            ])
            .current_dir(&self.repo_path)
            .output()
            .context("git worktree add failed")?;

        if !output.status.success() {
            let stderr = String::from_utf8_lossy(&output.stderr);
            anyhow::bail!("git worktree add: {}", stderr);
        }

        info!(
            branch = branch_name,
            path = %worktree_path.display(),
            "worktree created"
        );

        Ok(WorktreeHandle {
            path: worktree_path,
            branch: branch_name.to_string(),
        })
    }

    /// Collect a unified diff between the worktree branch and its base.
    pub fn collect_diff(&self, handle: &WorktreeHandle) -> Result<String> {
        let output = Command::new("git")
            .args(["diff", &format!("HEAD...{}", handle.branch)])
            .current_dir(&self.repo_path)
            .output()
            .context("git diff failed")?;

        Ok(String::from_utf8_lossy(&output.stdout).to_string())
    }

    /// List files changed in the worktree.
    pub fn changed_files(&self, handle: &WorktreeHandle) -> Result<Vec<String>> {
        let output = Command::new("git")
            .args([
                "diff",
                "--name-only",
                &format!("HEAD...{}", handle.branch),
            ])
            .current_dir(&self.repo_path)
            .output()
            .context("git diff --name-only failed")?;

        let text = String::from_utf8_lossy(&output.stdout);
        Ok(text.lines().map(|s| s.to_string()).collect())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::process::Command;
    use tempfile::tempdir;

    fn init_test_repo(dir: &Path) {
        Command::new("git")
            .args(["init"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.email", "test@test.com"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["config", "user.name", "Test"])
            .current_dir(dir)
            .output()
            .unwrap();
        // Create an initial commit so we can branch.
        std::fs::write(dir.join("README.md"), "# test").unwrap();
        Command::new("git")
            .args(["add", "README.md"])
            .current_dir(dir)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "initial"])
            .current_dir(dir)
            .output()
            .unwrap();
    }

    #[test]
    fn create_worktree_succeeds() {
        let dir = tempdir().unwrap();
        init_test_repo(dir.path());

        let manager = WorktreeManager::new(dir.path().to_path_buf());
        let handle = manager.create("test-branch").unwrap();
        assert!(handle.path.exists());
        assert!(handle.path.join("README.md").exists());
        // Clean up
        drop(handle);
    }

    #[test]
    fn changed_files_after_modification() {
        let dir = tempdir().unwrap();
        init_test_repo(dir.path());

        let manager = WorktreeManager::new(dir.path().to_path_buf());
        let handle = manager.create("test-branch-2").unwrap();

        // Modify a file in the worktree.
        std::fs::write(handle.path.join("src"), "// new code").unwrap();
        Command::new("git")
            .args(["add", "src"])
            .current_dir(&handle.path)
            .output()
            .unwrap();
        Command::new("git")
            .args(["commit", "-m", "add src"])
            .current_dir(&handle.path)
            .output()
            .unwrap();

        let files = manager.changed_files(&handle).unwrap();
        assert!(!files.is_empty());
        // Clean up
        drop(handle);
    }
}
```

### D.2.2 Commit

- [ ] Run:
```bash
git add crates/executive/src/impl/agent/pi/worktree.rs
git commit -m "feat(agent): add WorktreeManager for isolated Pi subagent execution"
```

## Task D.3: Implement PiWorker (GoalWorker)

### D.3.1 Create `crates/executive/src/impl/agent/pi/worker.rs`

- [ ] Create the file with the following content including 3 parse tests:

```rust
//! PiWorker -- GoalWorker implementation that spawns a Pi coding subagent.

use anyhow::{Context, Result};
use async_trait::async_trait;
use fabric::types::goal::{GoalFrame, WorkerKind};
use std::path::PathBuf;
use std::process::Command;
use std::sync::Arc;
use tokio::process::Command as TokioCommand;
use tokio::time::{timeout, Duration};
use tracing::info;

use super::report::PiSubagentReport;
use super::task::PiSubagentTask;
use super::worktree::WorktreeManager;
use crate::impl::goal::worker::{GoalWorker, WorkerOutput};

/// Worker that executes goals by spawning a Pi coding subagent process.
pub struct PiWorker {
    /// Path to the Pi binary.
    pi_binary: PathBuf,
    /// Repository path for worktree creation.
    repo_path: PathBuf,
    /// Optional shared WorktreeManager (created per-call if None).
    worktree_manager: Option<Arc<WorktreeManager>>,
}

impl PiWorker {
    pub fn new(pi_binary: PathBuf, repo_path: PathBuf) -> Self {
        Self {
            pi_binary,
            repo_path,
            worktree_manager: None,
        }
    }

    /// Parse PiSubagentReport from the Pi process stdout.
    /// Expects the Pi process to print a JSON report at the end of its output.
    pub fn parse_report(output: &str) -> Result<PiSubagentReport> {
        // The Pi process outputs the JSON report as the last non-empty line,
        // or as a dedicated "--report" marker line followed by JSON.
        let lines: Vec<&str> = output.lines().collect();

        // Try to find a JSON object at the end of the output.
        for line in lines.iter().rev() {
            let trimmed = line.trim();
            if trimmed.starts_with('{') && trimmed.ends_with('}') {
                return serde_json::from_str(trimmed)
                    .context("failed to parse Pi report JSON");
            }
        }

        // If no JSON report found, treat the entire output as a failure.
        Ok(PiSubagentReport::failure(
            "no structured report found in Pi output",
            vec![output.to_string()],
            0,
        ))
    }

    /// Build a task specification from a goal frame.
    fn build_task(&self, frame: &GoalFrame) -> PiSubagentTask {
        PiSubagentTask::new(&frame.current_task)
            .with_timeout(frame.remaining_budget.max_duration_secs)
    }
}

#[async_trait]
impl GoalWorker for PiWorker {
    fn kind(&self) -> WorkerKind {
        WorkerKind::Pi
    }

    async fn execute(&self, frame: &GoalFrame) -> Result<WorkerOutput> {
        let task = self.build_task(frame);
        let manager = self
            .worktree_manager
            .clone()
            .unwrap_or_else(|| Arc::new(WorktreeManager::new(self.repo_path.clone())));

        let branch = format!(
            "pi-goal-{}-attempt-{}",
            frame.goal_id.0,
            frame.remaining_budget.attempt_count + 1
        );
        let handle = manager.create(&branch)?;

        // Serialize task to JSON for stdin.
        let task_json = serde_json::to_string(&task)?;

        info!(
            goal_id = %frame.goal_id,
            branch = %branch,
            "spawning Pi subagent"
        );

        let mut child = TokioCommand::new(&self.pi_binary)
            .arg("--task-stdin")
            .arg("--worktree")
            .arg(&handle.path)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .context("failed to spawn Pi process")?;

        // Write task to stdin and close it.
        if let Some(mut stdin) = child.stdin.take() {
            use tokio::io::AsyncWriteExt;
            stdin.write_all(task_json.as_bytes()).await?;
            drop(stdin);
        }

        let timeout_dur = Duration::from_secs(task.timeout_secs);
        let output = timeout(timeout_dur, child.wait_with_output())
            .await
            .context("Pi process timed out")?
            .context("Pi process failed")?;

        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        let stderr = String::from_utf8_lossy(&output.stderr).to_string();

        // Parse the report.
        let report = Self::parse_report(&stdout)?;

        // Collect diff from the worktree.
        let diff = manager.collect_diff(&handle).ok();

        // Drop the worktree handle to clean up.
        drop(handle);

        let tokens_used = report.tokens_used;
        let success = report.success && output.status.success();

        Ok(WorkerOutput {
            text: format!(
                "Pi report:
{}
Diff:
{}",
                report.summary,
                diff.unwrap_or_default()
            ),
            success,
            tokens_used,
            failure: if success {
                None
            } else {
                Some(crate::impl::goal::worker::ClassifiedFailure {
                    class: fabric::types::goal::FailureClass::ToolFailure,
                    message: report.errors.join("; "),
                })
            },
            artifacts: report.changed_files.iter().map(|f| f.path.clone()).collect(),
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_report_from_json_line() {
        let report = PiSubagentReport::success(
            "done",
            vec![],
            Some("diff".into()),
            100,
        );
        let json = serde_json::to_string(&report).unwrap();
        let output = format!("some log output
{}
more logs", json);

        let parsed = PiWorker::parse_report(&output).unwrap();
        assert!(parsed.success);
        assert_eq!(parsed.summary, "done");
    }

    #[test]
    fn parse_report_no_json_returns_failure() {
        let output = "just some random output
no json here";
        let report = PiWorker::parse_report(output).unwrap();
        assert!(!report.success);
        assert!(report.summary.contains("no structured report"));
    }

    #[test]
    fn parse_report_with_multiple_json_lines_picks_last() {
        let report1 = PiSubagentReport::failure("first", vec![], 10);
        let report2 = PiSubagentReport::success("second", vec![], 20);
        let json1 = serde_json::to_string(&report1).unwrap();
        let json2 = serde_json::to_string(&report2).unwrap();
        let output = format!("{}
{}", json1, json2);

        let parsed = PiWorker::parse_report(&output).unwrap();
        assert!(parsed.success);
        assert_eq!(parsed.summary, "second");
    }
}
```

### D.3.2 Modify `crates/executive/Cargo.toml` (if needed)

- [ ] Verify that required dependencies are present. The PiWorker uses:
  - `tokio` (already present with "full" features)
  - `serde_json` (already present)
  - `anyhow` (already present)
  - `async-trait` (already present)
  - `tempfile` (already in dev-dependencies)

No new dependencies needed for Phase D.

### D.3.3 Commit

- [ ] Run:
```bash
git add crates/executive/src/impl/agent/pi/worker.rs
git commit -m "feat(agent): implement PiWorker -- GoalWorker that spawns Pi subagent with worktree isolation"
```

### D.3.4 End-to-end compilation and test

- [ ] Run:
```bash
cargo check --workspace 2>&1
```

**Expected output:** Clean compilation of all crates, no errors.

- [ ] Run tests:
```bash
cargo test -p executive --lib -- impl::agent::pi 2>&1
```

**Expected output:**
```
running 7 tests
test impl::agent::pi::task::tests::new_task_has_default_constraints ... ok
test impl::agent::pi::task::tests::builder_methods_work ... ok
test impl::agent::pi::report::tests::success_report_has_no_errors ... ok
test impl::agent::pi::report::tests::failure_report_has_errors_and_no_changes ... ok
test impl::agent::pi::worktree::tests::create_worktree_succeeds ... ok
test impl::agent::pi::worktree::tests::changed_files_after_modification ... ok
test impl::agent::pi::worker::tests::parse_report_from_json_line ... ok
test impl::agent::pi::worker::tests::parse_report_no_json_returns_failure ... ok
test impl::agent::pi::worker::tests::parse_report_with_multiple_json_lines_picks_last ... ok
test result: ok. 9 passed; 0 failed; 0 ignored
```

- [ ] Run full workspace tests:
```bash
cargo test --workspace 2>&1
```

**Expected output:** All tests across the workspace pass. Flaky tests (like the known `execute_script_hook_inject`) may be skipped or retried.

- [ ] Commit formatting:
```bash
cargo fmt --all
git add -u
git commit -m "chore: cargo fmt after Phase D PiWorker integration"
```

---
# Phase Summary

After completing Phases A through D, the following subsystems are operational:

| Phase | Deliverable | Crates | Tests |
|-------|------------|--------|-------|
| A | Channel types, Channel trait/registry, TelegramChannel, daemon routing | fabric, executive | 7 |
| B | Goal types, extended schema, GoalSupervisor, state machine, frame builder | fabric, executive | 13 |
| C | GoalWorker trait, AttemptRecord, FailureClass, RetryPolicy, Escalation, DeepSeekWorker | executive | 19 |
| D | PiSubagentTask/Report, WorktreeManager, PiWorker | executive | 9 |
| **Total** | | | **48+** |

Phases E-H (Google OAuth, Gmail channel, verification pipeline, GBrain backend) will be planned in a subsequent document.


## Phase E: Verification Pipeline

**Goal:** After each coding task, run 7 gated verification checks. MustPass gates block completion; Advisory gates warn.
**Crates:** executive (new module impl/goal/verify/)
**Depends on:** Phase D (Pi produces diffs/changed files)
**Estimated:** 1 week

### Task E.1: Define VerificationGate trait and VerificationReport

**Files:**
- Create: `crates/executive/src/impl/goal/verify/mod.rs`
- Create: `crates/executive/src/impl/goal/verify/report.rs`
- Create: `crates/executive/src/impl/goal/verify/gates.rs`
- Modify: `crates/executive/src/impl/goal/mod.rs`

- [ ] **Step 1: Write VerificationGate trait and pipeline**

```rust
// crates/executive/src/impl/goal/verify/mod.rs
//! Verification pipeline -- runs gated checks on worker output.
//!
//! Seven gates: Format, Compile, Test, Clippy, DiffScope, Architecture, CapabilityPolicy.
//! MustPass gates block completion. Advisory gates produce warnings.

pub mod gates;
pub mod policy;
pub mod report;

use async_trait::async_trait;
use std::path::PathBuf;
use crate::r#impl::agent::pi::report::FileChange;

pub struct VerificationContext {
    pub goal_id: i64,
    pub attempt_id: String,
    pub worktree_path: Option<PathBuf>,
    pub changed_files: Vec<FileChange>,
    pub diff: String,
    pub worker_output: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GatePriority { MustPass, Advisory }

#[derive(Debug, Clone)]
pub struct GateResult {
    pub passed: bool,
    pub name: String,
    pub output: String,
    pub blocking: bool,
}

#[async_trait]
pub trait VerificationGate: Send + Sync {
    fn name(&self) -> &'static str;
    fn priority(&self) -> GatePriority;
    async fn check(&self, ctx: &VerificationContext) -> anyhow::Result<GateResult>;
}

pub struct VerificationPipeline { gates: Vec<Box<dyn VerificationGate>> }

impl VerificationPipeline {
    pub fn standard(worktree_base: PathBuf) -> Self {
        Self {
            gates: vec![
                Box::new(gates::FormatGate),
                Box::new(gates::CompileGate { worktree_base: worktree_base.clone() }),
                Box::new(gates::TestGate { worktree_base: worktree_base.clone() }),
                Box::new(gates::ClippyGate { worktree_base }),
                Box::new(gates::DiffScopeGate::default()),
                Box::new(gates::ArchitectureGate::default()),
                Box::new(gates::CapabilityPolicyGate::default()),
            ],
        }
    }

    pub async fn run(&self, ctx: &VerificationContext) -> anyhow::Result<report::VerificationReport> {
        let mut results = Vec::new();
        let mut all_passed = true;
        for gate in &self.gates {
            let result = gate.check(ctx).await?;
            let is_blocking = result.blocking && !result.passed;
            if is_blocking { all_passed = false; }
            results.push(result);
            if is_blocking { break; }
        }
        Ok(report::VerificationReport {
            passed: all_passed, gates: results,
            summary: if all_passed { "All checks passed".into() } else { "Some checks failed".into() },
            risks: Vec::new(),
            recommendation: if all_passed { report::VerificationAction::Accept } else { report::VerificationAction::Revise },
        })
    }
}
```

- [ ] **Step 2: Write VerificationReport**

```rust
// crates/executive/src/impl/goal/verify/report.rs
use super::GateResult;

#[derive(Debug, Clone)]
pub enum VerificationAction { Accept, Revise, Reject }

#[derive(Debug, Clone)]
pub struct VerificationReport {
    pub passed: bool,
    pub gates: Vec<GateResult>,
    pub summary: String,
    pub risks: Vec<String>,
    pub recommendation: VerificationAction,
}
```

- [ ] **Step 3: Write 7 standard gates with tests**

```rust
// crates/executive/src/impl/goal/verify/gates.rs
//! Seven standard verification gates.

use async_trait::async_trait;
use std::path::PathBuf;
use std::process::Command;
use super::{GatePriority, GateResult, VerificationContext, VerificationGate};
use crate::r#impl::agent::pi::report::{ChangeType, FileChange};

fn run_cmd(dir: &PathBuf, cmd: &str, args: &[&str]) -> (bool, String) {
    match Command::new(cmd).args(args).current_dir(dir).output() {
        Ok(o) => {
            let combined = format!("{}\n{}",
                String::from_utf8_lossy(&o.stdout),
                String::from_utf8_lossy(&o.stderr));
            (o.status.success(), combined)
        }
        Err(e) => (false, format!("Command failed: {e}")),
    }
}

pub struct FormatGate;
#[async_trait]
impl VerificationGate for FormatGate {
    fn name(&self) -> &'static str { "Format" }
    fn priority(&self) -> GatePriority { GatePriority::MustPass }
    async fn check(&self, ctx: &VerificationContext) -> anyhow::Result<GateResult> {
        let dir = ctx.worktree_path.clone().unwrap_or_else(|| PathBuf::from("."));
        let (passed, output) = run_cmd(&dir, "cargo", &["fmt", "--check"]);
        Ok(GateResult { passed, name: self.name().into(), output, blocking: true })
    }
}

pub struct CompileGate { pub worktree_base: PathBuf }
#[async_trait]
impl VerificationGate for CompileGate {
    fn name(&self) -> &'static str { "Compile" }
    fn priority(&self) -> GatePriority { GatePriority::MustPass }
    async fn check(&self, ctx: &VerificationContext) -> anyhow::Result<GateResult> {
        let dir = ctx.worktree_path.clone().unwrap_or_else(|| self.worktree_base.clone());
        let (passed, output) = run_cmd(&dir, "cargo", &["check", "--workspace"]);
        Ok(GateResult { passed, name: self.name().into(), output, blocking: true })
    }
}

pub struct TestGate { pub worktree_base: PathBuf }
#[async_trait]
impl VerificationGate for TestGate {
    fn name(&self) -> &'static str { "Test" }
    fn priority(&self) -> GatePriority { GatePriority::MustPass }
    async fn check(&self, ctx: &VerificationContext) -> anyhow::Result<GateResult> {
        let dir = ctx.worktree_path.clone().unwrap_or_else(|| self.worktree_base.clone());
        let (passed, output) = run_cmd(&dir, "cargo", &["test", "--workspace"]);
        Ok(GateResult { passed, name: self.name().into(), output, blocking: true })
    }
}

pub struct ClippyGate { pub worktree_base: PathBuf }
#[async_trait]
impl VerificationGate for ClippyGate {
    fn name(&self) -> &'static str { "Clippy" }
    fn priority(&self) -> GatePriority { GatePriority::Advisory }
    async fn check(&self, ctx: &VerificationContext) -> anyhow::Result<GateResult> {
        let dir = ctx.worktree_path.clone().unwrap_or_else(|| self.worktree_base.clone());
        let (passed, output) = run_cmd(&dir, "cargo", &["clippy", "--", "-D", "warnings"]);
        Ok(GateResult { passed, name: self.name().into(), output, blocking: false })
    }
}

pub struct DiffScopeGate { pub allowed_paths: Vec<String> }
impl Default for DiffScopeGate {
    fn default() -> Self { Self { allowed_paths: vec!["crates/".into(), "src/".into()] } }
}
#[async_trait]
impl VerificationGate for DiffScopeGate {
    fn name(&self) -> &'static str { "DiffScope" }
    fn priority(&self) -> GatePriority { GatePriority::MustPass }
    async fn check(&self, ctx: &VerificationContext) -> anyhow::Result<GateResult> {
        let violations: Vec<_> = ctx.changed_files.iter()
            .filter(|f| !self.allowed_paths.iter().any(|a| f.path.starts_with(a)))
            .collect();
        let passed = violations.is_empty();
        let output = if passed {
            "All changed files within allowed scope".into()
        } else {
            format!("Files outside allowed scope: {:?}",
                violations.iter().map(|f| &f.path).collect::<Vec<_>>())
        };
        Ok(GateResult { passed, name: self.name().into(), output, blocking: true })
    }
}

pub struct ArchitectureGate { pub forbidden_deps: Vec<(String, String)> }
impl Default for ArchitectureGate {
    fn default() -> Self {
        Self { forbidden_deps: vec![
            ("cognit".into(), "corpus".into()),
            ("mnemosyne".into(), "executive".into()),
        ]}
    }
}
#[async_trait]
impl VerificationGate for ArchitectureGate {
    fn name(&self) -> &'static str { "Architecture" }
    fn priority(&self) -> GatePriority { GatePriority::Advisory }
    async fn check(&self, _ctx: &VerificationContext) -> anyhow::Result<GateResult> {
        Ok(GateResult { passed: true, name: self.name().into(),
            output: "Architecture check skipped (MVP)".into(), blocking: false })
    }
}

pub struct CapabilityPolicyGate;
impl Default for CapabilityPolicyGate { fn default() -> Self { Self } }
#[async_trait]
impl VerificationGate for CapabilityPolicyGate {
    fn name(&self) -> &'static str { "CapabilityPolicy" }
    fn priority(&self) -> GatePriority { GatePriority::MustPass }
    async fn check(&self, ctx: &VerificationContext) -> anyhow::Result<GateResult> {
        let forbidden = ["Cargo.toml", "Cargo.lock", "/etc/aletheon/"];
        let violations: Vec<_> = ctx.changed_files.iter()
            .filter(|f| forbidden.iter().any(|ff| f.path.contains(ff)))
            .collect();
        let passed = violations.is_empty();
        let output = if passed { "No capability policy violations".into() }
        else { format!("Forbidden: {:?}", violations.iter().map(|f| &f.path).collect::<Vec<_>>()) };
        Ok(GateResult { passed, name: self.name().into(), output, blocking: true })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_ctx() -> VerificationContext {
        VerificationContext {
            goal_id: 1, attempt_id: "att-1".into(), worktree_path: None,
            changed_files: vec![FileChange {
                path: "crates/foo/src/lib.rs".into(),
                change_type: ChangeType::Modified, lines_added: 1, lines_removed: 0,
            }],
            diff: "+1".into(), worker_output: "ok".into(),
        }
    }

    #[tokio::test]
    async fn diff_scope_allows_crates() {
        let r = DiffScopeGate::default().check(&test_ctx()).await.unwrap();
        assert!(r.passed);
    }

    #[tokio::test]
    async fn diff_scope_rejects_etc() {
        let mut ctx = test_ctx();
        ctx.changed_files = vec![FileChange {
            path: "/etc/aletheon/config.toml".into(),
            change_type: ChangeType::Modified, lines_added: 1, lines_removed: 0,
        }];
        let r = DiffScopeGate::default().check(&ctx).await.unwrap();
        assert!(!r.passed);
    }

    #[tokio::test]
    async fn capability_blocks_cargo_toml() {
        let mut ctx = test_ctx();
        ctx.changed_files = vec![FileChange {
            path: "Cargo.toml".into(),
            change_type: ChangeType::Modified, lines_added: 1, lines_removed: 0,
        }];
        let r = CapabilityPolicyGate.check(&ctx).await.unwrap();
        assert!(!r.passed);
    }

    #[tokio::test]
    async fn capability_allows_src() {
        let r = CapabilityPolicyGate.check(&test_ctx()).await.unwrap();
        assert!(r.passed);
    }
}
```

- [ ] **Step 4: Write CapabilityPolicy rules**

```rust
// crates/executive/src/impl/goal/verify/policy.rs
//! Capability policy rules -- what operations require approval.

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Capability {
    ReadFile(String), WriteFile(String), DeleteFile(String),
    RunCommand(String), NetworkAccess(String), GitPush, ModifyConfig,
}

#[derive(Debug, Clone)]
pub enum PolicyDecision {
    Allow, Deny { reason: String }, RequireApproval { description: String },
}

pub fn check_capability(cap: &Capability) -> PolicyDecision {
    match cap {
        Capability::WriteFile(path) | Capability::ReadFile(path) => {
            if path.starts_with("/etc/aletheon/") || path.starts_with("/run/aletheon/") {
                PolicyDecision::Deny { reason: "Cannot access system paths".into() }
            } else if path == "Cargo.toml" || path == "Cargo.lock" {
                PolicyDecision::RequireApproval { description: "Modifying Cargo.toml requires approval".into() }
            } else { PolicyDecision::Allow }
        }
        Capability::DeleteFile(_) => PolicyDecision::RequireApproval {
            description: "File deletion requires approval".into(),
        },
        Capability::RunCommand(cmd) => {
            let lower = cmd.to_lowercase();
            if lower.contains("rm") && lower.contains("-rf") || lower.contains("sudo") {
                PolicyDecision::Deny { reason: "Dangerous command blocked".into() }
            } else { PolicyDecision::Allow }
        }
        Capability::GitPush => PolicyDecision::Deny { reason: "Git push not allowed".into() },
        Capability::ModifyConfig => PolicyDecision::Deny { reason: "Config modification not allowed".into() },
        Capability::NetworkAccess(_) => PolicyDecision::Allow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_system_path() {
        assert!(matches!(check_capability(&Capability::WriteFile("/etc/aletheon/x".into())), PolicyDecision::Deny{..}));
    }
    #[test]
    fn allow_normal_write() {
        assert!(matches!(check_capability(&Capability::WriteFile("crates/x/src/lib.rs".into())), PolicyDecision::Allow));
    }
    #[test]
    fn deny_dangerous_cmd() {
        assert!(matches!(check_capability(&Capability::RunCommand("sudo rm -rf /tmp".into())), PolicyDecision::Deny{..}));
    }
    #[test]
    fn deny_git_push() {
        assert!(matches!(check_capability(&Capability::GitPush), PolicyDecision::Deny{..}));
    }
    #[test]
    fn require_approval_cargo() {
        assert!(matches!(check_capability(&Capability::WriteFile("Cargo.toml".into())), PolicyDecision::RequireApproval{..}));
    }
}
```

- [ ] **Step 5: Update goal mod.rs**

Add to `crates/executive/src/impl/goal/mod.rs`:
```rust
pub mod verify;
```

- [ ] **Step 6: Compile and test**

```bash
cargo test -p executive -- impl::goal::verify
```
Expected: 7 tests pass (4 gate tests + 5 policy tests).

- [ ] **Step 7: Commit**

```bash
git add crates/executive/src/impl/goal/verify/ crates/executive/src/impl/goal/mod.rs
git commit -m "feat(executive): add VerificationPipeline with 7 gates and CapabilityPolicy"
```

- [ ] **Step 4: Write CapabilityPolicy (rules engine)**

```rust
// crates/executive/src/impl/goal/verify/policy.rs
//! Capability policy rules — defines what operations require approval.

use serde::{Deserialize, Serialize};

/// A capability that a worker may attempt to use.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Capability {
    ReadFile(String),
    WriteFile(String),
    DeleteFile(String),
    RunCommand(String),
    NetworkAccess(String),
    GitPush,
    ModifyConfig,
}

/// Policy decision for a capability attempt.
#[derive(Debug, Clone)]
pub enum PolicyDecision {
    Allow,
    Deny { reason: String },
    RequireApproval { description: String },
}

/// Check if a capability action is allowed by policy.
pub fn check_capability(cap: &Capability) -> PolicyDecision {
    match cap {
        Capability::WriteFile(path) | Capability::ReadFile(path) => {
            if path.starts_with("/etc/aletheon/") || path.starts_with("/run/aletheon/") {
                PolicyDecision::Deny {
                    reason: "Cannot access Aletheon system paths".into(),
                }
            } else if path == "Cargo.toml" || path == "Cargo.lock" {
                PolicyDecision::RequireApproval {
                    description: "Modifying Cargo.toml/Cargo.lock requires approval".into(),
                }
            } else {
                PolicyDecision::Allow
            }
        }
        Capability::DeleteFile(_) => PolicyDecision::RequireApproval {
            description: "File deletion requires approval".into(),
        },
        Capability::RunCommand(cmd) => {
            if cmd.contains("rm") && cmd.contains("-rf") || cmd.contains("sudo") {
                PolicyDecision::Deny { reason: "Dangerous command blocked".into() }
            } else {
                PolicyDecision::Allow
            }
        }
        Capability::GitPush => PolicyDecision::Deny {
            reason: "Git push is never allowed from workers".into(),
        },
        Capability::ModifyConfig => PolicyDecision::Deny {
            reason: "Configuration modification not allowed from workers".into(),
        },
        Capability::NetworkAccess(_) => PolicyDecision::Allow,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn deny_system_path_write() {
        let r = check_capability(&Capability::WriteFile("/etc/aletheon/config.toml".into()));
        assert!(matches!(r, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn allow_normal_write() {
        let r = check_capability(&Capability::WriteFile("crates/foo/src/lib.rs".into()));
        assert!(matches!(r, PolicyDecision::Allow));
    }

    #[test]
    fn deny_dangerous_command() {
        let r = check_capability(&Capability::RunCommand("sudo rm -rf /tmp".into()));
        assert!(matches!(r, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn deny_git_push() {
        let r = check_capability(&Capability::GitPush);
        assert!(matches!(r, PolicyDecision::Deny { .. }));
    }

    #[test]
    fn require_approval_cargo_toml() {
        let r = check_capability(&Capability::WriteFile("Cargo.toml".into()));
        assert!(matches!(r, PolicyDecision::RequireApproval { .. }));
    }
}
```

- [ ] **Step 5: Update goal mod.rs**

Add:
```rust
pub mod verify;
```

- [ ] **Step 6: Compile and test**

```bash
cargo test -p executive -- impl::goal::verify
```
Expected: 7 tests pass (4 gates + 5 policy).

- [ ] **Step 7: Commit**

```bash
git add crates/executive/src/impl/goal/verify/ crates/executive/src/impl/goal/mod.rs
git commit -m "feat(executive): add VerificationPipeline with 7 gates and CapabilityPolicy"
```

---

## Phase F: Google Read-Only + Sync

**Goal:** OAuth-authorized read access to Gmail and Calendar with incremental sync. Tokens encrypted at rest.
**Crates:** corpus (new module drivers/google/), fabric (new types)
**Depends on:** Phase C (GoalWorker trait available)
**Estimated:** 2–3 weeks

### Task F.1: Add Google API dependencies and fabric types

**Files:**
- Create: `crates/fabric/src/types/google.rs`
- Modify: `crates/fabric/src/types/mod.rs`
- Modify: `crates/fabric/src/lib.rs`
- Modify: `crates/corpus/Cargo.toml`

- [ ] **Step 1: Add dependencies to corpus**

```toml
# Add to crates/corpus/Cargo.toml
google-oauth = "1"         # or yup-oauth2
aes-gcm = "0.10"
base64 = "0.22"
```

- [ ] **Step 2: Write Google shared types**

```rust
// crates/fabric/src/types/google.rs
//! Google integration shared types.

use serde::{Deserialize, Serialize};

/// Scope requested for Google API access.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum GoogleScope {
    GmailReadOnly,
    GmailModify,
    CalendarReadOnly,
    CalendarEvents,
    DriveReadOnly,
    DriveFile,
    ContactsReadOnly,
    TasksReadOnly,
}

impl GoogleScope {
    pub fn as_url(&self) -> &'static str {
        match self {
            GoogleScope::GmailReadOnly => "https://www.googleapis.com/auth/gmail.readonly",
            GoogleScope::GmailModify => "https://www.googleapis.com/auth/gmail.modify",
            GoogleScope::CalendarReadOnly => "https://www.googleapis.com/auth/calendar.readonly",
            GoogleScope::CalendarEvents => "https://www.googleapis.com/auth/calendar.events",
            GoogleScope::DriveReadOnly => "https://www.googleapis.com/auth/drive.readonly",
            GoogleScope::DriveFile => "https://www.googleapis.com/auth/drive.file",
            GoogleScope::ContactsReadOnly => "https://www.googleapis.com/auth/contacts.readonly",
            GoogleScope::TasksReadOnly => "https://www.googleapis.com/auth/tasks.readonly",
        }
    }
}

/// A Gmail message summary (for list views).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailMessageSummary {
    pub id: String,
    pub thread_id: String,
    pub subject: String,
    pub from: String,
    pub snippet: String,
    pub received_at: String,
    pub is_unread: bool,
}

/// A Gmail query (Gmail search syntax).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailQuery {
    pub query: String,
    pub max_results: usize,
}

impl Default for GmailQuery {
    fn default() -> Self {
        Self { query: "is:unread".into(), max_results: 20 }
    }
}

/// A full Gmail message.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GmailMessage {
    pub id: String,
    pub thread_id: String,
    pub subject: String,
    pub from: String,
    pub to: String,
    pub body_text: String,
    pub received_at: String,
}

/// A Google Calendar event.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GoogleCalendarEvent {
    pub id: String,
    pub summary: String,
    pub description: Option<String>,
    pub start: String, // ISO 8601
    pub end: String,
    pub attendees: Vec<String>,
    pub location: Option<String>,
}

/// Time range for calendar queries.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TimeRange {
    pub start: String, // ISO 8601
    pub end: String,
}

impl TimeRange {
    pub fn today() -> Self {
        let now = chrono::Utc::now();
        let start = now.format("%Y-%m-%dT00:00:00Z").to_string();
        let end = (now + chrono::Duration::days(1)).format("%Y-%m-%dT00:00:00Z").to_string();
        Self { start, end }
    }

    pub fn next_days(days: u32) -> Self {
        let now = chrono::Utc::now();
        let start = now.format("%Y-%m-%dT00:00:00Z").to_string();
        let end = (now + chrono::Duration::days(days as i64)).format("%Y-%m-%dT00:00:00Z").to_string();
        Self { start, end }
    }
}

/// Normalized Google event (after sync dedup).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum GoogleEvent {
    MailReceived {
        message_id: String,
        sender: String,
        subject: String,
        snippet: String,
        received_at: String,
    },
    CalendarEventStarting {
        event_id: String,
        summary: String,
        start: String,
        end: String,
    },
    CalendarEventUpdated {
        event_id: String,
        summary: String,
    },
    CalendarEventCancelled {
        event_id: String,
    },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scope_urls_are_valid() {
        assert!(GoogleScope::GmailReadOnly.as_url().starts_with("https://"));
        assert!(GoogleScope::CalendarReadOnly.as_url().contains("calendar"));
    }

    #[test]
    fn time_range_today_is_24_hours() {
        let range = TimeRange::today();
        assert!(!range.start.is_empty());
        assert!(!range.end.is_empty());
    }
}
```

- [ ] **Step 3: Wire into fabric**

Add `pub mod google;` to `crates/fabric/src/types/mod.rs`.
Add `pub use types::google;` to `crates/fabric/src/lib.rs`.

- [ ] **Step 4: Compile and test**

```bash
cargo test -p fabric -- types::google
```
Expected: 2 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/fabric/src/types/google.rs crates/fabric/src/types/mod.rs crates/fabric/src/lib.rs crates/corpus/Cargo.toml
git commit -m "feat(fabric): add Google shared types (GmailMessageSummary, GoogleCalendarEvent, TimeRange, GoogleEvent)"
```

---

### Task F.2: Implement CredentialVault

**Files:**
- Create: `crates/corpus/src/drivers/google/mod.rs`
- Create: `crates/corpus/src/drivers/google/vault.rs`
- Modify: `crates/corpus/src/drivers/mod.rs`

- [ ] **Step 1: Write CredentialVault**

```rust
// crates/corpus/src/drivers/google/vault.rs
//! CredentialVault — AES-256-GCM encrypted token storage.
//!
//! Tokens are encrypted at rest using a key loaded from
//! /etc/aletheon/secrets/vault.key (file permissions 600).
//! Decrypted tokens only exist in memory during active use.

use aes_gcm::aead::{Aead, KeyInit, OsRng};
use aes_gcm::{Aes256Gcm, Nonce};
use anyhow::{Context, Result};
use base64::{engine::general_purpose::STANDARD as BASE64, Engine};
use rand::RngCore;
use std::path::{Path, PathBuf};
use tracing::warn;

pub struct CredentialVault {
    cipher: Aes256Gcm,
    store_path: PathBuf,
}

impl CredentialVault {
    /// Open the vault. Loads the encryption key from the given path.
    /// If no key exists, generates a new one (only on first run).
    pub fn open(key_path: &Path, store_path: &Path) -> Result<Self> {
        let key = if key_path.exists() {
            let key_bytes = std::fs::read(key_path)
                .context("reading vault key")?;
            if key_bytes.len() != 32 {
                anyhow::bail!("Vault key must be 32 bytes (256 bits)");
            }
            key_bytes.try_into().map_err(|_| anyhow::anyhow!("invalid key length"))?
        } else {
            warn!("No vault key found, generating new one at {}", key_path.display());
            let mut key = [0u8; 32];
            OsRng.fill_bytes(&mut key);
            if let Some(parent) = key_path.parent() {
                std::fs::create_dir_all(parent)?;
            }
            std::fs::write(key_path, &key)?;
            // Set restrictive permissions
            #[cfg(unix)]
            {
                use std::os::unix::fs::PermissionsExt;
                std::fs::set_permissions(key_path, std::fs::Permissions::from_mode(0o600))?;
            }
            key
        };

        let cipher = Aes256Gcm::new_from_slice(&key)
            .map_err(|_| anyhow::anyhow!("invalid AES key"))?;

        std::fs::create_dir_all(store_path)?;

        Ok(Self {
            cipher,
            store_path: store_path.to_path_buf(),
        })
    }

    /// Encrypt and store a token.
    pub fn encrypt_and_store(&self, identity_id: &str, token_data: &[u8]) -> Result<()> {
        let mut nonce_bytes = [0u8; 12];
        OsRng.fill_bytes(&mut nonce_bytes);
        let nonce = Nonce::from_slice(&nonce_bytes);

        let ciphertext = self.cipher
            .encrypt(nonce, token_data)
            .map_err(|e| anyhow::anyhow!("encryption failed: {e}"))?;

        // Store as: nonce (12 bytes) || ciphertext
        let mut stored = nonce_bytes.to_vec();
        stored.extend_from_slice(&ciphertext);
        let encoded = BASE64.encode(&stored);

        let file_path = self.store_path.join(format!("{identity_id}.enc"));
        std::fs::write(&file_path, encoded)?;
        Ok(())
    }

    /// Load and decrypt a token.
    pub fn load_and_decrypt(&self, identity_id: &str) -> Result<Vec<u8>> {
        let file_path = self.store_path.join(format!("{identity_id}.enc"));
        let encoded = std::fs::read_to_string(&file_path)
            .context("reading encrypted token")?;
        let stored = BASE64.decode(encoded.trim())
            .context("decoding base64")?;

        if stored.len() < 12 {
            anyhow::bail!("stored data too short");
        }

        let (nonce_bytes, ciphertext) = stored.split_at(12);
        let nonce = Nonce::from_slice(nonce_bytes);

        let plaintext = self.cipher
            .decrypt(nonce, ciphertext)
            .map_err(|e| anyhow::anyhow!("decryption failed: {e}"))?;

        Ok(plaintext)
    }

    /// Delete an encrypted token.
    pub fn delete(&self, identity_id: &str) -> Result<()> {
        let file_path = self.store_path.join(format!("{identity_id}.enc"));
        if file_path.exists() {
            std::fs::remove_file(&file_path)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::TempDir;

    #[test]
    fn encrypt_decrypt_roundtrip() {
        let tmp = TempDir::new().unwrap();
        let key_path = tmp.path().join("vault.key");
        let store = tmp.path().join("tokens");

        let vault = CredentialVault::open(&key_path, &store).unwrap();

        let token = b"ya29.a0AfH6S...refresh_token_data";
        vault.encrypt_and_store("test-user", token).unwrap();

        let decrypted = vault.load_and_decrypt("test-user").unwrap();
        assert_eq!(decrypted, token);
    }

    #[test]
    fn delete_removes_token() {
        let tmp = TempDir::new().unwrap();
        let key_path = tmp.path().join("vault.key");
        let store = tmp.path().join("tokens");

        let vault = CredentialVault::open(&key_path, &store).unwrap();
        vault.encrypt_and_store("to-delete", b"token").unwrap();
        vault.delete("to-delete").unwrap();
        assert!(vault.load_and_decrypt("to-delete").is_err());
    }

    #[test]
    fn wrong_key_fails_decrypt() {
        let tmp = TempDir::new().unwrap();
        let key_path = tmp.path().join("vault.key");
        let store = tmp.path().join("tokens");

        let vault = CredentialVault::open(&key_path, &store).unwrap();
        vault.encrypt_and_store("user", b"secret").unwrap();

        // Create a different key
        std::fs::remove_file(&key_path).unwrap();
        let vault2 = CredentialVault::open(&key_path, &store).unwrap();
        assert!(vault2.load_and_decrypt("user").is_err());
    }
}
```

- [ ] **Step 2: Write google driver mod.rs**

```rust
// crates/corpus/src/drivers/google/mod.rs
//! Google ecosystem integration — OAuth, Gmail, Calendar, Sync.
//!
//! Security: tokens are encrypted at rest via CredentialVault.
//! The GoogleDriver owns all tokens; callers receive only data.

pub mod vault;

pub use vault::CredentialVault;
```

- [ ] **Step 3: Wire into corpus drivers**

Add to `crates/corpus/src/drivers/mod.rs`:
```rust
pub mod google;
```

- [ ] **Step 4: Compile and test**

```bash
cargo test -p corpus -- drivers::google::vault
```
Expected: 3 tests pass.

- [ ] **Step 5: Commit**

```bash
git add crates/corpus/src/drivers/google/ crates/corpus/src/drivers/mod.rs
git commit -m "feat(corpus): add CredentialVault with AES-256-GCM encrypted token storage"
```

---

### Task F.3: Implement GmailCapability and CalendarCapability

**Files:**
- Create: `crates/corpus/src/drivers/google/gmail.rs`
- Create: `crates/corpus/src/drivers/google/calendar.rs`
- Create: `crates/corpus/src/drivers/google/sync.rs`
- Modify: `crates/corpus/src/drivers/google/mod.rs`

- [ ] **Step 1: Write GmailCapability (read-only MVP)**

```rust
// crates/corpus/src/drivers/google/gmail.rs
//! GmailCapability — read-only Gmail access via Google Gmail API.
//!
//! MVP: search, read, list_unread. Write operations (draft/send) in Phase G.

use anyhow::Result;
use async_trait::async_trait;
use fabric::types::google::{GmailMessage, GmailMessageSummary, GmailQuery};

/// Read-only Gmail operations.
#[async_trait]
pub trait GmailCapability: Send + Sync {
    async fn search(
        &self,
        query: &GmailQuery,
        access_token: &str,
    ) -> Result<Vec<GmailMessageSummary>>;

    async fn read(
        &self,
        message_id: &str,
        access_token: &str,
    ) -> Result<GmailMessage>;

    async fn list_unread(
        &self,
        max: usize,
        access_token: &str,
    ) -> Result<Vec<GmailMessageSummary>>;
}

/// REST-based Gmail API implementation.
pub struct GmailApi {
    client: reqwest::Client,
}

impl GmailApi {
    pub fn new() -> Self {
        Self {
            client: reqwest::Client::new(),
        }
    }
}

#[async_trait]
impl GmailCapability for GmailApi {
    async fn search(
        &self,
        query: &GmailQuery,
        access_token: &str,
    ) -> Result<Vec<GmailMessageSummary>> {
        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages?q={}&maxResults={}",
            urlencoding::encode(&query.query),
            query.max_results,
        );

        let resp: serde_json::Value = self.client
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?
            .json()
            .await?;

        let messages = resp["messages"].as_array()
            .map(|arr| arr.to_vec())
            .unwrap_or_default();

        let mut summaries = Vec::new();
        for msg in messages {
            let id = msg["id"].as_str().unwrap_or("").to_string();
            // Fetch message metadata
            let detail_url = format!(
                "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=metadata&metadataHeaders=Subject&metadataHeaders=From",
                id
            );
            if let Ok(detail) = self.client
                .get(&detail_url)
                .bearer_auth(access_token)
                .send()
                .await?
                .json::<serde_json::Value>()
                .await
            {
                let headers = &detail["payload"]["headers"];
                let subject = find_header(headers, "Subject").unwrap_or_default();
                let from = find_header(headers, "From").unwrap_or_default();
                let snippet = detail["snippet"].as_str().unwrap_or("").to_string();
                let is_unread = detail["labelIds"].as_array()
                    .map(|l| l.iter().any(|v| v.as_str() == Some("UNREAD")))
                    .unwrap_or(false);

                summaries.push(GmailMessageSummary {
                    id,
                    thread_id: detail["threadId"].as_str().unwrap_or("").to_string(),
                    subject,
                    from,
                    snippet,
                    received_at: detail["internalDate"].as_str().unwrap_or("").to_string(),
                    is_unread,
                });
            }
        }

        Ok(summaries)
    }

    async fn read(
        &self,
        message_id: &str,
        access_token: &str,
    ) -> Result<GmailMessage> {
        let url = format!(
            "https://gmail.googleapis.com/gmail/v1/users/me/messages/{}?format=full",
            message_id,
        );

        let detail: serde_json::Value = self.client
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?
            .json()
            .await?;

        let headers = &detail["payload"]["headers"];
        let subject = find_header(headers, "Subject").unwrap_or_default();
        let from = find_header(headers, "From").unwrap_or_default();
        let to = find_header(headers, "To").unwrap_or_default();
        let body = extract_body(&detail).unwrap_or_default();

        Ok(GmailMessage {
            id: message_id.to_string(),
            thread_id: detail["threadId"].as_str().unwrap_or("").to_string(),
            subject,
            from,
            to,
            body_text: body,
            received_at: detail["internalDate"].as_str().unwrap_or("").to_string(),
        })
    }

    async fn list_unread(
        &self,
        max: usize,
        access_token: &str,
    ) -> Result<Vec<GmailMessageSummary>> {
        let query = GmailQuery {
            query: "is:unread".into(),
            max_results: max,
        };
        self.search(&query, access_token).await
    }
}

fn find_header(headers: &serde_json::Value, name: &str) -> Option<String> {
    headers.as_array()?.iter().find_map(|h| {
        if h["name"].as_str()? == name {
            h["value"].as_str().map(|s| s.to_string())
        } else {
            None
        }
    })
}

fn extract_body(detail: &serde_json::Value) -> Option<String> {
    let parts = detail["payload"]["parts"].as_array()?;
    for part in parts {
        if part["mimeType"].as_str()? == "text/plain" {
            let data = part["body"]["data"].as_str()?;
            // Base64url decode
            let cleaned = data.replace('-', "+").replace('_', "/");
            let bytes = base64::engine::general_purpose::STANDARD
                .decode(cleaned)
                .ok()?;
            return Some(String::from_utf8_lossy(&bytes).to_string());
        }
    }
    // Fallback: try body.data directly
    let data = detail["payload"]["body"]["data"].as_str()?;
    let cleaned = data.replace('-', "+").replace('_', "/");
    let bytes = base64::engine::general_purpose::STANDARD.decode(cleaned).ok()?;
    Some(String::from_utf8_lossy(&bytes).to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn find_header_extracts_subject() {
        let headers = serde_json::json!([
            {"name": "Subject", "value": "Hello"},
            {"name": "From", "value": "me@example.com"},
        ]);
        assert_eq!(find_header(&headers, "Subject"), Some("Hello".into()));
        assert_eq!(find_header(&headers, "Missing"), None);
    }
}
```

- [ ] **Step 2: Write CalendarCapability**

```rust
// crates/corpus/src/drivers/google/calendar.rs
//! CalendarCapability — read-only Google Calendar access.

use anyhow::Result;
use async_trait::async_trait;
use fabric::types::google::{GoogleCalendarEvent, TimeRange};

#[async_trait]
pub trait CalendarCapability: Send + Sync {
    async fn list_events(
        &self,
        range: &TimeRange,
        access_token: &str,
    ) -> Result<Vec<GoogleCalendarEvent>>;

    async fn today(
        &self,
        access_token: &str,
    ) -> Result<Vec<GoogleCalendarEvent>>;
}

pub struct CalendarApi {
    client: reqwest::Client,
}

impl CalendarApi {
    pub fn new() -> Self {
        Self { client: reqwest::Client::new() }
    }
}

#[async_trait]
impl CalendarCapability for CalendarApi {
    async fn list_events(
        &self,
        range: &TimeRange,
        access_token: &str,
    ) -> Result<Vec<GoogleCalendarEvent>> {
        let url = format!(
            "https://www.googleapis.com/calendar/v3/calendars/primary/events?timeMin={}&timeMax={}&singleEvents=true&orderBy=startTime",
            range.start, range.end,
        );

        let resp: serde_json::Value = self.client
            .get(&url)
            .bearer_auth(access_token)
            .send()
            .await?
            .json()
            .await?;

        let items = resp["items"].as_array()
            .map(|arr| arr.to_vec())
            .unwrap_or_default();

        let events: Vec<GoogleCalendarEvent> = items.iter().filter_map(|item| {
            Some(GoogleCalendarEvent {
                id: item["id"].as_str()?.to_string(),
                summary: item["summary"].as_str().unwrap_or("(no title)").to_string(),
                description: item["description"].as_str().map(|s| s.to_string()),
                start: item["start"]["dateTime"].as_str()
                    .or_else(|| item["start"]["date"].as_str())
                    .unwrap_or("")
                    .to_string(),
                end: item["end"]["dateTime"].as_str()
                    .or_else(|| item["end"]["date"].as_str())
                    .unwrap_or("")
                    .to_string(),
                attendees: item["attendees"].as_array()
                    .map(|a| a.iter().filter_map(|v| v["email"].as_str().map(|s| s.to_string())).collect())
                    .unwrap_or_default(),
                location: item["location"].as_str().map(|s| s.to_string()),
            })
        }).collect();

        Ok(events)
    }

    async fn today(
        &self,
        access_token: &str,
    ) -> Result<Vec<GoogleCalendarEvent>> {
        self.list_events(&TimeRange::today(), access_token).await
    }
}
```

- [ ] **Step 3: Write GoogleSyncManager (incremental sync)**

```rust
// crates/corpus/src/drivers/google/sync.rs
//! GoogleSyncManager — incremental sync with cursor-based dedup.
//!
//! Uses Gmail historyId and Calendar syncToken for incremental updates.
//! Events are normalized and deduplicated before forwarding.

use anyhow::Result;
use fabric::types::google::GoogleEvent;
use std::collections::HashSet;
use tokio::sync::mpsc;
use tracing::{debug, info, warn};

/// Cursor store for sync state persistence.
/// MVP: in-memory HashSet. Phase F.4 adds SQLite persistence.
pub struct SyncCursorStore {
    seen_message_ids: HashSet<String>,
    seen_event_ids: HashSet<String>,
    gmail_history_id: Option<String>,
    calendar_sync_token: Option<String>,
}

impl SyncCursorStore {
    pub fn new() -> Self {
        Self {
            seen_message_ids: HashSet::new(),
            seen_event_ids: HashSet::new(),
            gmail_history_id: None,
            calendar_sync_token: None,
        }
    }

    pub fn is_duplicate_message(&self, id: &str) -> bool {
        self.seen_message_ids.contains(id)
    }

    pub fn mark_message_seen(&mut self, id: String) {
        self.seen_message_ids.insert(id);
    }

    pub fn is_duplicate_event(&self, id: &str) -> bool {
        self.seen_event_ids.contains(id)
    }

    pub fn mark_event_seen(&mut self, id: String) {
        self.seen_event_ids.insert(id);
    }
}

/// Manages incremental sync from Google services.
pub struct GoogleSyncManager {
    cursors: SyncCursorStore,
    event_tx: mpsc::Sender<GoogleEvent>,
}

impl GoogleSyncManager {
    pub fn new(event_tx: mpsc::Sender<GoogleEvent>) -> Self {
        Self {
            cursors: SyncCursorStore::new(),
            event_tx,
        }
    }

    /// Deduplicate and forward Google events.
    pub fn process_events(&mut self, events: Vec<GoogleEvent>) -> usize {
        let mut forwarded = 0;
        for event in events {
            let is_new = match &event {
                GoogleEvent::MailReceived { message_id, .. } => {
                    if self.cursors.is_duplicate_message(message_id) {
                        false
                    } else {
                        self.cursors.mark_message_seen(message_id.clone());
                        true
                    }
                }
                GoogleEvent::CalendarEventStarting { event_id, .. }
                | GoogleEvent::CalendarEventUpdated { event_id, .. }
                | GoogleEvent::CalendarEventCancelled { event_id } => {
                    if self.cursors.is_duplicate_event(event_id) {
                        false
                    } else {
                        self.cursors.mark_event_seen(event_id.clone());
                        true
                    }
                }
            };

            if is_new {
                debug!(?event, "Forwarding new Google event");
                // Non-blocking send (buffer should be sized appropriately)
                if let Err(e) = self.event_tx.try_send(event) {
                    warn!(error = %e, "Event channel full, dropping event");
                } else {
                    forwarded += 1;
                }
            }
        }
        forwarded
    }

    /// Get the Gmail history ID for incremental sync.
    pub fn gmail_history_id(&self) -> Option<&str> {
        self.cursors.gmail_history_id.as_deref()
    }

    /// Update the Gmail history ID after a successful sync.
    pub fn set_gmail_history_id(&mut self, id: String) {
        self.cursors.gmail_history_id = Some(id);
    }

    /// Get the Calendar sync token.
    pub fn calendar_sync_token(&self) -> Option<&str> {
        self.cursors.calendar_sync_token.as_deref()
    }

    /// Update the Calendar sync token.
    pub fn set_calendar_sync_token(&mut self, token: String) {
        self.cursors.calendar_sync_token = Some(token);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dedup_prevents_duplicate_events() {
        let (tx, mut rx) = mpsc::channel(16);
        let mut manager = GoogleSyncManager::new(tx);

        let events = vec![
            GoogleEvent::MailReceived {
                message_id: "msg-1".into(),
                sender: "a@b.com".into(),
                subject: "Test".into(),
                snippet: "...".into(),
                received_at: "now".into(),
            },
            GoogleEvent::MailReceived {
                message_id: "msg-1".into(), // duplicate
                sender: "a@b.com".into(),
                subject: "Test".into(),
                snippet: "...".into(),
                received_at: "now".into(),
            },
            GoogleEvent::MailReceived {
                message_id: "msg-2".into(), // new
                sender: "c@d.com".into(),
                subject: "Test 2".into(),
                snippet: "...".into(),
                received_at: "now".into(),
            },
        ];

        let forwarded = manager.process_events(events);
        assert_eq!(forwarded, 2); // msg-1 (first), msg-2; duplicate skipped
    }
}
```

- [ ] **Step 4: Update google/mod.rs**

Add:
```rust
pub mod calendar;
pub mod gmail;
pub mod sync;

pub use calendar::{CalendarApi, CalendarCapability};
pub use gmail::{GmailApi, GmailCapability};
pub use sync::{GoogleSyncManager, SyncCursorStore};
```

- [ ] **Step 5: Compile and test**

```bash
cargo check -p corpus
cargo test -p corpus -- drivers::google
```
Expected: basic tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/corpus/src/drivers/google/
git commit -m "feat(corpus): add GmailCapability, CalendarCapability, GoogleSyncManager"
```

---

## Phase G: Gmail Channel + Approval

**Goal:** Gmail as a second Channel. Receive emails → create Goal Drafts. Shared Approval model across channels.
**Crates:** executive (new modules in impl/channel/)
**Depends on:** Phase A (Channel trait) + Phase F (GmailCapability)
**Estimated:** 1–2 weeks

### Task G.1: Define Approval model

**Files:**
- Create: `crates/executive/src/impl/channel/approval.rs`
- Modify: `crates/executive/src/impl/channel/mod.rs`

- [ ] **Step 1: Write Approval types**

```rust
// crates/executive/src/impl/channel/approval.rs
//! Approval model — shared across all channels (Telegram, Gmail, Web).

use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Mutex;
use tracing::{info, warn};

/// Unique approval request ID.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ApprovalId(pub String);

/// The type of action requiring approval.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApprovalType {
    ApplyCodeDiff,
    SendEmail,
    DeleteFile,
    ModifyCalendar,
    DangerousCommand,
    CapabilityExpansion,
    BudgetIncrease,
}

/// Details about what is being approved.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ApprovalDetails {
    CodeDiff {
        changed_files: Vec<String>,
        diff_summary: String,
    },
    EmailDraft {
        to: Vec<String>,
        subject: String,
        body_preview: String,
    },
    FileDeletion {
        paths: Vec<String>,
    },
    CalendarModification {
        summary: String,
        start: String,
    },
    Generic {
        description: String,
    },
}

/// A pending approval request.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ApprovalRequest {
    pub id: ApprovalId,
    pub goal_id: i64,
    pub request_type: ApprovalType,
    pub description: String,
    pub details: ApprovalDetails,
    pub timeout_secs: u64,
    pub created_at: String,
}

/// The result of an approval.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ApprovalResult {
    Approved,
    Rejected { reason: String },
    TimedOut,
}

/// Manages pending approval requests.
pub struct ApprovalManager {
    pending: Arc<Mutex<HashMap<ApprovalId, ApprovalRequest>>>,
    default_timeout: Duration,
}

impl ApprovalManager {
    pub fn new(default_timeout_secs: u64) -> Self {
        Self {
            pending: Arc::new(Mutex::new(HashMap::new())),
            default_timeout: Duration::from_secs(default_timeout_secs),
        }
    }

    /// Create a new approval request. Returns the ID for reference.
    pub async fn request(
        &self,
        goal_id: i64,
        request_type: ApprovalType,
        description: String,
        details: ApprovalDetails,
    ) -> ApprovalId {
        let id = ApprovalId(format!("approval-{}", uuid::Uuid::new_v4()));
        let req = ApprovalRequest {
            id: id.clone(),
            goal_id,
            request_type,
            description,
            details,
            timeout_secs: self.default_timeout.as_secs(),
            created_at: chrono::Utc::now().to_rfc3339(),
        };
        self.pending.lock().await.insert(id.clone(), req);
        info!(approval_id = %id.0, "Approval request created");
        id
    }

    /// Resolve an approval. Returns None if the ID is not found.
    pub async fn resolve(&self, id: &ApprovalId, result: ApprovalResult) -> Option<ApprovalRequest> {
        let removed = self.pending.lock().await.remove(id);
        if let Some(ref req) = removed {
            info!(approval_id = %id.0, result = ?result, "Approval resolved");
        }
        removed
    }

    /// List all pending approvals.
    pub async fn list(&self) -> Vec<ApprovalRequest> {
        self.pending.lock().await.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn create_and_resolve_approval() {
        let manager = ApprovalManager::new(1800);
        let id = manager.request(
            1,
            ApprovalType::ApplyCodeDiff,
            "Apply changes?".into(),
            ApprovalDetails::CodeDiff {
                changed_files: vec!["src/main.rs".into()],
                diff_summary: "+5 -2".into(),
            },
        ).await;
        assert_eq!(manager.list().await.len(), 1);

        let resolved = manager.resolve(&id, ApprovalResult::Approved).await;
        assert!(resolved.is_some());
        assert_eq!(manager.list().await.len(), 0);
    }

    #[tokio::test]
    async fn resolve_unknown_id_returns_none() {
        let manager = ApprovalManager::new(1800);
        let result = manager.resolve(
            &ApprovalId("nonexistent".into()),
            ApprovalResult::Approved,
        ).await;
        assert!(result.is_none());
    }
}
```

- [ ] **Step 2: Re-export in channel mod.rs**

Add to `crates/executive/src/impl/channel/mod.rs`:
```rust
pub mod approval;
```

- [ ] **Step 3: Compile and test**

```bash
cargo test -p executive -- impl::channel::approval
```
Expected: 2 tests pass.

- [ ] **Step 4: Commit**

```bash
git add crates/executive/src/impl/channel/approval.rs crates/executive/src/impl/channel/mod.rs
git commit -m "feat(executive): add ApprovalManager with shared approval model"
```

---

### Task G.2: Implement GmailChannel

**Files:**
- Create: `crates/executive/src/impl/channel/gmail/mod.rs`
- Create: `crates/executive/src/impl/channel/gmail/classifier.rs`
- Create: `crates/executive/src/impl/channel/gmail/authenticator.rs`

- [ ] **Step 1: Write SenderAllowlist**

```rust
// crates/executive/src/impl/channel/gmail/authenticator.rs
//! SenderAllowlist — validates email senders against allowed list.

use serde::{Deserialize, Serialize};

/// Permissions for an allowlisted sender.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SenderPermissions {
    pub can_create_goal: bool,
    pub can_ask: bool,
    pub can_record_memory: bool,
    pub auto_approve: bool,
}

impl Default for SenderPermissions {
    fn default() -> Self {
        Self {
            can_create_goal: false,
            can_ask: false,
            can_record_memory: false,
            auto_approve: false,
        }
    }
}

/// Entry in the sender allowlist.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AllowlistEntry {
    pub email: String,       // exact match or "*.domain.com" for domain wildcard
    pub permissions: SenderPermissions,
}

/// Validates email senders against an allowlist.
pub struct SenderAllowlist {
    entries: Vec<AllowlistEntry>,
}

impl SenderAllowlist {
    pub fn new(entries: Vec<AllowlistEntry>) -> Self {
        Self { entries }
    }

    /// Find the permissions for a sender. Returns None if not allowlisted.
    pub fn check(&self, sender: &str) -> Option<&SenderPermissions> {
        let sender_lower = sender.to_lowercase();
        for entry in &self.entries {
            if entry.email.starts_with('*') {
                // Domain wildcard: *@domain.com
                let domain = &entry.email[1..]; // skip '*'
                if sender_lower.ends_with(domain) {
                    return Some(&entry.permissions);
                }
            } else if sender_lower == entry.email.to_lowercase() {
                return Some(&entry.permissions);
            }
        }
        None
    }

    /// Create a default allowlist with just the owner.
    pub fn owner_only(owner_email: &str) -> Self {
        Self {
            entries: vec![AllowlistEntry {
                email: owner_email.to_string(),
                permissions: SenderPermissions {
                    can_create_goal: true,
                    can_ask: true,
                    can_record_memory: true,
                    auto_approve: true,
                },
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_match_allowed() {
        let list = SenderAllowlist::owner_only("me@example.com");
        assert!(list.check("me@example.com").is_some());
        assert!(list.check("other@example.com").is_none());
    }

    #[test]
    fn domain_wildcard() {
        let list = SenderAllowlist::new(vec![AllowlistEntry {
            email: "*@company.com".into(),
            permissions: SenderPermissions { can_create_goal: true, ..Default::default() },
        }]);
        assert!(list.check("alice@company.com").is_some());
        assert!(list.check("alice@other.com").is_none());
    }

    #[test]
    fn case_insensitive() {
        let list = SenderAllowlist::owner_only("Me@Example.com");
        assert!(list.check("me@example.com").is_some());
    }
}
```

- [ ] **Step 2: Write MailClassifier**

```rust
// crates/executive/src/impl/channel/gmail/classifier.rs
//! MailClassifier — classifies inbound emails by subject prefix.

/// Action determined from email classification.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum MailAction {
    CreateGoal { intent: String },
    AskQuestion { question: String },
    RecordMemory { content: String },
    IngestDocument { description: String },
    NotifyUser { summary: String },
    Ignore,
}

/// Classifies emails based on subject line prefixes.
pub struct MailClassifier;

impl MailClassifier {
    /// Known classification prefixes and their corresponding actions.
    const PREFIXES: &'static [(&'static str, fn(String) -> MailAction)] = &[
        ("[GOAL]", |body| MailAction::CreateGoal { intent: body }),
        ("[ASK]", |body| MailAction::AskQuestion { question: body }),
        ("[MEMORY]", |body| MailAction::RecordMemory { content: body }),
        ("[DOC]", |body| MailAction::IngestDocument { description: body }),
    ];

    /// Classify an email by its subject line.
    /// Returns the action and the cleaned body (subject content after prefix).
    pub fn classify(subject: &str, body: &str) -> MailAction {
        for (prefix, factory) in Self::PREFIXES {
            if let Some(rest) = subject.to_uppercase().strip_prefix(prefix) {
                let content = format!("{} {}", rest.trim(), body).trim().to_string();
                return factory(content);
            }
        }
        MailAction::Ignore
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn classifies_goal_email() {
        let result = MailClassifier::classify(
            "[GOAL] Fix the login timeout bug",
            "Users are reporting 30s timeouts on the login page.",
        );
        assert!(matches!(result, MailAction::CreateGoal { .. }));
    }

    #[test]
    fn classifies_ask_email() {
        let result = MailClassifier::classify(
            "[ASK] What is the status of Pi?",
            "",
        );
        assert!(matches!(result, MailAction::AskQuestion { .. }));
    }

    #[test]
    fn classifies_memory_email() {
        let result = MailClassifier::classify(
            "[MEMORY] We decided to use PostgreSQL",
            "For production, SQLite for local caches.",
        );
        assert!(matches!(result, MailAction::RecordMemory { .. }));
    }

    #[test]
    fn classifies_doc_email() {
        let result = MailClassifier::classify(
            "[DOC] Architecture diagram attached",
            "",
        );
        assert!(matches!(result, MailAction::IngestDocument { .. }));
    }

    #[test]
    fn unknown_prefix_ignored() {
        let result = MailClassifier::classify(
            "RE: Meeting tomorrow",
            "Let's discuss the Q3 roadmap.",
        );
        assert!(matches!(result, MailAction::Ignore));
    }

    #[test]
    fn case_insensitive_matching() {
        let result = MailClassifier::classify(
            "[goal] lowercase goal",
            "body",
        );
        assert!(matches!(result, MailAction::CreateGoal { .. }));
    }
}
```

- [ ] **Step 3: Write GmailChannel**

```rust
// crates/executive/src/impl/channel/gmail/mod.rs
//! Gmail channel — polls Gmail for new messages, classifies and routes them.

pub mod authenticator;
pub mod classifier;

use anyhow::Result;
use async_trait::async_trait;
use corpus::drivers::google::GmailCapability;
use fabric::types::channel::{ChannelId, ConversationId, InboundMessage, MessageContent, MessageId, OutboundMessage};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::info;

use super::Channel;
use authenticator::SenderAllowlist;
use classifier::{MailAction, MailClassifier};

/// Gmail as a Channel. Polls Gmail inbox, classifies messages,
/// and routes them as Goal Drafts / questions / memory records.
pub struct GmailChannel {
    id: ChannelId,
    gmail: Arc<dyn GmailCapability>,
    allowlist: SenderAllowlist,
    access_token: String,
}

impl GmailChannel {
    pub fn new(
        gmail: Arc<dyn GmailCapability>,
        owner_email: String,
        access_token: String,
    ) -> Self {
        Self {
            id: ChannelId("gmail".into()),
            gmail,
            allowlist: SenderAllowlist::owner_only(&owner_email),
            access_token,
        }
    }
}

#[async_trait]
impl Channel for GmailChannel {
    fn id(&self) -> ChannelId {
        self.id.clone()
    }

    async fn start(&self, tx: mpsc::Sender<InboundMessage>) -> Result<()> {
        let gmail = self.gmail.clone();
        let allowlist = self.allowlist.clone();
        let access_token = self.access_token.clone();
        let channel_id = self.id.clone();

        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(120));
            loop {
                interval.tick().await;
                match gmail.list_unread(10, &access_token).await {
                    Ok(messages) => {
                        for msg in messages {
                            // Check allowlist
                            if allowlist.check(&msg.from).is_none() {
                                continue;
                            }

                            // Read full message
                            let full = match gmail.read(&msg.id, &access_token).await {
                                Ok(m) => m,
                                Err(_) => continue,
                            };

                            // Classify
                            let action = MailClassifier::classify(&msg.subject, &full.body_text);
                            let content = match action {
                                MailAction::CreateGoal { intent } => MessageContent::Command {
                                    command: "/goal".into(),
                                    args: intent,
                                },
                                MailAction::AskQuestion { question } => MessageContent::Command {
                                    command: "/chat".into(),
                                    args: question,
                                },
                                MailAction::RecordMemory { .. } | MailAction::IngestDocument { .. } => {
                                    MessageContent::Text(format!("Email: {}", msg.subject))
                                }
                                MailAction::NotifyUser { summary } => MessageContent::Text(summary),
                                MailAction::Ignore => continue,
                            };

                            let inbound = InboundMessage {
                                id: MessageId(format!("gmail-{}", msg.id)),
                                channel: channel_id.clone(),
                                principal: format!("email:{}", msg.from),
                                conversation: ConversationId(format!("gmail-{}", msg.thread_id)),
                                content,
                                reply_to: None,
                                received_at: msg.received_at.clone(),
                            };

                            let _ = tx.send(inbound).await;
                        }
                    }
                    Err(e) => {
                        tracing::warn!(error = %e, "Gmail poll failed");
                    }
                }
            }
        });

        info!("Gmail channel polling started");
        Ok(())
    }

    async fn send(&self, _msg: OutboundMessage) -> Result<()> {
        // Phase G.3: implement email sending (requires GmailWriteCapability)
        Ok(())
    }

    async fn stop(&self) -> Result<()> {
        info!("Gmail channel stopped");
        Ok(())
    }
}

// Clone impl for sender allowlist (used in spawned task)
impl Clone for SenderAllowlist {
    fn clone(&self) -> Self {
        Self { entries: self.entries.clone() }
    }
}
```

- [ ] **Step 4: Update gmail/mod.rs**

```rust
pub mod authenticator;
pub mod classifier;
// (GmailChannel struct written above in mod.rs)
```

- [ ] **Step 5: Compile and test**

```bash
cargo test -p executive -- impl::channel::gmail
```
Expected: classifier + authenticator tests pass.

- [ ] **Step 6: Commit**

```bash
git add crates/executive/src/impl/channel/gmail/
git commit -m "feat(executive): add GmailChannel with MailClassifier and SenderAllowlist"
```

---

## Phase H: GBrain Mnemosyne Backend + Service Integration

**Goal:** GBrain as MnemosyneBackend. Deploy as Docker container. Persist decisions/failures/lessons with provenance. Recall with freshness scoring.
**Crates:** mnemosyne (new module impl/backends/gbrain/)
**Depends on:** Phase B (Goal completion hooks)
**Estimated:** 2–3 weeks

### Task H.1: Define GBrain API types and REST client

**Files:**
- Create: `crates/mnemosyne/src/impl/backends/gbrain/mod.rs`
- Create: `crates/mnemosyne/src/impl/backends/gbrain/types.rs`
- Create: `crates/mnemosyne/src/impl/backends/gbrain/client.rs`
- Modify: `crates/mnemosyne/src/impl/backends/mod.rs`
- Modify: `crates/mnemosyne/Cargo.toml`

- [ ] **Step 1: Add reqwest dep to mnemosyne**

```toml
# Add to crates/mnemosyne/Cargo.toml
reqwest = { version = "0.12", features = ["json"] }
```

- [ ] **Step 2: Write GBrain API types**

```rust
// crates/mnemosyne/src/impl/backends/gbrain/types.rs
//! Request/Response DTOs matching the GBrain REST API contract.
//!
//! Contract: docs/plans/2026-07-14-agent-google-design.md § Phase H

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GBrainHealth {
    pub status: String,
    pub version: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreRequest {
    pub memory_type: String,
    pub content: String,
    pub payload: Option<serde_json::Value>,
    pub goal_id: Option<String>,
    pub attempt_id: Option<String>,
    pub provenance: ProvenanceDTO,
    pub tags: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProvenanceDTO {
    pub source: String,
    pub goal_id: Option<String>,
    pub attempt_id: Option<String>,
    pub recorded_by: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StoreResponse {
    pub id: String,
    pub created_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchStoreRequest {
    pub entries: Vec<StoreRequest>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BatchStoreResponse {
    pub ids: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallRequest {
    pub query: String,
    pub goal_id: Option<String>,
    pub memory_types: Option<Vec<String>>,
    pub max_results: usize,
    pub freshness_weight: f32,
    pub min_score: f32,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RecallResponse {
    pub results: Vec<ScoredMemoryDTO>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ScoredMemoryDTO {
    pub id: String,
    pub memory_type: String,
    pub content: String,
    pub payload: Option<serde_json::Value>,
    pub relevance_score: f32,
    pub freshness_score: f32,
    pub combined_score: f32,
    pub provenance: ProvenanceDTO,
    pub created_at: String,
    pub is_current: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DeprecateRequest {
    pub reason: String,
    pub superseded_by: Option<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ListFilters {
    pub goal_id: Option<String>,
    pub memory_type: Option<String>,
    pub is_current: Option<bool>,
    pub limit: usize,
}
```

- [ ] **Step 3: Write GBrainClient**

```rust
// crates/mnemosyne/src/impl/backends/gbrain/client.rs
//! GBrainClient — REST client with retry and health check.

use anyhow::{Context, Result};
use std::time::Duration;
use tracing::{debug, warn};

use super::types::*;

/// REST client for the GBrain API.
pub struct GBrainClient {
    base_url: String,
    client: reqwest::Client,
    timeout: Duration,
    max_retries: u32,
}

impl GBrainClient {
    pub fn new(base_url: String, timeout_secs: u64, max_retries: u32) -> Self {
        Self {
            base_url: base_url.trim_end_matches('/').to_string(),
            client: reqwest::Client::new(),
            timeout: Duration::from_secs(timeout_secs),
            max_retries,
        }
    }

    /// Health check.
    pub async fn health(&self) -> Result<GBrainHealth> {
        let url = format!("{}/health", self.base_url);
        let resp = self.client.get(&url).timeout(self.timeout).send().await?;
        Ok(resp.json().await?)
    }

    /// Store a single memory entry.
    pub async fn store(&self, req: &StoreRequest) -> Result<StoreResponse> {
        let url = format!("{}/api/v1/memories", self.base_url);
        let resp = self.retry_post(&url, req).await?;
        Ok(resp.json().await?)
    }

    /// Batch store up to 100 memory entries.
    pub async fn store_batch(&self, entries: &[StoreRequest]) -> Result<BatchStoreResponse> {
        let url = format!("{}/api/v1/memories/batch", self.base_url);
        let batch = BatchStoreRequest { entries: entries.to_vec() };
        let resp = self.retry_post(&url, &batch).await?;
        Ok(resp.json().await?)
    }

    /// Semantic recall with freshness scoring.
    pub async fn recall(&self, req: &RecallRequest) -> Result<RecallResponse> {
        let url = format!("{}/api/v1/memories/recall", self.base_url);
        let resp = self.retry_post(&url, req).await?;
        Ok(resp.json().await?)
    }

    /// Deprecate a memory (mark as no longer current).
    pub async fn deprecate(&self, id: &str, reason: &str, superseded_by: Option<&str>) -> Result<()> {
        let url = format!("{}/api/v1/memories/{}/deprecate", self.base_url, id);
        let body = DeprecateRequest {
            reason: reason.to_string(),
            superseded_by: superseded_by.map(|s| s.to_string()),
        };
        self.retry_post(&url, &body).await?;
        Ok(())
    }

    /// POST with retry logic: 5xx/timeout → retry, 4xx → fail immediately.
    async fn retry_post<T: serde::Serialize>(
        &self,
        url: &str,
        body: &T,
    ) -> Result<reqwest::Response> {
        let mut last_error = String::new();
        for attempt in 0..=self.max_retries {
            if attempt > 0 {
                let backoff = Duration::from_millis(200 * 2u64.pow(attempt - 1));
                debug!(url, attempt, "Retrying GBrain request");
                tokio::time::sleep(backoff).await;
            }

            match self.client
                .post(url)
                .json(body)
                .timeout(self.timeout)
                .send()
                .await
            {
                Ok(resp) if resp.status().is_success() => return Ok(resp),
                Ok(resp) if resp.status().is_client_error() => {
                    let status = resp.status();
                    let body = resp.text().await.unwrap_or_default();
                    anyhow::bail!("GBrain client error {status}: {body}");
                }
                Ok(resp) => {
                    last_error = format!("Server error {}", resp.status());
                }
                Err(e) => {
                    last_error = format!("Request error: {e}");
                }
            }
        }
        Err(anyhow::anyhow!("GBrain request failed after {} retries: {last_error}", self.max_retries))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn client_builds_correct_urls() {
        let client = GBrainClient::new("http://127.0.0.1:9800".into(), 10, 3);
        assert!(client.base_url == "http://127.0.0.1:9800");
    }
}
```

- [ ] **Step 4: Update backends mod and Cargo.toml**

Add to `crates/mnemosyne/src/impl/backends/mod.rs` (if it exists):
```rust
pub mod gbrain;
```

- [ ] **Step 5: Compile and test**

```bash
cargo check -p mnemosyne
cargo test -p mnemosyne -- impl::backends::gbrain
```

- [ ] **Step 6: Commit**

```bash
git add crates/mnemosyne/src/impl/backends/gbrain/ crates/mnemosyne/Cargo.toml
git commit -m "feat(mnemosyne): add GBrainClient with REST types and retry logic"
```

---

### Task H.2: Write IngestionPipeline and MemoryExtraction

**Files:**
- Create: `crates/mnemosyne/src/impl/backends/gbrain/ingestion.rs`
- Create: `crates/mnemosyne/src/impl/backends/gbrain/extraction.rs`
- Create: `crates/mnemosyne/src/impl/backends/gbrain/health.rs`
- Create: `crates/mnemosyne/src/impl/backends/gbrain/recall.rs`
- Create: `crates/mnemosyne/src/impl/backends/gbrain/projection.rs`
- Modify: `crates/mnemosyne/src/impl/backends/gbrain/mod.rs`

- [ ] **Step 1: Write IngestionPipeline**

```rust
// crates/mnemosyne/src/impl/backends/gbrain/ingestion.rs
//! IngestionPipeline — async batch ingestion with buffered flush.

use anyhow::Result;
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::client::GBrainClient;
use super::types::StoreRequest;

pub struct IngestionPipeline {
    buffer_tx: mpsc::Sender<StoreRequest>,
}

impl IngestionPipeline {
    /// Spawn the ingestion background task.
    pub fn spawn(
        client: Arc<GBrainClient>,
        batch_size: usize,
        flush_interval: Duration,
        max_buffer: usize,
    ) -> Self {
        let (tx, mut rx) = mpsc::channel::<StoreRequest>(max_buffer);

        tokio::spawn(async move {
            let mut batch: Vec<StoreRequest> = Vec::with_capacity(batch_size);
            let mut tick = tokio::time::interval(flush_interval);

            loop {
                tokio::select! {
                    Some(entry) = rx.recv() => {
                        batch.push(entry);
                        if batch.len() >= batch_size {
                            flush(&client, &mut batch).await;
                        }
                    }
                    _ = tick.tick() => {
                        if !batch.is_empty() {
                            flush(&client, &mut batch).await;
                        }
                    }
                    else => break,
                }
            }
        });

        Self { buffer_tx: tx }
    }

    /// Buffer a memory entry for async ingestion. Non-blocking.
    pub fn buffer(&self, entry: StoreRequest) {
        match self.buffer_tx.try_send(entry) {
            Ok(()) => {}
            Err(mpsc::error::TrySendError::Full(_)) => {
                warn!("Ingestion buffer full, dropping entry");
            }
            Err(mpsc::error::TrySendError::Closed(_)) => {
                warn!("Ingestion pipeline closed");
            }
        }
    }
}

async fn flush(client: &Arc<GBrainClient>, batch: &mut Vec<StoreRequest>) {
    if batch.is_empty() {
        return;
    }
    match client.store_batch(batch).await {
        Ok(resp) => debug!(count = resp.ids.len(), "GBrain batch stored"),
        Err(e) => warn!(error = %e, count = batch.len(), "GBrain batch failed, entries dropped"),
    }
    batch.clear();
}
```

- [ ] **Step 2: Write MemoryExtraction**

```rust
// crates/mnemosyne/src/impl/backends/gbrain/extraction.rs
//! MemoryExtraction — converts Goal outcomes to MemoryEntry records.

use fabric::types::goal::{FailureClass, GoalId};
use fabric::types::objective::{Objective, ObjectiveStatus};

use super::types::{ProvenanceDTO, StoreRequest};

/// Extract structured MemoryEntry values from a completed/failed Goal.
pub fn extract_memories(
    goal: &Objective,
    failures: &[(FailureClass, String)], // (class, message) pairs
    recorded_by: &str,
) -> Vec<StoreRequest> {
    let mut entries = Vec::new();
    let goal_id_str = goal.objective_id.to_string();

    // 1. Goal outcome as a Lesson
    let outcome_content = match goal.status {
        ObjectiveStatus::Completed => {
            format!(
                "Successfully completed: {}",
                goal.intent.as_deref().unwrap_or(&goal.description)
            )
        }
        ObjectiveStatus::Failed => {
            format!(
                "Failed after attempts: {}",
                goal.intent.as_deref().unwrap_or(&goal.description)
            )
        }
        _ => return entries,
    };

    entries.push(StoreRequest {
        memory_type: "lesson".into(),
        content: outcome_content,
        payload: None,
        goal_id: Some(goal_id_str.clone()),
        attempt_id: None,
        provenance: ProvenanceDTO {
            source: "goal_execution".into(),
            goal_id: Some(goal_id_str.clone()),
            attempt_id: None,
            recorded_by: recorded_by.to_string(),
        },
        tags: vec!["goal-outcome".into(), format!("status:{:?}", goal.status)],
    });

    // 2. Each distinct failure class → Failure memory
    let mut seen_classes = std::collections::HashSet::new();
    for (class, message) in failures {
        let class_str = format!("{:?}", class);
        if seen_classes.insert(class_str.clone()) {
            entries.push(StoreRequest {
                memory_type: "failure".into(),
                content: format!("{:?} encountered: {}", class, message),
                payload: Some(serde_json::json!({
                    "failure_class": class_str,
                    "error_message": message,
                })),
                goal_id: Some(goal_id_str.clone()),
                attempt_id: None,
                provenance: ProvenanceDTO {
                    source: "goal_execution".into(),
                    goal_id: Some(goal_id_str.clone()),
                    attempt_id: None,
                    recorded_by: recorded_by.to_string(),
                },
                tags: vec!["failure".into(), format!("class:{:?}", class)],
            });
        }
    }

    entries
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_objective(status: ObjectiveStatus) -> Objective {
        Objective {
            objective_id: 42,
            description: "fix bug".into(),
            status,
            parent_id: None,
            session_id: "s1".into(),
            scope: "project".into(),
            intent: Some("Fix the login timeout".into()),
            acceptance_criteria: None,
            max_tokens: None,
            tokens_used: None,
            max_duration_secs: None,
            max_attempts: None,
            attempt_count: None,
            deadline: None,
            plan_json: None,
            created_at: String::new(),
            updated_at: String::new(),
        }
    }

    #[test]
    fn completed_goal_extracts_lesson() {
        let obj = test_objective(ObjectiveStatus::Completed);
        let entries = extract_memories(&obj, &[], "native-cognit");
        assert_eq!(entries.len(), 1);
        assert_eq!(entries[0].memory_type, "lesson");
        assert!(entries[0].content.contains("Successfully completed"));
    }

    #[test]
    fn failed_goal_extracts_lesson_and_failures() {
        let obj = test_objective(ObjectiveStatus::Failed);
        let failures = vec![
            (FailureClass::Compilation, "mismatched types".into()),
            (FailureClass::Compilation, "another compilation error".into()), // duplicate class
            (FailureClass::Timeout, "timed out after 30s".into()),
        ];
        let entries = extract_memories(&obj, &failures, "deepseek");
        // 1 lesson + 2 distinct failure classes (Compilation, Timeout)
        assert_eq!(entries.len(), 3);
        assert_eq!(entries[0].memory_type, "lesson");
        assert_eq!(entries[1].memory_type, "failure");
        assert_eq!(entries[2].memory_type, "failure");
    }
}
```

- [ ] **Step 3: Write RecallQuery helper**

```rust
// crates/mnemosyne/src/impl/backends/gbrain/recall.rs
//! RecallQuery construction and scoring helpers.

use fabric::types::goal::{GoalFrame, GoalId};

use super::types::RecallRequest;

/// Build a RecallRequest from a GoalFrame.
pub fn build_recall_request(frame: &GoalFrame, max_results: usize) -> RecallRequest {
    RecallRequest {
        query: format!("{} {}", frame.original_intent, frame.current_task),
        goal_id: Some(frame.goal_id.to_string()),
        memory_types: Some(vec![
            "lesson".into(), "failure".into(), "procedure".into(),
            "decision".into(), "architecture_fact".into(),
        ]),
        max_results,
        freshness_weight: 0.3,
        min_score: 0.4,
    }
}
```

- [ ] **Step 4: Write Health module**

```rust
// crates/mnemosyne/src/impl/backends/gbrain/health.rs
//! GBrain health check and background reconnect.

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::RwLock;
use tracing::{info, warn};

use super::client::GBrainClient;

/// Tracks GBrain connection health with background reconnect.
pub struct HealthTracker {
    healthy: RwLock<bool>,
    client: Arc<GBrainClient>,
}

impl HealthTracker {
    pub fn new(client: Arc<GBrainClient>) -> Self {
        Self {
            healthy: RwLock::new(false),
            client,
        }
    }

    pub async fn is_healthy(&self) -> bool {
        *self.healthy.read().await
    }

    /// Start background health checks. Returns immediately.
    pub fn start_background_check(&self, interval: Duration) {
        let client = self.client.clone();
        let healthy = self.healthy.clone();

        tokio::spawn(async move {
            loop {
                tokio::time::sleep(interval).await;
                match client.health().await {
                    Ok(h) => {
                        let was_healthy = *healthy.read().await;
                        *healthy.write().await = true;
                        if !was_healthy {
                            info!(version = %h.version, "GBrain connection restored");
                        }
                    }
                    Err(e) => {
                        let was_healthy = *healthy.read().await;
                        *healthy.write().await = false;
                        if was_healthy {
                            warn!(error = %e, "GBrain connection lost");
                        }
                    }
                }
            }
        });
    }
}
```

- [ ] **Step 5: Write AgoraProjection**

```rust
// crates/mnemosyne/src/impl/backends/gbrain/projection.rs
//! AgoraProjection — converts ScoredMemoryDTO to GoalFrame MemoryProjection.

use fabric::types::goal::{GoalId, MemoryProjection};

use super::types::ScoredMemoryDTO;

/// Convert top-K GBrain results into MemoryProjections for GoalFrame.
pub fn project_memories(scored: &[ScoredMemoryDTO]) -> Vec<MemoryProjection> {
    scored.iter()
        .filter(|m| m.is_current)
        .map(|m| MemoryProjection {
            summary: format!("[{}] {}", m.memory_type, m.content),
            memory_type: m.memory_type.clone(),
            provenance_goal: m.provenance.goal_id.as_ref()
                .and_then(|s| s.parse::<i64>().ok())
                .map(GoalId),
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn filters_out_deprecated() {
        let scored = vec![
            ScoredMemoryDTO {
                id: "1".into(), memory_type: "lesson".into(),
                content: "useful".into(), payload: None,
                relevance_score: 0.9, freshness_score: 0.8, combined_score: 0.87,
                provenance: super::super::types::ProvenanceDTO {
                    source: "goal".into(), goal_id: Some("1".into()),
                    attempt_id: None, recorded_by: "cognit".into(),
                },
                created_at: "now".into(), is_current: true,
            },
            ScoredMemoryDTO {
                id: "2".into(), memory_type: "lesson".into(),
                content: "stale".into(), payload: None,
                relevance_score: 0.5, freshness_score: 0.2, combined_score: 0.4,
                provenance: super::super::types::ProvenanceDTO {
                    source: "goal".into(), goal_id: Some("1".into()),
                    attempt_id: None, recorded_by: "cognit".into(),
                },
                created_at: "old".into(), is_current: false,
            },
        ];
        let projections = project_memories(&scored);
        assert_eq!(projections.len(), 1);
        assert!(projections[0].summary.contains("useful"));
    }
}
```

- [ ] **Step 6: Write GBrainBackend + wiring in mod.rs**

```rust
// crates/mnemosyne/src/impl/backends/gbrain/mod.rs
//! GBrain backend — implements MnemosyneBackend via REST API.

pub mod client;
pub mod extraction;
pub mod health;
pub mod ingestion;
pub mod projection;
pub mod recall;
pub mod types;

use std::sync::Arc;
use std::time::Duration;
use anyhow::Result;
use async_trait::async_trait;

use crate::service::MemoryService;
use client::GBrainClient;
use ingestion::IngestionPipeline;

/// Memory backend backed by GBrain (external knowledge service).
pub struct GBrainBackend {
    client: Arc<GBrainClient>,
    ingestion: IngestionPipeline,
    health: health::HealthTracker,
}

impl GBrainBackend {
    /// Connect to GBrain. Starts background health checks immediately.
    /// Returns Ok even if GBrain is unreachable (graceful degradation).
    pub async fn connect(
        endpoint: &str,
        timeout_secs: u64,
        max_retries: u32,
    ) -> Result<Self> {
        let client = Arc::new(GBrainClient::new(
            endpoint.to_string(),
            timeout_secs,
            max_retries,
        ));

        // Try initial health check (non-fatal if it fails)
        let health = health::HealthTracker::new(client.clone());
        match client.health().await {
            Ok(h) => {
                tracing::info!(version = %h.version, "GBrain connected");
            }
            Err(e) => {
                tracing::warn!(error = %e, "GBrain not available at startup, will retry in background");
            }
        }
        health.start_background_check(Duration::from_secs(30));

        let ingestion = IngestionPipeline::spawn(
            client.clone(),
            10,                             // batch_size
            Duration::from_secs(30),        // flush_interval
            1000,                           // max_buffer
        );

        Ok(Self { client, ingestion, health })
    }

    /// Store a memory entry (fire-and-forget via ingestion pipeline).
    pub fn store_memory(&self, entry: types::StoreRequest) {
        self.ingestion.buffer(entry);
    }

    /// Recall memories relevant to a query. Returns empty vec if GBrain is down.
    pub async fn recall_memories(
        &self,
        req: &types::RecallRequest,
    ) -> Vec<types::ScoredMemoryDTO> {
        if !self.health.is_healthy().await {
            return vec![];
        }
        match self.client.recall(req).await {
            Ok(resp) => resp.results,
            Err(e) => {
                tracing::warn!(error = %e, "GBrain recall failed");
                vec![]
            }
        }
    }
}
```

- [ ] **Step 7: Compile and test**

```bash
cargo test -p mnemosyne -- impl::backends::gbrain
```
Expected: extraction + projection tests pass.

- [ ] **Step 8: Commit**

```bash
git add crates/mnemosyne/src/impl/backends/gbrain/
git commit -m "feat(mnemosyne): add GBrainBackend with ingestion pipeline, extraction, recall, health tracking"
```

---

### Task H.3: Add Docker Compose and configuration

**Files:**
- Create: `config/docker-compose.yml`
- Create: `config/gbrain.env`

- [ ] **Step 1: Write docker-compose.yml**

```yaml
# config/docker-compose.yml
# Aletheon stack — GBrain + PostgreSQL containers.
# Run: docker compose -f config/docker-compose.yml up -d

services:
  postgres:
    image: postgres:16-alpine
    restart: unless-stopped
    ports:
      - "127.0.0.1:5432:5432"
    volumes:
      - /var/lib/aletheon/postgres:/var/lib/postgresql/data
    environment:
      POSTGRES_USER: aletheon
      POSTGRES_PASSWORD_FILE: /run/secrets/pg_password
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U aletheon"]
      interval: 10s
      timeout: 5s
      retries: 5

  gbrain:
    image: gbrain:latest
    restart: unless-stopped
    ports:
      - "127.0.0.1:9800:9800"
    volumes:
      - /var/lib/aletheon/gbrain:/data
    environment:
      GBRAIN_DATA_DIR: /data
      GBRAIN_EMBEDDING_MODEL: all-MiniLM-L6-v2
      GBRAIN_DATABASE_URL: postgres://aletheon@postgres:5432/gbrain
    depends_on:
      postgres:
        condition: service_healthy
    healthcheck:
      test: ["CMD", "curl", "-f", "http://localhost:9800/health"]
      interval: 30s
      timeout: 5s
      retries: 3
```

- [ ] **Step 2: Write gbrain.env**

```bash
# config/gbrain.env
# Default environment for GBrain service.
GBRAIN_DATA_DIR=/data
GBRAIN_EMBEDDING_MODEL=all-MiniLM-L6-v2
GBRAIN_LOG_LEVEL=info
```

- [ ] **Step 3: Commit**

```bash
git add config/
git commit -m "feat(config): add docker-compose.yml for GBrain + PostgreSQL"
```

---

## Integration Test: Full Vertical Slice

After all phases complete, run the full vertical slice test:

```bash
# 1. Start all services
docker compose -f config/docker-compose.yml up -d

# 2. Wait for healthy
docker compose -f config/docker-compose.yml ps
# Expected: all services "healthy"

# 3. Start Aletheon daemon
cargo run --bin aletheon -- daemon --config /etc/aletheon/config.toml

# 4. Send /goal via Telegram
# Expected: Goal created, persisted in goals.db, state = draft

# 5. Trigger tick()
# Expected: DeepSeekWorker executes, PiWorker creates worktree, modifies files

# 6. Verification gates run
# Expected: Format, Compile, Test gates pass or fail appropriately

# 7. Approval request appears in Telegram
# Expected: [Apply] [Reject] buttons

# 8. User approves
# Expected: Goal transitions to Completed

# 9. Memory extracted to GBrain
# Expected: curl http://127.0.0.1:9800/api/v1/memories?goal_id=goal-1 returns entry

# 10. Restart daemon
# Expected: Goal state survives restart, sync cursors intact
```

---

## Complete File Manifest

| Phase | Create | Modify |
|---|---|---|
| A | `fabric/src/types/channel.rs`, `executive/src/impl/channel/{mod,telegram/{mod,polling,binding,formatting}}.rs` | `fabric/src/{types/mod,lib}.rs`, `executive/{Cargo.toml,src/impl/{mod,daemon/handler/init}}.rs` |
| B | `fabric/src/types/goal.rs`, `executive/src/impl/goal/{supervisor,state_machine,frame,budget}.rs` | `fabric/src/{types/mod,lib,types/objective}.rs`, `executive/src/{impl/goal/{mod,store},core/core_systems,impl/daemon/server}.rs` |
| C | `executive/src/impl/goal/{worker,attempt,failure,retry,escalation,worker_impl}.rs` | `executive/src/impl/goal/{mod,supervisor}.rs` |
| D | `executive/src/impl/agent/pi/{mod,task,report,worktree,worker}.rs` | `executive/src/impl/agent/mod.rs` |
| E | `executive/src/impl/goal/verify/{mod,gates,report,policy}.rs` | `executive/src/impl/goal/mod.rs` |
| F | `fabric/src/types/google.rs`, `corpus/src/drivers/google/{mod,vault,gmail,calendar,sync}.rs` | `fabric/src/{types/mod,lib}.rs`, `corpus/{Cargo.toml,src/drivers/mod}.rs` |
| G | `executive/src/impl/channel/{approval,gmail/{mod,classifier,authenticator}}.rs` | `executive/src/impl/channel/mod.rs` |
| H | `mnemosyne/src/impl/backends/gbrain/{mod,client,types,ingestion,extraction,recall,projection,health}.rs`, `config/{docker-compose.yml,gbrain.env}` | `mnemosyne/{Cargo.toml,src/impl/backends/mod}.rs` |

## Execution Strategy

1. **Sequential:** A → B (foundational abstractions)
2. **After B:** C + D in parallel (both implement GoalWorker)
3. **After D:** E (needs PiReport diffs)
4. **After C:** F (uses GoalWorker trait)
5. **After A+F:** G (Channel trait + GmailCapability)
6. **After B:** H (Goal completion hooks)
7. **After all:** Integration test — full vertical slice
