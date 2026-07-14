//! Bounded Google Calendar incremental synchronization.

use super::{GoogleApiError, GoogleCalendarAdapter};
use chrono::DateTime;
use fabric::{
    CalendarEvent, ExternalEventDraft, ExternalEventEnvelope, ExternalIdentityId,
    ExternalObjectRef, ExternalScope, GoogleEvent, IdentityProvider, PrincipalId,
    ProviderRecordRef,
};
use serde::Deserialize;
use std::collections::HashSet;
use tokio_util::sync::CancellationToken;

const HARD_MAX_PAGES: usize = 100;

#[derive(Debug, Clone)]
pub struct CalendarSyncConfig {
    pub window_start_ms: i64,
    pub window_end_ms: i64,
    pub timezone: String,
    pub max_pages: usize,
    pub page_size: u16,
}

impl CalendarSyncConfig {
    fn validate(&self) -> Result<(), GoogleApiError> {
        if self.window_start_ms < 0
            || self.window_end_ms <= self.window_start_ms
            || self.timezone.is_empty()
            || self.timezone.len() > 128
            || self.max_pages == 0
            || self.max_pages > HARD_MAX_PAGES
            || !(1..=2_500).contains(&self.page_size)
        {
            Err(GoogleApiError::InvalidRequest)
        } else {
            Ok(())
        }
    }
}

#[derive(Debug, Clone)]
pub struct CalendarSyncBatch {
    pub input_cursor: Option<String>,
    pub successor_cursor: String,
    pub events: Vec<ExternalEventEnvelope>,
    pub reconciled: bool,
}

#[derive(Debug, Clone)]
pub struct CalendarSynchronizer {
    calendar: GoogleCalendarAdapter,
    config: CalendarSyncConfig,
}

impl CalendarSynchronizer {
    pub fn new(
        calendar: GoogleCalendarAdapter,
        config: CalendarSyncConfig,
    ) -> Result<Self, GoogleApiError> {
        config.validate()?;
        Ok(Self { calendar, config })
    }

    pub async fn synchronize(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        input_cursor: Option<&str>,
        known_dedup_keys: &HashSet<String>,
        cancel: &CancellationToken,
    ) -> Result<CalendarSyncBatch, GoogleApiError> {
        if input_cursor.is_some_and(|cursor| cursor.is_empty() || cursor.len() > 16 * 1024) {
            return Err(GoogleApiError::InvalidRequest);
        }
        match self
            .collect(principal, account, input_cursor, known_dedup_keys, cancel)
            .await
        {
            Ok(batch) => Ok(batch),
            Err(GoogleApiError::CursorExpired) if input_cursor.is_some() => {
                let mut batch = self
                    .collect(principal, account, None, known_dedup_keys, cancel)
                    .await?;
                batch.input_cursor = input_cursor.map(str::to_owned);
                batch.reconciled = true;
                Ok(batch)
            }
            Err(error) => Err(error),
        }
    }

    async fn collect(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        input_cursor: Option<&str>,
        known_dedup_keys: &HashSet<String>,
        cancel: &CancellationToken,
    ) -> Result<CalendarSyncBatch, GoogleApiError> {
        let mut page_token = None;
        let mut pages = 0;
        let mut events = Vec::new();
        let mut emitted = HashSet::new();
        let successor = loop {
            if pages >= self.config.max_pages {
                return Err(GoogleApiError::ResponseTooLarge);
            }
            let page = self
                .page(
                    principal,
                    account,
                    input_cursor,
                    page_token.as_deref(),
                    cancel,
                )
                .await?;
            pages += 1;
            if page.items.len() > usize::from(self.config.page_size) {
                return Err(GoogleApiError::MalformedResponse);
            }
            for raw in page.items {
                let event = normalize(account, raw, input_cursor.is_some(), &self.config.timezone)?;
                if !known_dedup_keys.contains(&event.dedup_key)
                    && emitted.insert(event.dedup_key.clone())
                {
                    events.push(event);
                }
            }
            if let Some(next) = page.next_page_token {
                validate_token(&next)?;
                page_token = Some(next);
            } else {
                let token = page
                    .next_sync_token
                    .ok_or(GoogleApiError::MalformedResponse)?;
                validate_token(&token)?;
                break token;
            }
        };
        Ok(CalendarSyncBatch {
            input_cursor: input_cursor.map(str::to_owned),
            successor_cursor: successor,
            events,
            reconciled: false,
        })
    }

