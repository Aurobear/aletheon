//! Channel routing boundaries.
//!
//! Defines the minimal transport and turn-execution traits that decouple
//! the channel router from the daemon runtime, plus a pure content-routing
//! function so the router is testable without constructing the full stack.

use std::sync::Arc;

use fabric::channel::{ConversationId, InboundMessage, MessageContent, MessageId, OutboundMessage};
use fabric::{AttemptId, GoalId, GoalSnapshot};

use crate::r#impl::goal::{AttemptCoordinationOutcome, RetryDecision};

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
    async fn receive(&self, cursor: Option<String>) -> anyhow::Result<Vec<ProviderEnvelope>>;

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
    async fn execute(&self, message: &str, correlation_id: &str) -> anyhow::Result<String>;
}

#[async_trait::async_trait]
pub trait ChannelGoalExecutor: Send + Sync {
    async fn create_draft(&self, owner: &str, intent: &str) -> anyhow::Result<GoalSnapshot>;
    async fn list(&self, owner: &str) -> anyhow::Result<Vec<GoalSnapshot>>;
    async fn show(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
    async fn pause(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
    async fn resume(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
    async fn cancel(&self, owner: &str, id: GoalId) -> anyhow::Result<GoalSnapshot>;
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
    /// Owner-scoped persistent Goal lifecycle command.
    GoalCommand { command: String, args: String },
    /// Input that the router cannot handle.
    Unsupported(String),
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoalProgressKind {
    Succeeded,
    RetryBackoff,
    Escalated,
    AwaitingHuman,
    Failed,
    Cancelled,
}

impl GoalProgressKind {
    fn as_str(self) -> &'static str {
        match self {
            Self::Succeeded => "succeeded",
            Self::RetryBackoff => "retry_backoff",
            Self::Escalated => "escalated",
            Self::AwaitingHuman => "awaiting_human",
            Self::Failed => "failed",
            Self::Cancelled => "cancelled",
        }
    }
}

/// Bounded proactive Goal notification. Raw provider output and errors are
/// deliberately absent from this contract.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GoalProgress {
    pub goal_id: GoalId,
    pub attempt_id: AttemptId,
    pub kind: GoalProgressKind,
}

impl GoalProgress {
    pub fn from_outcome(outcome: &AttemptCoordinationOutcome) -> Self {
        match outcome {
            AttemptCoordinationOutcome::Succeeded { attempt, .. } => Self {
                goal_id: attempt.goal_id,
                attempt_id: attempt.id,
                kind: GoalProgressKind::Succeeded,
            },
            AttemptCoordinationOutcome::Failed {
                attempt, decision, ..
            } => Self {
                goal_id: attempt.goal_id,
                attempt_id: attempt.id,
                kind: match decision {
                    RetryDecision::RetrySame { .. } => GoalProgressKind::RetryBackoff,
                    RetryDecision::Escalate { .. } => GoalProgressKind::Escalated,
                    RetryDecision::AwaitHuman { .. } => GoalProgressKind::AwaitingHuman,
                    RetryDecision::Fail { .. } => GoalProgressKind::Failed,
                    RetryDecision::Cancel => GoalProgressKind::Cancelled,
                },
            },
        }
    }

    fn text(&self) -> String {
        let status = match self.kind {
            GoalProgressKind::Succeeded => "completed successfully",
            GoalProgressKind::RetryBackoff => "will retry after bounded backoff",
            GoalProgressKind::Escalated => "escalated to reviewer",
            GoalProgressKind::AwaitingHuman => "is awaiting human input",
            GoalProgressKind::Failed => "failed",
            GoalProgressKind::Cancelled => "was cancelled",
        };
        format!(
            "Goal {} attempt {} {status}.",
            self.goal_id.0, self.attempt_id.0
        )
    }

    fn correlation_id(&self) -> String {
        format!(
            "goal:{}:attempt:{}:{}",
            self.goal_id.0,
            self.attempt_id.0,
            self.kind.as_str()
        )
    }
}

/// Classify a [`MessageContent`] into a [`RoutedInput`].
///
/// This is a pure function with no side-effects or async — easy to test.
pub fn route_content(content: &MessageContent) -> RoutedInput {
    match content {
        MessageContent::Command { command, args } => match command.as_str() {
            "/start" => RoutedInput::Greeting,
            "/chat" => RoutedInput::Chat(args.clone()),
            "/goal" | "/goals" | "/status" | "/pause" | "/resume" | "/cancel" => {
                RoutedInput::GoalCommand {
                    command: command.clone(),
                    args: args.clone(),
                }
            }
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
    goal_executor: Option<Arc<dyn ChannelGoalExecutor>>,
}

impl ChannelRouter {
    /// Create a new router that owns `store` and uses `turn_executor` for
    /// AI turn execution.
    pub fn new(store: ChannelStore, turn_executor: Arc<dyn ChannelTurnExecutor>) -> Self {
        Self {
            store,
            turn_executor,
            goal_executor: None,
        }
    }

    pub fn with_goal_executor(mut self, executor: Arc<dyn ChannelGoalExecutor>) -> Self {
        self.goal_executor = Some(executor);
        self
    }

    /// Persist a proactive Goal progress notification, then attempt delivery.
    /// The caller can only construct `progress` from an already-persisted
    /// AttemptCoordinator outcome, preserving Goal-event-before-send ordering.
    pub async fn notify_goal_progress(
        &self,
        transport: &dyn ChannelTransport,
        conversation_id: ConversationId,
        progress: &GoalProgress,
    ) -> anyhow::Result<bool> {
        let outbound = OutboundMessage {
            conversation_id,
            content: MessageContent::Text {
                text: progress.text(),
            },
            actions: vec![],
            reply_to: None,
            correlation_id: progress.correlation_id(),
        };
        if !self
            .store
            .enqueue_outbound(transport.channel_id(), &outbound)?
        {
            return Ok(false);
        }
        match transport.send(&outbound).await {
            Ok(provider_id) => {
                self.store
                    .mark_outbound_sent(&outbound.correlation_id, &provider_id)?;
                Ok(true)
            }
            Err(error) => {
                self.store
                    .mark_outbound_failed(&outbound.correlation_id, &error.to_string())?;
                Ok(false)
            }
        }
    }

    async fn execute_goal_command(
        &mut self,
        owner: &str,
        command: &str,
        args: &str,
    ) -> anyhow::Result<String> {
        let executor = self
            .goal_executor
            .as_ref()
            .ok_or_else(|| anyhow::anyhow!("Goal runtime is not configured"))?;
        if command == "/goal" {
            if args.trim().is_empty() {
                anyhow::bail!("usage: /goal <intent>");
            }
            let goal = executor.create_draft(owner, args.trim()).await?;
            return Ok(format!(
                "Goal {} created as draft: {}",
                goal.id.0, goal.spec.original_intent
            ));
        }
        if command == "/goals" {
            let goals = executor.list(owner).await?;
            if goals.is_empty() {
                return Ok("No goals.".into());
            }
            return Ok(goals
                .iter()
                .map(|g| format!("{} {} {}", g.id.0, g.state, g.spec.original_intent))
                .collect::<Vec<_>>()
                .join("\n"));
        }
        let id = args
            .trim()
            .parse::<i64>()
            .map(GoalId)
            .map_err(|_| anyhow::anyhow!("usage: {command} <goal-id>"))?;
        let goal = match command {
            "/status" => executor.show(owner, id).await?,
            "/pause" => executor.pause(owner, id).await?,
            "/resume" => executor.resume(owner, id).await?,
            "/cancel" => executor.cancel(owner, id).await?,
            _ => anyhow::bail!("unsupported goal command"),
        };
        Ok(format!("Goal {}: {}", goal.id.0, goal.state))
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
        if let RoutedInput::GoalCommand { command, args } = &routed {
            match self
                .execute_goal_command(principal.as_deref().unwrap(), command, args)
                .await
            {
                Ok(reply) => ai_reply = Some(reply),
                Err(error) => ai_reply = Some(error.to_string()),
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
    fn fail_inbound(&mut self, channel: &str, message_id: &str, error: &str) -> anyhow::Result<()> {
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
            if let RoutedInput::GoalCommand { command, args } = &routed {
                match self
                    .execute_goal_command(principal.as_deref().unwrap(), command, args)
                    .await
                {
                    Ok(reply) => ai_reply = Some(reply),
                    Err(error) => ai_reply = Some(error.to_string()),
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
        RoutedInput::GoalCommand { .. } => MessageContent::Text {
            text: ai_reply
                .unwrap_or("Goal runtime is not configured")
                .to_string(),
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
        let content = MessageContent::Text { text: "   ".into() };
        assert_eq!(
            route_content(&content),
            RoutedInput::Unsupported(String::new())
        );
    }

    #[test]
    fn m2_commands_are_goal_commands() {
        for cmd in &["/goal", "/goals", "/status", "/pause", "/resume", "/cancel"] {
            let content = MessageContent::Command {
                command: (*cmd).into(),
                args: String::new(),
            };
            assert_eq!(
                route_content(&content),
                RoutedInput::GoalCommand {
                    command: (*cmd).into(),
                    args: String::new()
                },
                "command {cmd} should be routed to Goal runtime"
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
