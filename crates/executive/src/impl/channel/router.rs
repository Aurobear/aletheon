//! Channel routing boundaries.
//!
//! Defines the minimal transport and turn-execution traits that decouple
//! the channel router from the daemon runtime, plus a pure content-routing
//! function so the router is testable without constructing the full stack.

use std::sync::Arc;

use fabric::channel::{ConversationId, InboundMessage, MessageContent, MessageId, OutboundMessage};

use super::store::{ChannelStore, InsertOutcome};

// ---------------------------------------------------------------------------
// Transport trait
// ---------------------------------------------------------------------------

/// Minimal channel transport abstraction.
///
/// Implementations read from a provider inbox (cursor-based) and write
/// outbound messages back to the provider.
#[async_trait::async_trait]
pub trait ChannelTransport: Send + Sync {
    /// Stable identifier for this channel (e.g. `"telegram"`).
    fn channel_id(&self) -> &str;

    /// Receive pending messages since `cursor`, or from the start when
    /// `cursor` is `None`.
    async fn receive(
        &self,
        cursor: Option<String>,
    ) -> anyhow::Result<Vec<ProviderEnvelope>>;

    /// Send an outbound message. Returns the provider-assigned message id.
    async fn send(&self, message: &OutboundMessage) -> anyhow::Result<String>;
}

/// A provider message bundled with the cursor to use for the next
/// receive window.
#[derive(Debug)]
pub struct ProviderEnvelope {
    pub message: InboundMessage,
    pub next_cursor: String,
}

// ---------------------------------------------------------------------------
// Turn-execution trait
// ---------------------------------------------------------------------------

/// Minimal contract for executing a single turn.
///
/// This prevents router tests from needing the entire daemon stack.
/// The production adapter calls `DaemonTurnOrchestrator::execute_turn()`
/// and extracts either the `result` text or a stable error.
#[async_trait::async_trait]
pub trait ChannelTurnExecutor: Send + Sync {
    /// Execute a turn given the text input and a correlation id.
    ///
    /// Returns the result text on success or a stable error string on
    /// failure.
    async fn execute(
        &self,
        message: &str,
        correlation_id: &str,
    ) -> anyhow::Result<String>;
}

// ---------------------------------------------------------------------------
// Input routing (pure)
// ---------------------------------------------------------------------------

/// Classification of an inbound message for routing purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RoutedInput {
    /// `/start` — respond with a greeting, no LLM call.
    Greeting,
    /// Text to be executed as a chat turn.
    Chat(String),
    /// Feature not yet available (M2).
    GoalUnavailable,
    /// Input that the router cannot handle.
    Unsupported(String),
}

/// Classify a [`MessageContent`] into a [`RoutedInput`].
///
/// This is a pure function with no side-effects or async — easy to test.
pub fn route_content(content: &MessageContent) -> RoutedInput {
    match content {
        MessageContent::Command { command, args } => match command.as_str() {
            "/start" => RoutedInput::Greeting,
            "/chat" => RoutedInput::Chat(args.clone()),
            "/goal" | "/goals" | "/status" | "/pause" | "/resume" | "/cancel"
            | "/approve" | "/reject" => RoutedInput::GoalUnavailable,
            _ => RoutedInput::Unsupported(command.clone()),
        },
        MessageContent::Text { text } => {
            if text.trim().is_empty() {
                RoutedInput::Unsupported(String::new())
            } else {
                RoutedInput::Chat(text.clone())
            }
        }
    }
}

// ---------------------------------------------------------------------------
// Channel router
// ---------------------------------------------------------------------------

/// Durable owner-only channel message router.
///
/// Owns a [`ChannelStore`] for persistence and delegates AI turns to a
/// [`ChannelTurnExecutor`]. Rejection-check happens before the LLM is
/// invoked, and turn outcomes are persisted before the network send so
/// that a send failure retries only the outbox — never the LLM turn.
pub struct ChannelRouter {
    store: ChannelStore,
    turn_executor: Arc<dyn ChannelTurnExecutor>,
}

