use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ChannelId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct MessageId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ConversationId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct ExternalSenderId(pub String);

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum MessageContent {
    Text { text: String },
    Command { command: String, args: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ActionType {
    Callback,
    Url,
    Approve,
    Reject,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct UserAction {
    pub action_id: String,
    pub label: String,
    pub action_type: ActionType,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct InboundMessage {
    pub channel_id: ChannelId,
    pub message_id: MessageId,
    pub conversation_id: ConversationId,
    pub sender_id: ExternalSenderId,
    pub content: MessageContent,
    pub timestamp_ms: i64,
    pub reply_to_action: Option<String>,
    pub correlation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct OutboundMessage {
    pub conversation_id: ConversationId,
    pub content: MessageContent,
    pub actions: Vec<UserAction>,
    pub reply_to: Option<MessageId>,
    pub correlation_id: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "state", rename_all = "snake_case")]
pub enum ChannelHealth {
    Healthy,
    Degraded { reason: String },
    Disconnected { since_ms: i64, reason: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn inbound_text_round_trips() {
        let message = InboundMessage {
            channel_id: ChannelId("telegram".into()),
            message_id: MessageId("42".into()),
            conversation_id: ConversationId("1001".into()),
            sender_id: ExternalSenderId("telegram:7".into()),
            content: MessageContent::Text {
                text: "hello".into(),
            },
            timestamp_ms: 1_720_000_000_000,
            reply_to_action: None,
            correlation_id: "telegram:42".into(),
        };
        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(
            serde_json::from_str::<InboundMessage>(&json).unwrap(),
            message
        );
    }

    #[test]
    fn outbound_actions_round_trip() {
        let message = OutboundMessage {
            conversation_id: ConversationId("1001".into()),
            content: MessageContent::Text {
                text: "continue?".into(),
            },
            actions: vec![UserAction {
                action_id: "approve:abc".into(),
                label: "Approve".into(),
                action_type: ActionType::Approve,
            }],
            reply_to: Some(MessageId("42".into())),
            correlation_id: "approval:abc".into(),
        };
        let json = serde_json::to_string(&message).unwrap();
        assert_eq!(
            serde_json::from_str::<OutboundMessage>(&json).unwrap(),
            message
        );
    }
}
