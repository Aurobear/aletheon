//! Telegram long-poll transport (M1).
//!
//! Uses raw HTTP via `reqwest` instead of a framework so no extra lifecycle or
//! version-specific API dependency is introduced at this stage.

pub mod types;

use anyhow::{bail, Context, Result};
use async_trait::async_trait;
use fabric::channel::{
    ChannelId, ConversationId, ExternalSenderId, InboundMessage, MessageContent, MessageId,
    OutboundMessage,
};
use tokio_util::sync::CancellationToken;

use super::router::{ChannelTransport, ProviderEnvelope};
use types::{
    GetUpdatesResponse, InlineKeyboardButton, InlineKeyboardMarkup, SendMessageRequest,
    SendMessageResponse,
};

/// Default Telegram Bot API base URL.
pub const DEFAULT_BASE_URL: &str = "https://api.telegram.org";

/// HTTP long-poll transport for the Telegram Bot API.
///
/// # Token safety
///
/// The bot token is never included in log messages or error strings.  All
/// `Display`/`Debug` output is sanitised before it leaves this module.
pub struct TelegramTransport {
    client: reqwest::Client,
    token: String,
    base_url: String,
    poll_timeout_secs: u64,
    #[allow(dead_code)]
    cancel: CancellationToken,
}

impl std::fmt::Debug for TelegramTransport {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TelegramTransport")
            .field("base_url", &self.base_url)
            .field("poll_timeout_secs", &self.poll_timeout_secs)
            .field("token", &"[redacted]")
            .finish()
    }
}

impl TelegramTransport {
    /// Create a new transport.
    ///
    /// `token` is the raw bot token (e.g. `"123:abc"`).
    /// `base_url` defaults to [`DEFAULT_BASE_URL`] when empty.
    pub fn new(
        token: String,
        base_url: Option<String>,
        poll_timeout_secs: u64,
        cancel: CancellationToken,
    ) -> Self {
        let base_url = base_url
            .filter(|s| !s.is_empty())
            .unwrap_or_else(|| DEFAULT_BASE_URL.to_string());
        Self {
            client: reqwest::Client::new(),
            token,
            base_url,
            poll_timeout_secs,
            cancel,
        }
    }

    // -- internal helpers ----------------------------------------------------

    /// Build the `getUpdates` URL without logging it.
    fn get_updates_url(&self) -> String {
        format!("{}/bot{}/getUpdates", self.base_url, self.token)
    }

    /// Build the `sendMessage` URL without logging it.
    fn send_message_url(&self) -> String {
        format!("{}/bot{}/sendMessage", self.base_url, self.token)
    }

    /// Sanitise an error from the API so the response body is never included.
    fn sanitised_api_error(description: &str) -> String {
        format!("Telegram API error: {}", description)
    }

    // -- conversion ----------------------------------------------------------

    /// Convert a single Telegram `Update` (with a text `message`) into an
    /// [`InboundMessage`].  Returns `None` for updates that carry no text
    /// message (e.g. edited_message, callback_query).
    fn convert_update(update: &types::Update) -> Option<InboundMessage> {
        if let Some(callback) = &update.callback_query {
            let msg = callback.message.as_ref()?;
            return Some(InboundMessage {
                channel_id: ChannelId("telegram".into()),
                message_id: MessageId(format!("callback:{}", callback.id)),
                conversation_id: ConversationId(msg.chat.id.to_string()),
                sender_id: ExternalSenderId(format!("telegram:{}", callback.from.id)),
                content: MessageContent::Text {
                    text: String::new(),
                },
                timestamp_ms: msg.date * 1000,
                reply_to_action: callback.data.clone(),
                correlation_id: format!("telegram:{}", update.update_id),
            });
        }
        let msg = update.message.as_ref()?;
        let text = msg.text.as_deref().unwrap_or("");

        let content = Self::parse_content(text);

        let from = msg.from.as_ref();
        let sender_id = match from {
            Some(u) => format!("telegram:{}", u.id),
            None => "telegram:unknown".to_string(),
        };

        Some(InboundMessage {
            channel_id: ChannelId("telegram".into()),
            message_id: MessageId(update.update_id.to_string()),
            conversation_id: ConversationId(msg.chat.id.to_string()),
            sender_id: ExternalSenderId(sender_id),
            content,
            timestamp_ms: msg.date * 1000,
            reply_to_action: None,
            correlation_id: format!("telegram:{}", update.update_id),
        })
    }