impl ChannelRouter {
    /// Create a new router that owns `store` and uses `turn_executor` for
    /// AI turn execution.
    pub fn new(store: ChannelStore, turn_executor: Arc<dyn ChannelTurnExecutor>) -> Self {
        Self {
            store,
            turn_executor,
        }
    }

    /// Process a single provider message envelope.
    ///
    /// # Algorithm
    ///
    /// 1. Insert into inbox; skip if duplicate.
    /// 2. Resolve the sender's active binding to a principal.
    /// 3. Unknown senders are marked rejected and cursor is advanced
    ///    (no LLM invocation, no outbox).
    /// 4. Normalize content via [`route_content`].
    /// 5. Execute the AI turn only for chat messages.
    /// 6. Build an outbound DTO from the routed input and optional AI reply.
    /// 7. Persist inbox-completed + outbox + cursor in one transaction.
    /// 8. Send the outbound message through the transport.
    /// 9. Mark the outbox row as sent or failed (never rolls back the
    ///    completed turn).
    pub async fn process(
        &mut self,
        transport: &dyn ChannelTransport,
        envelope: ProviderEnvelope,
    ) -> anyhow::Result<()> {
        let message = &envelope.message;
        let channel = message.channel_id.0.as_str();

        // 1. Insert into inbox; duplicate messages are silently skipped.
        match self.store.insert_inbound(message)? {
            InsertOutcome::Duplicate => return Ok(()),
            InsertOutcome::Inserted => { /* continue processing */ }
        }

        // 2. Resolve the active principal binding for this sender.
        let principal = self
            .store
            .resolve_principal(channel, &message.sender_id.0)?;

        // 3. Unknown sender: mark rejected, advance cursor, no LLM turn.
        if principal.is_none() {
            self.reject_inbound(channel, &message.message_id.0, &envelope.next_cursor)?;
            return Ok(());
        }

        // 4. Normalize the message content through command routing.
        let routed = route_content(&message.content);

        // 5. Execute AI turn only for chat messages.
        let mut ai_reply: Option<String> = None;
        if let RoutedInput::Chat(text) = &routed {
            match self
                .turn_executor
                .execute(text, &message.correlation_id)
                .await
            {
                Ok(reply) => ai_reply = Some(reply),
                Err(e) => {
                    // Executor failure: mark inbox failed so it stays
                    // retryable, do NOT advance the cursor.
                    self.fail_inbound(channel, &message.message_id.0, &e.to_string())?;
                    return Err(e);
                }
            }
        }

        // 6. Build the outbound message DTO.
        let outbound = build_outbound(
            &routed,
            &message.conversation_id,
            &message.message_id,
            &message.correlation_id,
            ai_reply.as_deref(),
        );

        // 7. Persist inbox+outbox+cursor in one atomic transaction.
        self.store.complete_inbound(
            channel,
            &message.message_id.0,
            &envelope.next_cursor,
            &outbound,
        )?;

        // 8. Attempt the network send.
        match transport.send(&outbound).await {
            Ok(_provider_msg_id) => {
                // 9a. Mark outbox row as sent.
                self.store.db.execute(
                    "UPDATE channel_outbox SET status = 'sent', updated_at = datetime('now')
                     WHERE correlation_id = ?1",
                    rusqlite::params![message.correlation_id],
                )?;
            }
            Err(e) => {
                // 9b. Mark outbox row as failed so it can be retried
                //     independently of the already-completed inbox turn.
                self.store.db.execute(
                    "UPDATE channel_outbox SET status = 'failed', last_error = ?1, updated_at = datetime('now')
                     WHERE correlation_id = ?2",
                    rusqlite::params![e.to_string(), message.correlation_id],
                )?;
            }
        }

        Ok(())
    }