    async fn page(
        &self,
        principal: &PrincipalId,
        account: ExternalIdentityId,
        sync_token: Option<&str>,
        page_token: Option<&str>,
        cancel: &CancellationToken,
    ) -> Result<RawEventList, GoogleApiError> {
        let mut url = reqwest::Url::parse(&format!(
            "{}/calendars/primary/events",
            self.calendar
                .client
                .endpoints()
                .calendar_base
                .trim_end_matches('/')
        ))
        .map_err(|_| GoogleApiError::InvalidRequest)?;
        {
            let mut query = url.query_pairs_mut();
            query
                .append_pair("maxResults", &self.config.page_size.to_string())
                .append_pair("showDeleted", "true")
                .append_pair("singleEvents", "true");
            if let Some(token) = sync_token {
                query.append_pair("syncToken", token);
            } else {
                query
                    .append_pair("timeMin", &rfc3339(self.config.window_start_ms)?)
                    .append_pair("timeMax", &rfc3339(self.config.window_end_ms)?)
                    .append_pair("timeZone", &self.config.timezone)
                    .append_pair("orderBy", "startTime");
            }
            if let Some(token) = page_token {
                query.append_pair("pageToken", token);
            }
        }
        self.calendar
            .client
            .get_json(
                principal,
                account,
                ExternalScope::CalendarReadonly,
                url,
                cancel,
            )
            .await
    }
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventList {
    #[serde(default)]
    items: Vec<RawEvent>,
    next_page_token: Option<String>,
    next_sync_token: Option<String>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEvent {
    id: String,
    etag: Option<String>,
    updated: Option<String>,
    #[serde(default)]
    summary: String,
    location: Option<String>,
    status: Option<String>,
    start: Option<RawEventTime>,
    end: Option<RawEventTime>,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventTime {
    date_time: Option<String>,
    date: Option<String>,
    time_zone: Option<String>,
}

fn normalize(
    account: ExternalIdentityId,
    raw: RawEvent,
    incremental: bool,
    timezone: &str,
) -> Result<ExternalEventEnvelope, GoogleApiError> {
    validate_id(&raw.id)?;
    let now = chrono::Utc::now().timestamp_millis();
    let source_ms = raw
        .updated
        .as_deref()
        .map(parse_timestamp)
        .transpose()?
        .unwrap_or(now);
    let version = raw
        .etag
        .clone()
        .or(raw.updated.clone())
        .unwrap_or_else(|| source_ms.to_string());
    validate_token(&version)?;
    let object = ExternalObjectRef {
        provider: IdentityProvider::Google,
        account_id: account,
        object_id: raw.id.clone(),
        object_version: version.clone(),
    };
    let provenance = ProviderRecordRef {
        account_id: account,
        provider_object_id: raw.id.clone(),
        fetched_at_ms: now,
        source_timestamp_ms: source_ms,
        etag_or_history: Some(version.clone()),
    };
    let event = if raw.status.as_deref() == Some("cancelled") {
        GoogleEvent::CalendarEventDeleted(object.clone())
    } else {
        let start = raw.start.ok_or(GoogleApiError::MalformedResponse)?;
        let end = raw.end.ok_or(GoogleApiError::MalformedResponse)?;
        let all_day = start.date_time.is_none();
        let calendar_event = CalendarEvent {
            source: provenance.clone(),
            calendar_id: "primary".into(),
            summary: raw.summary,
            location: raw.location,
            start_ms: parse_time(&start)?,
            end_ms: parse_time(&end)?,
            timezone: start.time_zone.unwrap_or_else(|| timezone.to_owned()),
            all_day,
        };
        if incremental {
            GoogleEvent::CalendarEventUpdated(calendar_event)
        } else {
            GoogleEvent::CalendarEventCreated(calendar_event)
        }
    };
    ExternalEventEnvelope::from_draft(ExternalEventDraft {
        provider: IdentityProvider::Google,
        account_id: account,
        provider_event_id: Some(raw.id),
        object,
        observed_at_ms: now,
        source_timestamp_ms: source_ms,
        provenance,
        event,
    })
    .map_err(|_| GoogleApiError::MalformedResponse)
}

fn parse_time(value: &RawEventTime) -> Result<i64, GoogleApiError> {
    if let Some(value) = value.date_time.as_deref() {
        return parse_timestamp(value);
    }
    let date = value
        .date
        .as_deref()
        .ok_or(GoogleApiError::MalformedResponse)?;
    chrono::NaiveDate::parse_from_str(date, "%Y-%m-%d")
        .ok()
        .and_then(|date| date.and_hms_opt(0, 0, 0))
        .map(|date| date.and_utc().timestamp_millis())
        .ok_or(GoogleApiError::MalformedResponse)
}

fn parse_timestamp(value: &str) -> Result<i64, GoogleApiError> {
    DateTime::parse_from_rfc3339(value)
        .map(|value| value.timestamp_millis())
        .map_err(|_| GoogleApiError::MalformedResponse)
}

fn rfc3339(value: i64) -> Result<String, GoogleApiError> {
    DateTime::from_timestamp_millis(value)
        .map(|value| value.to_rfc3339())
        .ok_or(GoogleApiError::InvalidRequest)
}

fn validate_id(value: &str) -> Result<(), GoogleApiError> {
    if value.is_empty() || value.len() > 1_024 {
        Err(GoogleApiError::MalformedResponse)
    } else {
        Ok(())
    }
}

fn validate_token(value: &str) -> Result<(), GoogleApiError> {
    if value.is_empty() || value.len() > 16 * 1_024 {
        Err(GoogleApiError::MalformedResponse)
    } else {
        Ok(())
    }
}
