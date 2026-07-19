//! Pure input classification for the channel router.
//!
//! This module is free of side-effects or async — easy to test in
//! isolation from the rest of the channel stack.

use fabric::channel::MessageContent;

/// Classification of an inbound message for routing purposes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Intent {
    /// `/start` — respond with a greeting, no LLM call.
    Greeting,
    /// Text to be executed as a chat turn.
    Chat(String),
    /// Owner-scoped persistent Goal lifecycle command.
    GoalCommand { command: String, args: String },
    /// Input that the router cannot handle.
    Unsupported(String),
}

/// Classify a [`MessageContent`] into an [`Intent`].
///
/// This is a pure function with no side-effects or async — easy to test.
pub fn classify_intent(content: &MessageContent) -> Intent {
    match content {
        MessageContent::Command { command, args } => match command.as_str() {
            "/start" => Intent::Greeting,
            "/chat" => Intent::Chat(args.clone()),
            "/goal" | "/goals" | "/status" | "/pause" | "/resume" | "/cancel" | "/edit" => {
                Intent::GoalCommand {
                    command: command.clone(),
                    args: args.clone(),
                }
            }
            _ => Intent::Unsupported(command.clone()),
        },
        MessageContent::Text { text } => {
            if text.trim().is_empty() {
                Intent::Unsupported(String::new())
            } else {
                Intent::Chat(text.clone())
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn start_command_is_greeting() {
        let content = MessageContent::Command {
            command: "/start".into(),
            args: String::new(),
        };
        assert_eq!(classify_intent(&content), Intent::Greeting);
    }

    #[test]
    fn chat_command_forwards_text() {
        let content = MessageContent::Command {
            command: "/chat".into(),
            args: "hello world".into(),
        };
        assert_eq!(
            classify_intent(&content),
            Intent::Chat("hello world".into())
        );
    }

    #[test]
    fn plain_text_is_chat() {
        let content = MessageContent::Text {
            text: "tell me a joke".into(),
        };
        assert_eq!(
            classify_intent(&content),
            Intent::Chat("tell me a joke".into())
        );
    }

    #[test]
    fn empty_text_is_unsupported() {
        let content = MessageContent::Text {
            text: String::new(),
        };
        assert_eq!(
            classify_intent(&content),
            Intent::Unsupported(String::new())
        );
    }

    #[test]
    fn whitespace_only_text_is_unsupported() {
        let content = MessageContent::Text { text: "   ".into() };
        assert_eq!(
            classify_intent(&content),
            Intent::Unsupported(String::new())
        );
    }

    #[test]
    fn m2_commands_are_goal_commands() {
        for cmd in &[
            "/goal", "/goals", "/status", "/pause", "/resume", "/cancel", "/edit",
        ] {
            let content = MessageContent::Command {
                command: (*cmd).into(),
                args: String::new(),
            };
            assert_eq!(
                classify_intent(&content),
                Intent::GoalCommand {
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
            classify_intent(&content),
            Intent::Unsupported("/unknown".into())
        );
    }
}
