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
