//! Telegram Bot API JSON DTOs — only the fields used by M1 long-poll transport.
//!
//! All optional Telegram fields use `Option` so unknown/new fields added by
//! Telegram in the future are silently ignored.

use serde::{Deserialize, Serialize};

// ---------------------------------------------------------------------------
// getUpdates
// ---------------------------------------------------------------------------

/// `GET /bot<token>/getUpdates` response envelope.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct GetUpdatesResponse {
    pub ok: bool,
    #[serde(default)]
    pub result: Vec<Update>,
    #[serde(default)]
    pub description: Option<String>,
}

/// A single Telegram update.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Update {
    pub update_id: i64,
    #[serde(default)]
    pub message: Option<Message>,
    #[serde(default)]
    pub callback_query: Option<CallbackQuery>,
}

#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct CallbackQuery {
    pub id: String,
    pub from: User,
    #[serde(default)]
    pub message: Option<Message>,
    #[serde(default)]
    pub data: Option<String>,
}

/// A Telegram message.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Message {
    pub message_id: i64,
    pub date: i64,
    pub chat: Chat,
    #[serde(default)]
    pub from: Option<User>,
    #[serde(default)]
    pub text: Option<String>,
}

/// A Telegram chat.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct Chat {
    pub id: i64,
}

/// A Telegram user.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct User {
    pub id: i64,
    #[serde(default)]
    pub first_name: Option<String>,
}

// ---------------------------------------------------------------------------
// sendMessage
// ---------------------------------------------------------------------------

/// `POST /bot<token>/sendMessage` response envelope.
#[derive(Debug, Clone, Deserialize, Serialize, PartialEq)]
pub struct SendMessageResponse {
    pub ok: bool,
    #[serde(default)]
    pub result: Option<Message>,
    #[serde(default)]
    pub description: Option<String>,
}

/// `POST /bot<token>/sendMessage` request body.
#[derive(Debug, Clone, Serialize)]
pub struct SendMessageRequest {
    pub chat_id: i64,
    pub text: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub reply_markup: Option<InlineKeyboardMarkup>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InlineKeyboardMarkup {
    pub inline_keyboard: Vec<Vec<InlineKeyboardButton>>,
}

#[derive(Debug, Clone, Serialize)]
pub struct InlineKeyboardButton {
    pub text: String,
    pub callback_data: String,
}

// ---------------------------------------------------------------------------
// tests
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;

    // -- getUpdates fixtures --------------------------------------------------

    #[test]
    fn deserialize_text_message() {
        let json = r#"{
            "ok": true,
            "result": [{
                "update_id": 100,
                "message": {
                    "message_id": 10,
                    "from": { "id": 7, "first_name": "Alice" },
                    "chat": { "id": 1001 },
                    "date": 1720000000,
                    "text": "hello"
                }
            }]
        }"#;
        let resp: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.result.len(), 1);
        let msg = resp.result[0].message.as_ref().unwrap();
        assert_eq!(msg.text.as_deref(), Some("hello"));
        assert_eq!(msg.from.as_ref().unwrap().id, 7);
    }

    #[test]
    fn deserialize_slash_command() {
        let json = r#"{
            "ok": true,
            "result": [{
                "update_id": 200,
                "message": {
                    "message_id": 20,
                    "from": { "id": 99 },
                    "chat": { "id": 2001 },
                    "date": 1720000100,
                    "text": "/chat hello"
                }
            }]
        }"#;
        let resp: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        let msg = resp.result[0].message.as_ref().unwrap();
        assert_eq!(msg.text.as_deref(), Some("/chat hello"));
        assert!(msg.text.as_deref().unwrap().starts_with('/'));
    }

    #[test]
    fn deserialize_update_without_message() {
        // e.g. edited_message, channel_post, callback_query, etc.
        let json = r#"{
            "ok": true,
            "result": [{
                "update_id": 300,
                "edited_message": {
                    "message_id": 30,
                    "from": { "id": 1 },
                    "chat": { "id": 3001 },
                    "date": 1720000200,
                    "text": "edited"
                }
            }]
        }"#;
        let resp: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        // The update has no `message` field — it should be None.
        assert!(resp.result[0].message.is_none());
    }

    #[test]
    fn deserialize_callback_query() {
        let json = r#"{
            "ok": true,
            "result": [{
                "update_id": 301,
                "callback_query": {
                    "id": "callback-1",
                    "from": { "id": 7 },
                    "message": {
                        "message_id": 30,
                        "chat": { "id": 3001 },
                        "date": 1720000200,
                        "text": "approval"
                    },
                    "data": "00000000-0000-0000-0000-000000000001:apply"
                }
            }]
        }"#;
        let response: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        let callback = response.result[0].callback_query.as_ref().unwrap();
        assert_eq!(callback.from.id, 7);
        assert_eq!(
            callback.data.as_deref(),
            Some("00000000-0000-0000-0000-000000000001:apply")
        );
    }

    #[test]
    fn deserialize_api_error() {
        let json = r#"{
            "ok": false,
            "description": "Unauthorized"
        }"#;
        let resp: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        assert!(!resp.ok);
        assert_eq!(resp.description.as_deref(), Some("Unauthorized"));
        assert!(resp.result.is_empty());
    }

    // -- sendMessage response ------------------------------------------------

    #[test]
    fn deserialize_send_message_ok() {
        let json = r#"{
            "ok": true,
            "result": {
                "message_id": 99,
                "from": { "id": 888, "first_name": "Bot" },
                "chat": { "id": 1001 },
                "date": 1720000300,
                "text": "hello back"
            }
        }"#;
        let resp: SendMessageResponse = serde_json::from_str(json).unwrap();
        assert!(resp.ok);
        assert_eq!(resp.result.unwrap().text.as_deref(), Some("hello back"));
    }

    // -- unknown fields are silently ignored ----------------------------------

    #[test]
    fn ignore_unknown_top_level_fields() {
        let json = r#"{
            "ok": true,
            "result": [],
            "future_field": "some_value"
        }"#;
        let resp: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        assert!(resp.ok);
    }

    #[test]
    fn ignore_unknown_message_fields() {
        let json = r#"{
            "ok": true,
            "result": [{
                "update_id": 1,
                "message": {
                    "message_id": 10,
                    "date": 1720000000,
                    "chat": { "id": 1001, "type": "private" },
                    "text": "hi",
                    "entities": []
                }
            }]
        }"#;
        let resp: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        let msg = resp.result[0].message.as_ref().unwrap();
        assert_eq!(msg.text.as_deref(), Some("hi"));
    }
}
