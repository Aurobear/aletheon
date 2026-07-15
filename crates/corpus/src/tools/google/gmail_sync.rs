//! Bounded Gmail History API synchronization without persistence side effects.

use super::{GmailCapability, GoogleApiError, GoogleGmailAdapter};
use fabric::{
    ExternalEventDraft, ExternalEventEnvelope, ExternalIdentityId, ExternalObjectRef,
    ExternalScope, GmailQuery, GoogleEvent, IdentityProvider, MailChange, PrincipalId,
};
use serde::Deserialize;
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

const HARD_MAX_PAGES: usize = 100;
const HARD_MAX_MESSAGES: usize = 2_000;

#[derive(Debug, Clone)]
pub struct GmailHistorySyncConfig {
    pub max_pages: usize,
    pub max_messages: usize,
    pub page_size: u16,
    pub reconciliation_query: String,
}

impl Default for GmailHistorySyncConfig {
    fn default() -> Self {
        Self {
            max_pages: 20,
            max_messages: 500,
            page_size: 100,
            reconciliation_query: "in:anywhere".into(),
        }
    }
}

impl GmailHistorySyncConfig {
    fn validate(&self) -> Result<(), GoogleApiError> {
        if self.max_pages == 0
            || self.max_pages > HARD_MAX_PAGES
            || self.max_messages == 0
            || self.max_messages > HARD_MAX_MESSAGES
            || !(1..=100).contains(&self.page_size)
            || self.reconciliation_query.is_empty()
            || self.reconciliation_query.len() > 1_024
        {
            Err(GoogleApiError::InvalidRequest)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GmailSyncHealthEvent {
    ReconciliationBounded {
        pages_examined: usize,
        messages_examined: usize,
    },
}

#[derive(Debug, Clone)]
pub struct GmailSyncBatch {
    pub input_cursor: Option<String>,
    pub successor_cursor: String,
    pub events: Vec<ExternalEventEnvelope>,
    pub health_events: Vec<GmailSyncHealthEvent>,
    pub baseline_only: bool,
    pub reconciled: bool,
}

#[derive(Debug, Clone)]
pub struct GmailHistorySynchronizer {
    gmail: GoogleGmailAdapter,
    config: GmailHistorySyncConfig,
}

impl GmailHistorySynchronizer {
    pub fn new(
        gmail: GoogleGmailAdapter,
        config: GmailHistorySyncConfig,
    ) -> Result<Self, GoogleApiError> {
        config.validate()?;
        Ok(Self { gmail, config })
    }

    pub async fn synchronize(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        input_cursor: Option<&str>,
        known_dedup_keys: &HashSet<String>,
        cancel: &CancellationToken,
    ) -> Result<GmailSyncBatch, GoogleApiError> {
        if input_cursor.is_some_and(|cursor| cursor.is_empty() || cursor.len() > 16 * 1_024) {
            return Err(GoogleApiError::InvalidRequest);
        }
        let Some(cursor) = input_cursor else {
            let successor = self.profile_history_id(principal, account, cancel).await?;
            return Ok(GmailSyncBatch {
                input_cursor: None,
                successor_cursor: successor,
                events: Vec::new(),
                health_events: Vec::new(),
                baseline_only: true,
                reconciled: false,
            });
        };

        match self
            .history(principal, account, cursor, known_dedup_keys, cancel)
            .await
        {
            Ok(batch) => Ok(batch),
            Err(GoogleApiError::CursorExpired) => {
                self.reconcile(principal, account, cursor, known_dedup_keys, cancel)
                    .await
            }
            Err(error) => Err(error),
        }
    }

    async fn profile_history_id(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        cancel: &CancellationToken,
    ) -> Result<String, GoogleApiError> {
        let url = reqwest::Url::parse(&format!(
            "{}/users/me/profile",
            self.gmail
                .client
                .endpoints()
                .gmail_base
                .trim_end_matches('/')
        ))
        .map_err(|_| GoogleApiError::InvalidRequest)?;
        let profile: RawProfile = self
            .gmail
            .client
            .get_json(
                principal,
                account,
                ExternalScope::GmailReadonly,
                url,
                cancel,
            )
            .await?;
        validate_history_id(&profile.history_id)?;
        Ok(profile.history_id)
    }

    async fn history(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        input_cursor: &str,
        known_dedup_keys: &HashSet<String>,
        cancel: &CancellationToken,
    ) -> Result<GmailSyncBatch, GoogleApiError> {
        let mut page_token = None;
        let mut pages = 0;
        let mut examined = 0;
        let mut emitted_keys = HashSet::new();
        let mut events = Vec::new();
        let successor = loop {
            if pages >= self.config.max_pages {
                return Err(GoogleApiError::ResponseTooLarge);
            }
            let page = self
                .history_page(
                    principal,
                    account,
                    input_cursor,
                    page_token.as_deref(),
                    cancel,
                )
                .await?;
            pages += 1;
            validate_history_id(&page.history_id)?;
            for history in page.history {
                validate_history_id(&history.id)?;
                for added in history.messages_added {
                    examined += 1;
                    self.ensure_message_bound(examined)?;
                    if let Some(event) = self
                        .message_event(
                            principal,
                            account,
                            &history.id,
                            &added.message.id,
                            true,
                            cancel,
                        )
                        .await?
                        .filter(|event| {
                            !known_dedup_keys.contains(&event.dedup_key)
                                && emitted_keys.insert(event.dedup_key.clone())
                        })
                    {
                        events.push(event);
                    }
                }
                for changed in history
                    .labels_added
                    .into_iter()
                    .chain(history.labels_removed)
                {
                    examined += 1;
                    self.ensure_message_bound(examined)?;
                    if let Some(event) = self
                        .message_event(
                            principal,
                            account,
                            &history.id,
                            &changed.message.id,
                            false,
                            cancel,
                        )
                        .await?
                        .filter(|event| {
                            !known_dedup_keys.contains(&event.dedup_key)
                                && emitted_keys.insert(event.dedup_key.clone())
                        })
                    {
                        events.push(event);
                    }
                }
                for deleted in history.messages_deleted {
                    examined += 1;
                    self.ensure_message_bound(examined)?;
                    let event = deletion_event(account, &history.id, &deleted.message.id)?;
                    if !known_dedup_keys.contains(&event.dedup_key)
                        && emitted_keys.insert(event.dedup_key.clone())
                    {
                        events.push(event);
                    }
                }
            }
            page_token = page.next_page_token;
            if page_token.is_none() {
                break page.history_id;
            }
        };
        Ok(GmailSyncBatch {
            input_cursor: Some(input_cursor.into()),
            successor_cursor: successor,
            events,
            health_events: Vec::new(),
            baseline_only: false,
            reconciled: false,
        })
    }

    async fn reconcile(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        input_cursor: &str,
        known_dedup_keys: &HashSet<String>,
        cancel: &CancellationToken,
    ) -> Result<GmailSyncBatch, GoogleApiError> {
        let mut page_token = None;
        let mut pages = 0;
        let mut examined = 0;
        let mut events = Vec::new();
        let mut emitted_keys = HashSet::new();
        let mut bounded = false;
        loop {
            if pages >= self.config.max_pages || examined >= self.config.max_messages {
                bounded = page_token.is_some();
                break;
            }
            let page = self
                .gmail
                .search_messages(
                    principal,
                    GmailQuery {
                        account_id: account,
                        query: self.config.reconciliation_query.clone(),
                        page_size: self.config.page_size,
                        page_token: page_token.clone(),
                    },
                    cancel,
                )
                .await?;
            pages += 1;
            for summary in page.messages {
                examined += 1;
                if examined > self.config.max_messages {
                    bounded = true;
                    break;
                }
                let version = summary
                    .source
                    .etag_or_history
                    .clone()
                    .unwrap_or_else(|| input_cursor.into());
                let event = summary_event(account, &version, None, summary, false)?;
                if !known_dedup_keys.contains(&event.dedup_key)
                    && emitted_keys.insert(event.dedup_key.clone())
                {
                    events.push(event);
                }
            }
            page_token = page.next_page_token;
            if page_token.is_none() || bounded {
                break;
            }
        }
        let successor = self.profile_history_id(principal, account, cancel).await?;
        let health_events = bounded
            .then_some(GmailSyncHealthEvent::ReconciliationBounded {
                pages_examined: pages,
                messages_examined: examined,
            })
            .into_iter()
            .collect();
        Ok(GmailSyncBatch {
            input_cursor: Some(input_cursor.into()),
            successor_cursor: successor,
            events,
            health_events,
            baseline_only: false,
            reconciled: true,
        })
    }

    async fn history_page(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        cursor: &str,
        page_token: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<RawHistoryPage, GoogleApiError> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/users/me/history",
            self.gmail
                .client
                .endpoints()
                .gmail_base
                .trim_end_matches('/')
        ))
        .map_err(|_| GoogleApiError::InvalidRequest)?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("startHistoryId", cursor)
                .append_pair("maxResults", &self.config.page_size.to_string());
            if let Some(token) = page_token {
                query.append_pair("pageToken", token);
            }
        }
        self.gmail
            .client
            .get_json(
                principal,
                account,
                ExternalScope::GmailReadonly,
                url,
                cancel,
            )
            .await
    }

