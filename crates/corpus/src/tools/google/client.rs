//! Shared bounded Google REST client.

use async_trait::async_trait;
use fabric::{ExternalIdentityId, ExternalScope, PrincipalId};
use futures::StreamExt;
use serde::de::DeserializeOwned;
use std::fmt;
use std::sync::Arc;
use std::time::Duration;
use tokio_util::sync::CancellationToken;

pub const MAX_GOOGLE_RESPONSE_BYTES: usize = 1_048_576;

#[derive(Clone, PartialEq, Eq)]
pub struct GoogleAccessToken(String);

impl GoogleAccessToken {
    pub fn new(value: String) -> Result<Self, GoogleApiError> {
        if value.is_empty() || value.len() > 16 * 1024 {
            return Err(GoogleApiError::CredentialUnavailable);
        }
        Ok(Self(value))
    }

    fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for GoogleAccessToken {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("GoogleAccessToken([REDACTED])")
    }
}

#[async_trait]
pub trait GoogleCredentialSource: Send + Sync {
    /// This boundary must authenticate principal ownership and an active exact
    /// read grant before returning credential material.
    async fn access_token(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        required_scope: ExternalScope,
    ) -> Result<GoogleAccessToken, GoogleApiError>;

    async fn refresh_access_token(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        required_scope: ExternalScope,
    ) -> Result<GoogleAccessToken, GoogleApiError>;
}

#[derive(Debug, Clone)]
pub struct GoogleApiEndpoints {
    pub gmail_base: String,
    pub calendar_base: String,
    pub drive_base: String,
}

impl Default for GoogleApiEndpoints {
    fn default() -> Self {
        Self {
            gmail_base: "https://gmail.googleapis.com/gmail/v1".into(),
            calendar_base: "https://www.googleapis.com/calendar/v3".into(),
            drive_base: "https://www.googleapis.com/drive/v3".into(),
        }
    }
}

#[derive(Clone)]
pub struct GoogleApiClient {
    client: reqwest::Client,
    credentials: Arc<dyn GoogleCredentialSource>,
    endpoints: GoogleApiEndpoints,
}

impl fmt::Debug for GoogleApiClient {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GoogleApiClient")
            .field("endpoints", &self.endpoints)
            .field("credentials", &"[REDACTED]")
            .finish()
    }
}

impl GoogleApiClient {
    pub fn new(
        credentials: Arc<dyn GoogleCredentialSource>,
        endpoints: GoogleApiEndpoints,
    ) -> Result<Self, GoogleApiError> {
        let client = reqwest::Client::builder()
            .connect_timeout(Duration::from_secs(10))
            .timeout(Duration::from_secs(30))
            .build()
            .map_err(|_| GoogleApiError::ProviderUnavailable)?;
        Ok(Self {
            client,
            credentials,
            endpoints,
        })
    }

    pub fn endpoints(&self) -> &GoogleApiEndpoints {
        &self.endpoints
    }