    /// Mark an inbox message as rejected and advance the cursor.
    ///
    /// No outbox row is created — rejected senders receive no reply.
    fn reject_inbound(
        &mut self,
        channel: &str,
        message_id: &str,
        next_cursor: &str,
    ) -> anyhow::Result<()> {
        let tx = self.store.db.transaction()?;

        tx.execute(
            "UPDATE channel_inbox SET status = 'rejected', result_json = '{\"reason\":\"unknown sender\"}', updated_at = datetime('now')
             WHERE channel_id = ?1 AND message_id = ?2",
            rusqlite::params![channel, message_id],
        )?;

        tx.execute(
            "INSERT INTO channel_cursor (channel_id, cursor, updated_at)
             VALUES (?1, ?2, datetime('now'))
             ON CONFLICT(channel_id) DO UPDATE SET cursor = excluded.cursor, updated_at = excluded.updated_at",
            rusqlite::params![channel, next_cursor],
        )?;

        tx.commit()?;
        Ok(())
    }

    /// Mark an inbox message as failed (leaving it retryable) without
    /// advancing the cursor.
    fn fail_inbound(
        &mut self,
        channel: &str,
        message_id: &str,
        error: &str,
    ) -> anyhow::Result<()> {
        self.store.db.execute(
            "UPDATE channel_inbox SET status = 'failed', result_json = ?3,
             attempt_count = attempt_count + 1, updated_at = datetime('now')
             WHERE channel_id = ?1 AND message_id = ?2",
            rusqlite::params![channel, message_id, format!(r#"{{"error":"{}"}}"#, error)],
        )?;
        Ok(())
    }

    // -----------------------------------------------------------------------
    // Restart recovery
    // -----------------------------------------------------------------------

    /// Recover pending inbox messages after a restart.
    pub async fn recover_pending_inbox(
        &mut self,
        transport: &dyn ChannelTransport,
        limit: usize,
    ) -> anyhow::Result<usize> {
        let channel = transport.channel_id();
        let pending = self.store.pending_inbound(channel, limit)?;
        let mut count = 0usize;

        for msg in &pending {
            let channel_str = msg.channel_id.0.as_str();
            let principal = self
                .store
                .resolve_principal(channel_str, &msg.sender_id.0)?;
            if principal.is_none() {
                self.reject_inbound(channel_str, &msg.message_id.0, &msg.message_id.0)?;
                count += 1;
                continue;
            }
            let routed = route_content(&msg.content);
            let mut ai_reply: Option<String> = None;
            if let RoutedInput::Chat(text) = &routed {
                match self.turn_executor.execute(text, &msg.correlation_id).await {
                    Ok(reply) => ai_reply = Some(reply),
                    Err(e) => {
                        self.fail_inbound(channel_str, &msg.message_id.0, &e.to_string())?;
                        return Err(e);
                    }
                }
            }
            let outbound = build_outbound(
                &routed,
                &msg.conversation_id,
                &msg.message_id,
                &msg.correlation_id,
                ai_reply.as_deref(),
            );
            self.store.complete_inbound(
                channel_str,
                &msg.message_id.0,
                &msg.message_id.0,
                &outbound,
            )?;
            match transport.send(&outbound).await {
                Ok(_) => {
                    self.store.db.execute(
                        "UPDATE channel_outbox SET status = 'sent', updated_at = datetime('now')
                         WHERE correlation_id = ?1",
                        rusqlite::params![msg.correlation_id],
                    )?;
                }
                Err(e) => {
                    self.store.db.execute(
                        "UPDATE channel_outbox SET status = 'failed', last_error = ?1, updated_at = datetime('now')
                         WHERE correlation_id = ?2",
                        rusqlite::params![e.to_string(), msg.correlation_id],
                    )?;
                }
            }
            count += 1;
        }
        Ok(count)
    }

    /// Flush pending and failed outbox messages after a restart.
    ///
    /// # At-least-once boundary
    ///
    /// If the original `transport.send()` succeeded but the outbox-status
    /// update crashed, this method will re-send the same outbound message.
    /// The provider may deliver the same reply twice. The LLM turn is never
    /// re-executed because inbox completion and outbox insertion happen
    /// atomically before the send.
    pub async fn flush_pending_outbox(
        &self,
        transport: &dyn ChannelTransport,
        limit: usize,
    ) -> anyhow::Result<usize> {
        let channel = transport.channel_id();
        let pending = self.store.pending_outbox(channel, limit)?;
        let mut count = 0usize;
        for outbound in &pending {
            match transport.send(outbound).await {
                Ok(_) => {
                    self.store.db.execute(
                        "UPDATE channel_outbox SET status = 'sent', updated_at = datetime('now')
                         WHERE correlation_id = ?1",
                        rusqlite::params![outbound.correlation_id],
                    )?;
                }
                Err(e) => {
                    self.store.db.execute(
                        "UPDATE channel_outbox SET status = 'failed', last_error = ?1, updated_at = datetime('now')
                         WHERE correlation_id = ?2",
                        rusqlite::params![e.to_string(), outbound.correlation_id],
                    )?;
                }
            }
            count += 1;
        }
        Ok(count)
    }

    /// Expose the store for tests to inspect state.
    #[cfg(test)]
    #[allow(dead_code)]
    pub fn store(&self) -> &ChannelStore {
        &self.store
    }
}

/// Build an [`OutboundMessage`] from a routed input and optional AI reply.
fn build_outbound(
    routed: &RoutedInput,
    conversation_id: &ConversationId,
    message_id: &MessageId,
    correlation_id: &str,
    ai_reply: Option<&str>,
) -> OutboundMessage {
    let content = match routed {
        RoutedInput::Chat(_) => MessageContent::Text {
            text: ai_reply.unwrap_or_default().to_string(),
        },
        RoutedInput::Greeting => MessageContent::Text {
            text: "Hello! I am Aletheon. How can I help you today?".into(),
        },
        RoutedInput::GoalUnavailable => MessageContent::Text {
            text: "Objective creation via chat is not yet available (targeting M2).".into(),
        },
        RoutedInput::Unsupported(_) => MessageContent::Text {
            text: "I don't recognize your identity. Please contact an administrator.".into(),
        },
    };

    OutboundMessage {
        conversation_id: conversation_id.clone(),
        content,
        actions: vec![],
        reply_to: Some(message_id.clone()),
        correlation_id: correlation_id.to_string(),
    }
}

// ---------------------------------------------------------------------------
// Tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_command_is_greeting() {
        let content = MessageContent::Command {
            command: "/start".into(),
            args: String::new(),
        };
        assert_eq!(route_content(&content), RoutedInput::Greeting);
    }