    async fn message_event(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        history_id: &str,
        message_id: &str,
        received: bool,
        cancel: &CancellationToken,
    ) -> Result<Option<ExternalEventEnvelope>, GoogleApiError> {
        validate_message_id(message_id)?;
        let summary = self
            .gmail
            .metadata(principal, account, message_id, cancel)
            .await?;
        let version = summary
            .source
            .etag_or_history
            .clone()
            .unwrap_or_else(|| history_id.to_owned());
        summary_event(account, &version, Some(history_id), summary, received).map(Some)
    }

    fn ensure_message_bound(&self, examined: usize) -> Result<(), GoogleApiError> {
        if examined > self.config.max_messages {
            Err(GoogleApiError::ResponseTooLarge)
        } else {
            Ok(())
        }
    }
}

fn summary_event(
    account: ExternalIdentityId,
    version: &str,
    provider_event_id: Option<&str>,
    summary: fabric::GmailMessageSummary,
    received: bool,
) -> Result<ExternalEventEnvelope, GoogleApiError> {
    let object = ExternalObjectRef {
        provider: IdentityProvider::Google,
        account_id: account,
        object_id: summary.source.provider_object_id.clone(),
        object_version: version.into(),
    };
    let event = if received {
        GoogleEvent::MailReceived(MailChange {
            message: summary.clone(),
            content: None,
        })
    } else {
        GoogleEvent::MailUpdated(MailChange {
            message: summary.clone(),
            content: None,
        })
    };
    ExternalEventEnvelope::from_draft(ExternalEventDraft {
        provider: IdentityProvider::Google,
        account_id: account,
        provider_event_id: provider_event_id.map(str::to_owned),
        object,
        observed_at_ms: summary.source.fetched_at_ms,
        source_timestamp_ms: summary.source.source_timestamp_ms,
        provenance: summary.source.clone(),
        event,
    })
    .map_err(|_| GoogleApiError::MalformedResponse)
}

