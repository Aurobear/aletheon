//! Read-only Google Calendar capability adapter.

use super::client::{GoogleApiClient, GoogleApiError};
use async_trait::async_trait;
use chrono::DateTime;
use fabric::{
    CalendarEntry, CalendarEntryPage, CalendarQuery, ExternalCapabilityId, ExternalRecordRef,
    OpaqueCursor, OpaqueProviderObjectId, PrincipalId,
};
use serde::Deserialize;
use tokio_util::sync::CancellationToken;

#[async_trait]
pub trait CalendarCapability: Send + Sync {
    async fn list_events(
        &self,
        principal: &PrincipalId,
        range: CalendarQuery,
        cancel: &CancellationToken,
    ) -> Result<CalendarEntryPage, GoogleApiError>;
}

#[derive(Debug, Clone)]
pub struct GoogleCalendarAdapter {
    pub(crate) client: GoogleApiClient,
}

impl GoogleCalendarAdapter {
    pub fn new(client: GoogleApiClient) -> Self {
        Self { client }
    }
}

#[async_trait]
impl CalendarCapability for GoogleCalendarAdapter {
    async fn list_events(
        &self,
        principal: &PrincipalId,
        range: CalendarQuery,
        cancel: &CancellationToken,
    ) -> Result<CalendarEntryPage, GoogleApiError> {
        range
            .validate()
            .map_err(|_| GoogleApiError::InvalidRequest)?;
        let mut url = reqwest::Url::parse(&format!(
            "{}/calendars/primary/events",
            self.client.endpoints().calendar_base.trim_end_matches('/')
        ))
        .map_err(|_| GoogleApiError::InvalidRequest)?;
        {
            let mut pairs = url.query_pairs_mut();
            pairs
                .append_pair("timeMin", &millis_to_rfc3339(range.start_ms)?)
                .append_pair("timeMax", &millis_to_rfc3339(range.end_ms)?)
                .append_pair("timeZone", &range.timezone)
                .append_pair("maxResults", &range.page_size.to_string())
                .append_pair("singleEvents", "true")
                .append_pair("orderBy", "startTime");
            if let Some(token) = &range.page_token {
                pairs.append_pair("pageToken", token);
            }
        }
        let raw: RawEventList = self
            .client
            .get_json(
                principal,
                range.account_id,
                ExternalCapabilityId::new("calendar.read").unwrap(),
                url,
                cancel,
            )
            .await?;
        if raw.items.len() > usize::from(range.page_size) {
            return Err(GoogleApiError::MalformedResponse);
        }
        let mut events = Vec::with_capacity(raw.items.len());
        for item in raw.items {
            events.push(normalize_event(range.account_id, &range.timezone, item)?);
        }
        let page = CalendarEntryPage {
            account_id: range.account_id,
            events,
            next_page_token: raw
                .next_page_token
                .map(OpaqueCursor::new)
                .transpose()
                .map_err(|_| GoogleApiError::MalformedResponse)?,
        };
        page.validate()
            .map_err(|_| GoogleApiError::MalformedResponse)?;
        Ok(page)
    }
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventList {
    #[serde(default)]
    items: Vec<RawEvent>,
    next_page_token: Option<String>,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEvent {
    id: String,
    etag: Option<String>,
    updated: String,
    #[serde(default)]
    summary: String,
    location: Option<String>,
    start: RawEventTime,
    end: RawEventTime,
}

#[derive(Deserialize)]
#[serde(rename_all = "camelCase")]
struct RawEventTime {
    date_time: Option<String>,
    date: Option<String>,
    time_zone: Option<String>,
}

fn normalize_event(
    account: fabric::ExternalIdentityId,
    requested_timezone: &str,
    raw: RawEvent,
) -> Result<CalendarEntry, GoogleApiError> {
    if raw.id.is_empty() || raw.id.len() > 1_024 {
        return Err(GoogleApiError::MalformedResponse);
    }
    let all_day = raw.start.date_time.is_none();
    let start_ms = parse_event_time(&raw.start)?;
    let end_ms = parse_event_time(&raw.end)?;
    let source_timestamp_ms = DateTime::parse_from_rfc3339(&raw.updated)
        .map_err(|_| GoogleApiError::MalformedResponse)?
        .timestamp_millis();
    let event = CalendarEntry {
        source: ExternalRecordRef {
            account_id: account,
            provider_object_id: OpaqueProviderObjectId::new(raw.id)
                .map_err(|_| GoogleApiError::MalformedResponse)?,
            fetched_at_ms: chrono::Utc::now().timestamp_millis(),
            source_timestamp_ms,
            etag_or_history: raw
                .etag
                .map(OpaqueCursor::new)
                .transpose()
                .map_err(|_| GoogleApiError::MalformedResponse)?,
        },
        calendar_id: OpaqueProviderObjectId::new("primary").unwrap(),
        summary: raw.summary,
        location: raw.location,
        start_ms,
        end_ms,
        timezone: raw
            .start
            .time_zone
            .unwrap_or_else(|| requested_timezone.to_owned()),
        all_day,
    };
    event
        .validate()
        .map_err(|_| GoogleApiError::MalformedResponse)?;
    Ok(event)
}

fn parse_event_time(value: &RawEventTime) -> Result<i64, GoogleApiError> {
    if let Some(date_time) = &value.date_time {
        return DateTime::parse_from_rfc3339(date_time)
            .map(|value| value.timestamp_millis())
            .map_err(|_| GoogleApiError::MalformedResponse);
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

fn millis_to_rfc3339(value: i64) -> Result<String, GoogleApiError> {
    DateTime::from_timestamp_millis(value)
        .map(|value| value.to_rfc3339())
        .ok_or(GoogleApiError::InvalidRequest)
}
