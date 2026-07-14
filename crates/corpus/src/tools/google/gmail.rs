//! Read-only Gmail capability adapter.

use super::client::{GoogleApiClient, GoogleApiError};
use async_trait::async_trait;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use base64::Engine;
use fabric::google::MAX_GMAIL_BODY_BYTES;
use fabric::{
    ExternalIdentityId, ExternalScope, GmailMessage, GmailMessagePage, GmailMessageSummary,
    GmailQuery, PrincipalId, ProviderRecordRef,
};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait GmailCapability: Send + Sync {
    async fn search_messages(
        &self,
        principal: &PrincipalId,
        query: GmailQuery,
        cancel: &CancellationToken,
    ) -> Result<GmailMessagePage, GoogleApiError>;

    async fn important_unread(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        page_size: u16,
        cancel: &CancellationToken,
    ) -> Result<GmailMessagePage, GoogleApiError>;

    async fn read_message(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        message_id: &str,
        cancel: &CancellationToken,
    ) -> Result<GmailMessage, GoogleApiError>;
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
    ) -> Result<GmailMessageSummary, GoogleApiError> {
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
                ExternalScope::GmailReadonly,
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
        query: GmailQuery,
        cancel: &CancellationToken,
    ) -> Result<GmailMessagePage, GoogleApiError> {
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
                ExternalScope::GmailReadonly,
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
        let page = GmailMessagePage {
            account_id: query.account_id,
            messages,
            next_page_token: list.next_page_token,
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
    ) -> Result<GmailMessagePage, GoogleApiError> {
        self.search_messages(
            principal,
            GmailQuery {
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
    ) -> Result<GmailMessage, GoogleApiError> {
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
                ExternalScope::GmailReadonly,
                url,
                cancel,
            )
            .await?;
        let body_text = decode_body(raw.payload.as_ref())?;
        let message = GmailMessage {
            summary: normalize_summary(account, raw)?,
            body_text,
        };
        message
            .validate()
            .map_err(|_| GoogleApiError::MalformedResponse)?;
        Ok(message)
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
}

fn normalize_summary(
    account: ExternalIdentityId,
    raw: RawMessage,
) -> Result<GmailMessageSummary, GoogleApiError> {
    validate_provider_id(&raw.id)?;
    let source_timestamp_ms = raw
        .internal_date
        .as_deref()
        .and_then(|value| value.parse().ok())
        .ok_or(GoogleApiError::MalformedResponse)?;
    let subject = header(raw.payload.as_ref(), "Subject");
    let from = header(raw.payload.as_ref(), "From");
    let summary = GmailMessageSummary {
        source: ProviderRecordRef {
            account_id: account,
            provider_object_id: raw.id,
            fetched_at_ms: chrono::Utc::now().timestamp_millis(),
            source_timestamp_ms,
            etag_or_history: raw.history_id,
        },
        thread_id: raw.thread_id,
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
    if decoded.len() > MAX_GMAIL_BODY_BYTES {
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