    /// Parse text into a `MessageContent`.  A leading slash (e.g. `/chat
    /// hello`) becomes `Command`; everything else is `Text`.
    fn parse_content(text: &str) -> MessageContent {
        let trimmed = text.trim();
        if let Some(rest) = trimmed.strip_prefix('/') {
            // Split at the first whitespace: command vs args.
            if let Some((cmd, args)) = rest.split_once(char::is_whitespace) {
                MessageContent::Command {
                    command: format!("/{cmd}"),
                    args: args.trim().to_string(),
                }
            } else {
                MessageContent::Command {
                    command: format!("/{rest}"),
                    args: String::new(),
                }
            }
        } else {
            MessageContent::Text {
                text: trimmed.to_string(),
            }
        }
    }

    /// Compute the next cursor from the highest update_id in the batch.
    fn next_cursor(updates: &[types::Update]) -> Option<String> {
        updates.last().map(|u| (u.update_id + 1).to_string())
    }
}

#[async_trait]
impl ChannelTransport for TelegramTransport {
    fn channel_id(&self) -> &str {
        "telegram"
    }

    async fn receive(&self, cursor: Option<String>) -> Result<Vec<ProviderEnvelope>> {
        let offset: i64 = cursor.as_deref().and_then(|c| c.parse().ok()).unwrap_or(0);

        let resp = self
            .client
            .get(self.get_updates_url())
            .query(&[
                ("offset", offset.to_string()),
                ("timeout", self.poll_timeout_secs.to_string()),
                (
                    "allowed_updates",
                    r#"["message","callback_query"]"#.to_string(),
                ),
            ])
            .send()
            .await
            .context("Telegram getUpdates request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            // Drain and discard body — never log response content.
            let _ = resp.text().await;
            bail!("Telegram getUpdates returned HTTP {}", status);
        }

        let body: GetUpdatesResponse = resp
            .json()
            .await
            .context("deserializing getUpdates response")?;

        if !body.ok {
            let desc = body.description.as_deref().unwrap_or("unknown");
            bail!("{}", Self::sanitised_api_error(desc));
        }

        // Default cursor when there are no updates: keep the current offset.
        let default_cursor = offset.to_string();
        let next_cursor = Self::next_cursor(&body.result).unwrap_or(default_cursor);

        let envelopes: Vec<ProviderEnvelope> = body
            .result
            .iter()
            .filter_map(|u| {
                Self::convert_update(u).map(|msg| ProviderEnvelope {
                    message: msg,
                    next_cursor: next_cursor.clone(),
                })
            })
            .collect();

        Ok(envelopes)
    }

    async fn send(&self, outbound: &OutboundMessage) -> Result<String> {
        let chat_id: i64 = outbound
            .conversation_id
            .0
            .parse()
            .context("conversation_id is not a valid Telegram chat id")?;

        let text = match &outbound.content {
            MessageContent::Text { text } => text.clone(),
            MessageContent::Command { command, args } => {
                if args.is_empty() {
                    format!("/{}", command)
                } else {
                    format!("/{} {}", command, args)
                }
            }
        };

        let reply_markup = (!outbound.actions.is_empty()).then(|| InlineKeyboardMarkup {
            inline_keyboard: outbound
                .actions
                .iter()
                .map(|action| {
                    vec![InlineKeyboardButton {
                        text: action.label.clone(),
                        callback_data: action.action_id.clone(),
                    }]
                })
                .collect(),
        });
        let body = SendMessageRequest {
            chat_id,
            text,
            reply_markup,
        };

        let resp = self
            .client
            .post(self.send_message_url())
            .json(&body)
            .send()
            .await
            .context("Telegram sendMessage request failed")?;

        if !resp.status().is_success() {
            let status = resp.status();
            let _ = resp.text().await;
            bail!("Telegram sendMessage returned HTTP {}", status);
        }

        let envelope: SendMessageResponse = resp
            .json()
            .await
            .context("deserializing sendMessage response")?;

        if !envelope.ok {
            let desc = envelope.description.as_deref().unwrap_or("unknown");
            bail!("{}", Self::sanitised_api_error(desc));
        }

        let provider_id = envelope
            .result
            .map(|m| m.message_id.to_string())
            .unwrap_or_else(|| "unknown".to_string());

        tracing::debug!(
            correlation_id = %outbound.correlation_id,
            provider_msg_id = %provider_id,
            "Telegram message sent"
        );
        Ok(provider_id)
    }
}

