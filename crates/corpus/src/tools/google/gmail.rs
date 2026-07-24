//! Read-only Gmail capability adapter.

use super::client::{GoogleApiClient, GoogleApiError};
use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use fabric::external_source::MAX_MAIL_BODY_BYTES;
use fabric::{
    ExternalCapabilityId, ExternalIdentityId, ExternalRecordRef, MailMessage, MailMessagePage,
    MailMessageSummary, MailQuery, OpaqueCursor, OpaqueProviderObjectId, PrincipalId,
};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait GmailCapability: Send + Sync {
    async fn search_messages(
        &self,
        principal: &PrincipalId,
        query: MailQuery,
        cancel: &CancellationToken,
    ) -> Result<MailMessagePage, GoogleApiError>;

    async fn important_unread(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        page_size: u16,
        cancel: &CancellationToken,
    ) -> Result<MailMessagePage, GoogleApiError>;

    async fn read_message(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        message_id: &str,
        cancel: &CancellationToken,
    ) -> Result<MailMessage, GoogleApiError>;
}

/// Bounded full-message view used by the authenticated Gmail channel. This is
/// deliberately separate from [`GmailCapability`] so read-only model tools do
/// not expose raw authentication headers or attachment identifiers.
#[async_trait]
pub trait GmailIngressCapability: Send + Sync {
    async fn read_ingress_message(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        message_id: &str,
        cancel: &CancellationToken,
    ) -> Result<GmailIngressMessage, GoogleApiError>;