fn deletion_event(
    account: ExternalIdentityId,
    history_id: &str,
    message_id: &str,
) -> Result<ExternalEventEnvelope, GoogleApiError> {
    validate_message_id(message_id)?;
    let now = chrono::Utc::now().timestamp_millis();
    let object = ExternalObjectRef {
        provider: IdentityProvider::Google,
        account_id: account,
        object_id: message_id.into(),
        object_version: history_id.into(),
    };
    ExternalEventEnvelope::from_draft(ExternalEventDraft {
        provider: IdentityProvider::Google,
        account_id: account,
        provider_event_id: Some(history_id.into()),
        object: object.clone(),
        observed_at_ms: now,
        source_timestamp_ms: now,
        provenance: fabric::ProviderRecordRef {
            account_id: account,
            provider_object_id: message_id.into(),
            fetched_at_ms: now,
            source_timestamp_ms: now,
            etag_or_history: Some(history_id.into()),
        },
        event: GoogleEvent::MailDeleted(object),
    })
    .map_err(|_| GoogleApiError::MalformedResponse)
}

fn validate_history_id(value: &str) -> Result<(), GoogleApiError> {
    if value.is_empty() || value.len() > 1_024 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        Err(GoogleApiError::MalformedResponse)
    } else {
        Ok(())
    }
}

fn validate_message_id(value: &str) -> Result<(), GoogleApiError> {
    if value.is_empty()
        || value.len() > 1_024
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
    {
        Err(GoogleApiError::MalformedResponse)
    } else {
        Ok(())
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawProfile {
    history_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawHistoryPage {
    #[serde(default)]
    history: Vec<RawHistory>,
    next_page_token: Option<String>,
    history_id: String,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawHistory {
    id: String,
    #[serde(default)]
    messages_added: Vec<RawAdded>,
    #[serde(default)]
    messages_deleted: Vec<RawDeleted>,
    #[serde(default)]
    labels_added: Vec<RawChanged>,
    #[serde(default)]
    labels_removed: Vec<RawChanged>,
}

#[derive(Deserialize)]
struct RawAdded {
    message: RawMessageRef,
}

#[derive(Deserialize)]
struct RawDeleted {
    message: RawMessageRef,
}

#[derive(Deserialize)]
struct RawChanged {
    message: RawMessageRef,
}

#[derive(Deserialize)]
struct RawMessageRef {
    id: String,
}