    pub(crate) async fn get_json<T: DeserializeOwned>(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        required_scope: ExternalScope,
        url: reqwest::Url,
        cancel: &CancellationToken,
    ) -> Result<T, GoogleApiError> {
        let mut token = self
            .credentials
            .access_token(principal, account, required_scope)
            .await?;
        let mut refreshed = false;
        let mut rate_retried = false;
        loop {
            let response = tokio::select! {
                _ = cancel.cancelled() => return Err(GoogleApiError::Cancelled),
                response = self.client.get(url.clone()).bearer_auth(token.expose()).send() => {
                    response.map_err(|_| GoogleApiError::ProviderUnavailable)?
                }
            };
            match response.status().as_u16() {
                200..=299 => return decode_bounded(response, cancel).await,
                401 if !refreshed => {
                    token = self
                        .credentials
                        .refresh_access_token(principal, account, required_scope)
                        .await?;
                    refreshed = true;
                }
                401 => return Err(GoogleApiError::ReauthorizationRequired),
                403 => return Err(GoogleApiError::ScopeDenied),
                404 | 410 => return Err(GoogleApiError::CursorExpired),
                429 if !rate_retried => {
                    rate_retried = true;
                    let delay = retry_after(&response);
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(GoogleApiError::Cancelled),
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
                429 => return Err(GoogleApiError::RateLimited),
                _ => return Err(GoogleApiError::ProviderUnavailable),
            }
        }
    }

    pub(crate) async fn get_bounded_bytes(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        required_scope: ExternalScope,
        url: reqwest::Url,
        max_bytes: usize,
        cancel: &CancellationToken,
    ) -> Result<Vec<u8>, GoogleApiError> {
        if max_bytes == 0 || max_bytes > 16 * 1_048_576 {
            return Err(GoogleApiError::InvalidRequest);
        }
        let mut token = self
            .credentials
            .access_token(principal, account, required_scope)
            .await?;
        let mut refreshed = false;
        let mut rate_retried = false;
        loop {
            let response = tokio::select! {
                _ = cancel.cancelled() => return Err(GoogleApiError::Cancelled),
                response = self.client.get(url.clone()).bearer_auth(token.expose()).send() => {
                    response.map_err(|_| GoogleApiError::ProviderUnavailable)?
                }
            };
            match response.status().as_u16() {
                200..=299 => {
                    if response
                        .content_length()
                        .is_some_and(|length| length > max_bytes as u64)
                    {
                        return Err(GoogleApiError::ResponseTooLarge);
                    }
                    let mut bytes = Vec::new();
                    let mut stream = response.bytes_stream();
                    while let Some(chunk) = tokio::select! {
                        _ = cancel.cancelled() => return Err(GoogleApiError::Cancelled),
                        chunk = stream.next() => chunk,
                    } {
                        let chunk = chunk.map_err(|_| GoogleApiError::ProviderUnavailable)?;
                        if bytes.len().saturating_add(chunk.len()) > max_bytes {
                            return Err(GoogleApiError::ResponseTooLarge);
                        }
                        bytes.extend_from_slice(&chunk);
                    }
                    return Ok(bytes);
                }
                401 if !refreshed => {
                    token = self
                        .credentials
                        .refresh_access_token(principal, account, required_scope)
                        .await?;
                    refreshed = true;
                }
                401 => return Err(GoogleApiError::ReauthorizationRequired),
                403 => return Err(GoogleApiError::ScopeDenied),
                404 | 410 => return Err(GoogleApiError::CursorExpired),
                429 if !rate_retried => {
                    rate_retried = true;
                    let delay = retry_after(&response);
                    tokio::select! {
                        _ = cancel.cancelled() => return Err(GoogleApiError::Cancelled),
                        _ = tokio::time::sleep(delay) => {}
                    }
                }
                429 => return Err(GoogleApiError::RateLimited),
                _ => return Err(GoogleApiError::ProviderUnavailable),
            }
        }
    }
}

fn retry_after(response: &reqwest::Response) -> Duration {
    response
        .headers()
        .get(reqwest::header::RETRY_AFTER)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .map(|seconds| Duration::from_secs(seconds.min(5)))
        .unwrap_or(Duration::from_millis(100))
}

async fn decode_bounded<T: DeserializeOwned>(
    response: reqwest::Response,
    cancel: &CancellationToken,
) -> Result<T, GoogleApiError> {
    if response
        .content_length()
        .is_some_and(|length| length > MAX_GOOGLE_RESPONSE_BYTES as u64)
    {
        return Err(GoogleApiError::ResponseTooLarge);
    }
    let mut bytes = Vec::new();
    let mut stream = response.bytes_stream();
    loop {
        let chunk = tokio::select! {
            _ = cancel.cancelled() => return Err(GoogleApiError::Cancelled),
            chunk = stream.next() => chunk,
        };
        let Some(chunk) = chunk else { break };
        let chunk = chunk.map_err(|_| GoogleApiError::ProviderUnavailable)?;
        if bytes.len().saturating_add(chunk.len()) > MAX_GOOGLE_RESPONSE_BYTES {
            return Err(GoogleApiError::ResponseTooLarge);
        }
        bytes.extend_from_slice(&chunk);
    }
    serde_json::from_slice(&bytes).map_err(|_| GoogleApiError::MalformedResponse)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GoogleApiError {
    InvalidRequest,
    UnauthorizedAccount,
    ScopeDenied,
    CredentialUnavailable,
    ReauthorizationRequired,
    RateLimited,
    ProviderUnavailable,
    MalformedResponse,
    ResponseTooLarge,
    Cancelled,
    CursorExpired,
}

impl fmt::Display for GoogleApiError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::InvalidRequest => "google_invalid_request",
            Self::UnauthorizedAccount => "google_unauthorized_account",
            Self::ScopeDenied => "google_scope_denied",
            Self::CredentialUnavailable => "google_credential_unavailable",
            Self::ReauthorizationRequired => "google_reauthorization_required",
            Self::RateLimited => "google_rate_limited",
            Self::ProviderUnavailable => "google_provider_unavailable",
            Self::MalformedResponse => "google_malformed_response",
            Self::ResponseTooLarge => "google_response_too_large",
            Self::Cancelled => "google_cancelled",
            Self::CursorExpired => "google_cursor_expired",
        })
    }
}

impl std::error::Error for GoogleApiError {}