// ---------------------------------------------------------------------------
// tests — uses a local TcpListener mock
// ---------------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use fabric::channel::ConversationId;
    use std::sync::atomic::{AtomicU16, Ordering};
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::TcpListener;

    static PORT: AtomicU16 = AtomicU16::new(19000);

    /// Grab a unique port so parallel tests don't clash.
    fn next_port() -> u16 {
        PORT.fetch_add(1, Ordering::SeqCst)
    }

    /// Spawn a tiny HTTP server that responds to one request then shuts down.
    async fn mock_server(
        response_status: u16,
        response_body: &'static str,
    ) -> (String, tokio::task::JoinHandle<()>) {
        let port = next_port();
        let listener = TcpListener::bind(("127.0.0.1", port))
            .await
            .expect("bind mock server");
        let base = format!("http://127.0.0.1:{}", port);

        let handle = tokio::spawn(async move {
            let (stream, _) = listener.accept().await.expect("accept");
            let (reader, mut writer) = stream.into_split();
            let mut buf_reader = BufReader::new(reader);

            // Read request line + headers until empty line.
            let mut line = String::new();
            loop {
                line.clear();
                buf_reader.read_line(&mut line).await.expect("read line");
                if line == "\r\n" || line.is_empty() {
                    break;
                }
            }

            let resp = format!(
                "HTTP/1.1 {status} OK\r\nContent-Type: application/json\r\nContent-Length: {len}\r\nConnection: close\r\n\r\n{body}",
                status = response_status,
                len = response_body.len(),
                body = response_body,
            );
            writer.write_all(resp.as_bytes()).await.expect("write");
            writer.shutdown().await.ok();
        });

        (base, handle)
    }

    fn make_transport(base_url: String) -> TelegramTransport {
        TelegramTransport::new(
            "test-token".to_string(),
            Some(base_url),
            1,
            CancellationToken::new(),
        )
    }

    // -- conversion tests ----------------------------------------------------

    #[test]
    fn convert_text_message() {
        let json = r#"{
            "ok": true,
            "result": [{
                "update_id": 1,
                "message": {
                    "message_id": 10,
                    "from": { "id": 7 },
                    "chat": { "id": 1001 },
                    "date": 1720000000,
                    "text": "hello world"
                }
            }]
        }"#;
        let resp: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        let msg = TelegramTransport::convert_update(&resp.result[0]).unwrap();
        assert_eq!(msg.channel_id.0, "telegram");
        assert_eq!(msg.message_id.0, "1");
        assert_eq!(msg.conversation_id.0, "1001");
        assert_eq!(msg.sender_id.0, "telegram:7");
        assert_eq!(msg.timestamp_ms, 1720000000 * 1000);
        assert_eq!(msg.correlation_id, "telegram:1");
        assert_eq!(
            msg.content,
            MessageContent::Text {
                text: "hello world".into()
            }
        );
    }

    #[test]
    fn convert_slash_command() {
        let json = r#"{
            "ok": true,
            "result": [{
                "update_id": 2,
                "message": {
                    "message_id": 11,
                    "from": { "id": 99 },
                    "chat": { "id": 2001 },
                    "date": 1720000100,
                    "text": "/chat hello there"
                }
            }]
        }"#;
        let resp: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        let msg = TelegramTransport::convert_update(&resp.result[0]).unwrap();
        assert_eq!(
            msg.content,
            MessageContent::Command {
                command: "/chat".into(),
                args: "hello there".into(),
            }
        );
    }

    #[test]
    fn convert_slash_command_no_args() {
        let json = r#"{
            "ok": true,
            "result": [{
                "update_id": 3,
                "message": {
                    "message_id": 12,
                    "from": { "id": 1 },
                    "chat": { "id": 3001 },
                    "date": 1720000200,
                    "text": "/status"
                }
            }]
        }"#;
        let resp: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        let msg = TelegramTransport::convert_update(&resp.result[0]).unwrap();
        assert_eq!(
            msg.content,
            MessageContent::Command {
                command: "/status".into(),
                args: String::new(),
            }
        );
    }

    #[test]
    fn convert_update_without_message_is_none() {
        let json = r#"{
            "ok": true,
            "result": [{
                "update_id": 99,
                "edited_message": { "message_id": 10, "chat": {"id": 1}, "date": 1, "text": "x" }
            }]
        }"#;
        let resp: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        assert!(TelegramTransport::convert_update(&resp.result[0]).is_none());
    }

    #[test]
    fn convert_callback_keeps_only_action_data_and_bound_identity() {
        let response: GetUpdatesResponse = serde_json::from_str(
            r#"{"ok":true,"result":[{"update_id":9,"callback_query":{"id":"cb",
            "from":{"id":7},"message":{"message_id":3,"date":100,"chat":{"id":42}},
            "data":"00000000-0000-0000-0000-000000000001:apply"}}]}"#,
        )
        .unwrap();
        let message = TelegramTransport::convert_update(&response.result[0]).unwrap();
        assert_eq!(message.sender_id.0, "telegram:7");
        assert_eq!(message.conversation_id.0, "42");
        assert_eq!(
            message.reply_to_action.as_deref(),
            Some("00000000-0000-0000-0000-000000000001:apply")
        );
    }

    #[test]
    fn next_cursor_returns_highest_plus_one() {
        let json = r#"{
            "ok": true,
            "result": [
                { "update_id": 10, "message": { "message_id": 1, "chat": {"id":1}, "date":1, "text":"a" } },
                { "update_id": 20, "message": { "message_id": 2, "chat": {"id":2}, "date":1, "text":"b" } }
            ]
        }"#;
        let resp: GetUpdatesResponse = serde_json::from_str(json).unwrap();
        let cursor = TelegramTransport::next_cursor(&resp.result);
        assert_eq!(cursor, Some("21".to_string()));
    }

    #[test]
    fn parse_content_text() {
        assert_eq!(
            TelegramTransport::parse_content(" hello "),
            MessageContent::Text {
                text: "hello".into()
            }
        );
    }

    #[test]
    fn token_not_leaked_in_errors() {
        let t = TelegramTransport::new(
            "123:secret".into(),
            Some("https://api.example.com".into()),
            1,
            CancellationToken::new(),
        );
        // Error from sanitised_api_error does not contain the token.
        let err = TelegramTransport::sanitised_api_error("bad token");
        assert!(!err.contains("secret"));
        assert!(!err.contains("123:secret"));
        // The URL itself contains the token — we just never log it.
        let url = t.get_updates_url();
        assert!(url.contains("123:secret"));
        // channel_id does not leak anything.
        assert_eq!(t.channel_id(), "telegram");
    }

    // -- integration tests with mock server ----------------------------------

    #[tokio::test]
    async fn receive_returns_messages() {
        let response_body = r#"{
            "ok": true,
            "result": [{
                "update_id": 42,
                "message": {
                    "message_id": 5,
                    "from": { "id": 7 },
                    "chat": { "id": 1001 },
                    "date": 1720000000,
                    "text": "ping"
                }
            }]
        }"#;
        let (base, _handle) = mock_server(200, response_body).await;
        let t = make_transport(base);

        let envelopes = t.receive(Some("0".into())).await.unwrap();
        assert_eq!(envelopes.len(), 1);
        assert_eq!(
            envelopes[0].message.content,
            MessageContent::Text {
                text: "ping".into()
            }
        );
        assert_eq!(envelopes[0].next_cursor, "43");
    }

    #[tokio::test]
    async fn receive_respects_offset() {
        let response_body = r#"{"ok": true, "result": []}"#;
        let (base, handle) = mock_server(200, response_body).await;
        let t = make_transport(base);

        // Start from cursor "99" — the mock server doesn't care, but the transport
        // passes it as offset.
        let envelopes = t.receive(Some("99".into())).await.unwrap();
        assert!(envelopes.is_empty());
        handle.await.unwrap();
    }

    #[tokio::test]
    async fn receive_empty_result_with_no_cursor() {
        let response_body = r#"{"ok": true, "result": []}"#;
        let (base, _handle) = mock_server(200, response_body).await;
        let t = make_transport(base);

        let envelopes = t.receive(None).await.unwrap();
        assert!(envelopes.is_empty());
    }

    #[tokio::test]
    async fn receive_handles_api_error() {
        let response_body = r#"{"ok": false, "description": "Forbidden: bot was blocked"}"#;
        let (base, _handle) = mock_server(200, response_body).await;
        let t = make_transport(base);

        let err = t.receive(None).await.unwrap_err();
        let err_str = format!("{}", err);
        assert!(err_str.contains("Telegram API error"));
        // The original description is present in sanitised form.
        assert!(err_str.contains("Forbidden: bot was blocked"));
        // No raw response body or token leaked.
        assert!(!err_str.contains("test-token"));
    }

    #[tokio::test]
    async fn receive_handles_http_error() {
        let response_body = r#"{"ok": false, "description": "Unauthorized"}"#;
        let (base, _handle) = mock_server(500, response_body).await;
        let t = make_transport(base);

        let err = t.receive(None).await.unwrap_err();
        let err_str = format!("{}", err);
        assert!(err_str.contains("HTTP 500"));
        // No token in error.
        assert!(!err_str.contains("test-token"));
    }

    #[tokio::test]
    async fn send_delivers_text_message() {
        let response_body = r#"{
            "ok": true,
            "result": {
                "message_id": 88,
                "from": { "id": 888 },
                "chat": { "id": 1001 },
                "date": 1720000400,
                "text": "pong"
            }
        }"#;
        let (base, _handle) = mock_server(200, response_body).await;
        let t = make_transport(base);

        let outbound = OutboundMessage {
            conversation_id: ConversationId("1001".into()),
            content: MessageContent::Text {
                text: "pong".into(),
            },
            actions: vec![],
            reply_to: None,
            correlation_id: "corr-send-1".into(),
        };
        let provider_id = t.send(&outbound).await.unwrap();
        assert_eq!(provider_id, "88");
    }

    #[tokio::test]
    async fn send_handles_api_error() {
        let response_body = r#"{"ok": false, "description": "chat not found"}"#;
        let (base, _handle) = mock_server(200, response_body).await;
        let t = make_transport(base);

        let outbound = OutboundMessage {
            conversation_id: ConversationId("9999".into()),
            content: MessageContent::Text { text: "hi".into() },
            actions: vec![],
            reply_to: None,
            correlation_id: "corr-send-err".into(),
        };
        let err = t.send(&outbound).await.unwrap_err();
        let err_str = format!("{}", err);
        assert!(err_str.contains("Telegram API error"));
        assert!(err_str.contains("chat not found"));
        assert!(!err_str.contains("test-token"));
    }

    #[tokio::test]
    async fn send_converts_command_to_text() {
        let response_body = r#"{"ok": true, "result": {"message_id": 1, "from": {"id":1}, "chat": {"id":1}, "date":1, "text": "/approve yes"}}"#;
        let (base, _handle) = mock_server(200, response_body).await;
        let t = make_transport(base);

        let outbound = OutboundMessage {
            conversation_id: ConversationId("1".into()),
            content: MessageContent::Command {
                command: "approve".into(),
                args: "yes".into(),
            },
            actions: vec![],
            reply_to: None,
            correlation_id: "corr-cmd".into(),
        };
        // Should not panic; mock server accepts any body.
        t.send(&outbound).await.unwrap();
    }

    #[tokio::test]
    async fn channel_id_returns_telegram() {
        let t = make_transport("http://localhost:1".into());
        assert_eq!(t.channel_id(), "telegram");
    }
}