    async fn read_ingress_attachment(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        message_id: &str,
        attachment_id: &str,
        max_decoded_bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, GoogleApiError>;
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailIngressHeader {
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailIngressPart {
    pub part_id: String,
    pub mime_type: String,
    pub filename: Option<String>,
    pub declared_size: Option<u64>,
    pub inline_body: Option<Vec<u8>>,
    pub attachment_id: Option<String>,
    pub parts: Vec<GmailIngressPart>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GmailIngressMessage {
    pub account_id: ExternalIdentityId,
    pub message_id: String,
    pub thread_id: String,
    pub source_timestamp_ms: i64,
    pub headers: Vec<GmailIngressHeader>,
    pub root: GmailIngressPart,
}

#[derive(Debug, Clone)]
pub struct GoogleGmailAdapter {
    pub(crate) client: GoogleApiClient,
}

impl GoogleGmailAdapter {
    pub fn new(client: GoogleApiClient) -> Self {
        Self { client }
    }

    pub(crate) async fn metadata(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        id: &str,
        cancel: &CancellationToken,
    ) -> Result<MailMessageSummary, GoogleApiError> {
        validate_provider_id(id)?;
        let mut url = reqwest::Url::parse(&format!(
            "{}/users/me/messages/{}",
            self.client.endpoints().gmail_base.trim_end_matches('/'),
            id
        ))
        .map_err(|_| GoogleApiError::InvalidRequest)?;
        url.query_pairs_mut().append_pair("format", "metadata");
        let raw: RawMessage = self
            .client
            .get_json(
                principal,
                account,
                ExternalCapabilityId::new("mail.read").unwrap(),
                url,
                cancel,
            )
            .await?;
        normalize_summary(account, raw)
    }
}

#[async_trait]
impl GmailCapability for GoogleGmailAdapter {
    async fn search_messages(
        &self,
        principal: &PrincipalId,
        query: MailQuery,
        cancel: &CancellationToken,
    ) -> Result<MailMessagePage, GoogleApiError> {
        query
            .validate()
            .map_err(|_| GoogleApiError::InvalidRequest)?;
        let mut url = reqwest::Url::parse(&format!(
            "{}/users/me/messages",
            self.client.endpoints().gmail_base.trim_end_matches('/')
        ))
        .map_err(|_| GoogleApiError::InvalidRequest)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("q", &query.query)
                .append_pair("maxResults", &query.page_size.to_string());
            if let Some(token) = &query.page_token {
                pairs.append_pair("pageToken", token);
            }
        }
        let list: RawMessageList = self
            .client
            .get_json(
                principal,
                query.account_id,
                ExternalCapabilityId::new("mail.read").unwrap(),
                url,
                cancel,
            )
            .await?;
        if list.messages.len() > usize::from(query.page_size) {
            return Err(GoogleApiError::MalformedResponse);
        }
        let mut messages = Vec::with_capacity(list.messages.len());
        for item in list.messages {
            messages.push(
                self.metadata(principal, query.account_id, &item.id, cancel)
                    .await?,
            );
        }
        let page = MailMessagePage {
            account_id: query.account_id,
            messages,
            next_page_token: list
                .next_page_token
                .map(OpaqueCursor::new)
                .transpose()
                .map_err(|_| GoogleApiError::MalformedResponse)?,
        };
        page.validate()
            .map_err(|_| GoogleApiError::MalformedResponse)?;
        Ok(page)
    }

    async fn important_unread(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        page_size: u16,
        cancel: &CancellationToken,
    ) -> Result<MailMessagePage, GoogleApiError> {
        self.search_messages(
            principal,
            MailQuery {
                account_id: account,
                query: "is:unread is:important".into(),
                page_size,
                page_token: None,
            },
            cancel,
        )
        .await
    }

    async fn read_message(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        message_id: &str,
        cancel: &CancellationToken,
    ) -> Result<MailMessage, GoogleApiError> {
        validate_provider_id(message_id)?;
        let mut url = reqwest::Url::parse(&format!(
            "{}/users/me/messages/{}",
            self.client.endpoints().gmail_base.trim_end_matches('/'),
            message_id
        ))
        .map_err(|_| GoogleApiError::InvalidRequest)?;
        url.query_pairs_mut().append_pair("format", "full");
        let raw: RawMessage = self
            .client
            .get_json(
                principal,
                account,
                ExternalCapabilityId::new("mail.read").unwrap(),
                url,
                cancel,
            )
            .await?;
        let body_text = decode_body(raw.payload.as_ref())?;
        let message = MailMessage {
            summary: normalize_summary(account, raw)?,
            body_text,
        };
        message
            .validate()
            .map_err(|_| GoogleApiError::MalformedResponse)?;
        Ok(message)
    }
}

#[async_trait]
impl GmailIngressCapability for GoogleGmailAdapter {
    async fn read_ingress_message(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        message_id: &str,
        cancel: &CancellationToken,
    ) -> Result<GmailIngressMessage, GoogleApiError> {
        validate_provider_id(message_id)?;
        let mut url = reqwest::Url::parse(&format!(
            "{}/users/me/messages/{}",
            self.client.endpoints().gmail_base.trim_end_matches('/'),
            message_id
        ))
        .map_err(|_| GoogleApiError::InvalidRequest)?;
        url.query_pairs_mut().append_pair("format", "full");
        let raw: RawMessage = self
            .client
            .get_json(
                principal,
                account,
                ExternalCapabilityId::new("mail.read").unwrap(),
                url,
                cancel,
            )
            .await?;
        if raw.id != message_id {
            return Err(GoogleApiError::MalformedResponse);
        }
        let source_timestamp_ms = raw
            .internal_date
            .as_deref()
            .and_then(|value| value.parse().ok())
            .ok_or(GoogleApiError::MalformedResponse)?;
        let payload = raw.payload.ok_or(GoogleApiError::MalformedResponse)?;
        let headers = payload
            .headers
            .iter()
            .map(|header| GmailIngressHeader {
                name: header.name.clone(),
                value: header.value.clone(),
            })
            .collect();
        Ok(GmailIngressMessage {
            account_id: account,
            message_id: raw.id,
            thread_id: raw.thread_id,
            source_timestamp_ms,
            headers,
            root: normalize_ingress_part(payload, 0)?,
        })
    }

    async fn read_ingress_attachment(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        message_id: &str,
        attachment_id: &str,
        max_decoded_bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, GoogleApiError> {
        validate_provider_id(message_id)?;
        validate_provider_id(attachment_id)?;
        if max_decoded_bytes == 0 || max_decoded_bytes > 8 * 1_048_576 {
            return Err(GoogleApiError::InvalidRequest);
        }
        let url = reqwest::Url::parse(&format!(
            "{}/users/me/messages/{}/attachments/{}",
            self.client.endpoints().gmail_base.trim_end_matches('/'),
            message_id,
            attachment_id
        ))
        .map_err(|_| GoogleApiError::InvalidRequest)?;
        let encoded_cap = max_decoded_bytes
            .checked_mul(4)
            .and_then(|value| value.checked_div(3))
            .and_then(|value| value.checked_add(16 * 1024))
            .ok_or(GoogleApiError::InvalidRequest)?
            .min(16 * 1_048_576);
        let bytes = self
            .client
            .get_bounded_bytes(
                principal,
                account,
                ExternalCapabilityId::new("mail.read").unwrap(),
                url,
                encoded_cap,
                cancel,
            )
            .await?;
        let raw: RawAttachment =
            serde_json::from_slice(&bytes).map_err(|_| GoogleApiError::MalformedResponse)?;
        let decoded = URL_SAFE_NO_PAD
            .decode(raw.data)
            .map_err(|_| GoogleApiError::MalformedResponse)?;
        if decoded.len() > max_decoded_bytes || raw.size.is_some_and(|size| size != decoded.len()) {
            return Err(GoogleApiError::ResponseTooLarge);
        }
        Ok(decoded)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMessageList {
    #[serde(default)]
    messages: Vec<RawMessageId>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
struct RawMessageId {
    id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawMessage {
    id: String,
    thread_id: String,
    #[serde(default)]
    label_ids: Vec<String>,
    #[serde(default)]
    snippet: String,
    internal_date: Option<String>,
    history_id: Option<String>,
    payload: Option<RawPayload>,
}

#[derive(Deserialize)]
struct RawPayload {
    #[serde(default, rename = "partId")]
    part_id: String,
    #[serde(default)]
    filename: String,
    #[serde(default)]
    headers: Vec<RawHeader>,
    body: Option<RawBody>,
    #[serde(default)]
    parts: Vec<RawPayload>,
    #[serde(rename = "mimeType")]
    mime_type: Option<String>,
}

#[derive(Deserialize)]
struct RawHeader {
    name: String,
    value: String,
}

#[derive(Deserialize)]
struct RawBody {
    data: Option<String>,
    #[serde(rename = "attachmentId")]
    attachment_id: Option<String>,
    size: Option<usize>,
}

#[derive(Deserialize)]
struct RawAttachment {
    data: String,
    size: Option<usize>,
}

fn normalize_ingress_part(
    payload: RawPayload,
    depth: usize,
) -> Result<GmailIngressPart, GoogleApiError> {
    if depth > 16 || payload.parts.len() > 200 {
        return Err(GoogleApiError::ResponseTooLarge);
    }
    let inline_body = payload
        .body
        .as_ref()
        .and_then(|body| body.data.as_deref())
        .map(|data| {
            URL_SAFE_NO_PAD
                .decode(data)
                .map_err(|_| GoogleApiError::MalformedResponse)
        })
        .transpose()?;
    if inline_body
        .as_ref()
        .is_some_and(|body| body.len() > 256 * 1_024)
    {
        return Err(GoogleApiError::ResponseTooLarge);
    }
    let declared_size = payload
        .body
        .as_ref()
        .and_then(|body| body.size)
        .map(|value| value as u64);
    let attachment_id = payload
        .body
        .as_ref()
        .and_then(|body| body.attachment_id.clone());
    let parts = payload
        .parts
        .into_iter()
        .map(|part| normalize_ingress_part(part, depth + 1))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(GmailIngressPart {
        part_id: payload.part_id,
        mime_type: payload
            .mime_type
            .unwrap_or_else(|| "application/octet-stream".into()),
        filename: (!payload.filename.is_empty()).then_some(payload.filename),
        declared_size,
        inline_body,
        attachment_id,
        parts,
    })
}

fn normalize_summary(
    account: ExternalIdentityId,
    raw: RawMessage,
) -> Result<MailMessageSummary, GoogleApiError> {
    validate_provider_id(&raw.id)?;
    let source_timestamp_ms = raw
        .internal_date
        .as_deref()
        .and_then(|value| value.parse().ok())
        .ok_or(GoogleApiError::MalformedResponse)?;
    let subject = header(raw.payload.as_ref(), "Subject");
    let from = header(raw.payload.as_ref(), "From");
    let summary = MailMessageSummary {
        source: ExternalRecordRef {
            account_id: account,
            provider_object_id: OpaqueProviderObjectId::new(raw.id)
                .map_err(|_| GoogleApiError::MalformedResponse)?,
            fetched_at_ms: chrono::Utc::now().timestamp_millis(),
            source_timestamp_ms,
            etag_or_history: raw
                .history_id
                .map(OpaqueCursor::new)
                .transpose()
                .map_err(|_| GoogleApiError::MalformedResponse)?,
        },
        thread_id: OpaqueProviderObjectId::new(raw.thread_id)
            .map_err(|_| GoogleApiError::MalformedResponse)?,
        subject,
        from,
        snippet: raw.snippet,
        unread: raw.label_ids.iter().any(|label| label == "UNREAD"),
        important: raw.label_ids.iter().any(|label| label == "IMPORTANT"),
    };
    summary
        .validate()
        .map_err(|_| GoogleApiError::MalformedResponse)?;
    Ok(summary)
}

fn header(payload: Option<&RawPayload>, name: &str) -> String {
    payload
        .into_iter()
        .flat_map(|payload| payload.headers.iter())
        .find(|header| header.name.eq_ignore_ascii_case(name))
        .map(|header| header.value.clone())
        .unwrap_or_default()
}

fn decode_body(payload: Option<&RawPayload>) -> Result<String, GoogleApiError> {
    let payload = payload.ok_or(GoogleApiError::MalformedResponse)?;
    let candidate = if payload.mime_type.as_deref() == Some("text/plain") {
        payload.body.as_ref().and_then(|body| body.data.as_deref())
    } else {
        payload.parts.iter().find_map(|part| {
            (part.mime_type.as_deref() == Some("text/plain"))
                .then(|| part.body.as_ref().and_then(|body| body.data.as_deref()))
                .flatten()
        })
    };
    let Some(data) = candidate else {
        return Ok(String::new());
    };
    let decoded = URL_SAFE_NO_PAD
        .decode(data)
        .map_err(|_| GoogleApiError::MalformedResponse)?;
    if decoded.len() > MAX_MAIL_BODY_BYTES {
        return Err(GoogleApiError::ResponseTooLarge);
    }
    String::from_utf8(decoded).map_err(|_| GoogleApiError::MalformedResponse)
}

fn validate_provider_id(id: &str) -> Result<(), GoogleApiError> {
    if id.is_empty()
        || id.len() > 1_024
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Err(GoogleApiError::InvalidRequest)
    } else {
        Ok(())
    }
}
