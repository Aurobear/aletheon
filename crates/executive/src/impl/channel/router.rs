//! Channel routing boundaries.
//!
//! Defines the minimal transport and turn-execution traits that decouple
//! the channel router from the daemon runtime, plus a pure content-routing
//! function so the router is testable without constructing the full stack.

use fabric::channel::{InboundMessage, MessageContent, OutboundMessage};

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
enum RoutedInput {
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
fn route_content(content: &MessageContent) -> RoutedInput {
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