    #[test]
    fn chat_command_forwards_text() {
        let content = MessageContent::Command {
            command: "/chat".into(),
            args: "hello world".into(),
        };
        assert_eq!(
            route_content(&content),
            RoutedInput::Chat("hello world".into())
        );
    }

    #[test]
    fn plain_text_is_chat() {
        let content = MessageContent::Text {
            text: "tell me a joke".into(),
        };
        assert_eq!(
            route_content(&content),
            RoutedInput::Chat("tell me a joke".into())
        );
    }

    #[test]
    fn empty_text_is_unsupported() {
        let content = MessageContent::Text {
            text: String::new(),
        };
        assert_eq!(
            route_content(&content),
            RoutedInput::Unsupported(String::new())
        );
    }

    #[test]
    fn whitespace_only_text_is_unsupported() {
        let content = MessageContent::Text {
            text: "   ".into(),
        };
        assert_eq!(
            route_content(&content),
            RoutedInput::Unsupported(String::new())
        );
    }

    #[test]
    fn m2_commands_are_goal_unavailable() {
        for cmd in &[
            "/goal", "/goals", "/status", "/pause", "/resume", "/cancel",
            "/approve", "/reject",
        ] {
            let content = MessageContent::Command {
                command: (*cmd).into(),
                args: String::new(),
            };
            assert_eq!(
                route_content(&content),
                RoutedInput::GoalUnavailable,
                "command {cmd} should be GoalUnavailable"
            );
        }
    }

    #[test]
    fn unknown_command_is_unsupported() {
        let content = MessageContent::Command {
            command: "/unknown".into(),
            args: String::new(),
        };
        assert_eq!(
            route_content(&content),
            RoutedInput::Unsupported("/unknown".into())
        );
    }
}
