//! Bounded, credential-free contracts for Google read-only capabilities.

use super::external_identity::ExternalIdentityId;
use serde::{Deserialize, Serialize};
use std::fmt;

pub const MAX_GMAIL_QUERY_BYTES: usize = 1_024;
pub const MAX_GOOGLE_PAGE_SIZE: u16 = 100;
pub const MAX_GMAIL_BODY_BYTES: usize = 256 * 1_024;
pub const MAX_CALENDAR_RANGE_MS: i64 = 366 * 24 * 60 * 60 * 1_000;
pub const MAX_GOOGLE_PROVIDER_ID_BYTES: usize = 1_024;
const MAX_TEXT_BYTES: usize = 8 * 1_024;
const MAX_TIMEZONE_BYTES: usize = 128;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ProviderRecordRef {
    pub account_id: ExternalIdentityId,
    pub provider_object_id: String,
    pub fetched_at_ms: i64,
    pub source_timestamp_ms: i64,
    pub etag_or_history: Option<String>,
}

impl ProviderRecordRef {
    pub fn validate(&self) -> Result<(), GoogleContractError> {
        bounded(
            &self.provider_object_id,
            MAX_GOOGLE_PROVIDER_ID_BYTES,
            "provider_object_id",
        )?;
        if self.fetched_at_ms < 0 || self.source_timestamp_ms < 0 {
            return Err(GoogleContractError::InvalidField("timestamp"));
        }
        if let Some(marker) = &self.etag_or_history {
            bounded(marker, MAX_GOOGLE_PROVIDER_ID_BYTES, "etag_or_history")?;
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailQuery {
    pub account_id: ExternalIdentityId,
    pub query: String,
    pub page_size: u16,
    pub page_token: Option<String>,
}

impl GmailQuery {
    pub fn validate(&self) -> Result<(), GoogleContractError> {
        bounded(&self.query, MAX_GMAIL_QUERY_BYTES, "query")?;
        valid_page_size(self.page_size)?;
        if let Some(token) = &self.page_token {
            bounded(token, MAX_GOOGLE_PROVIDER_ID_BYTES, "page_token")?;
        }
        Ok(())
    }
}

impl fmt::Debug for GmailQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("GmailQuery")
            .field("account_id", &self.account_id)
            .field("query", &"[REDACTED]")
            .field("page_size", &self.page_size)
            .field(
                "page_token",
                &self.page_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl fmt::Display for GmailQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "gmail query for {} (limit {})",
            self.account_id, self.page_size
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailMessageSummary {
    pub source: ProviderRecordRef,
    pub thread_id: String,
    pub subject: String,
    pub from: String,
    pub snippet: String,
    pub unread: bool,
    pub important: bool,
}

impl GmailMessageSummary {
    pub fn validate(&self) -> Result<(), GoogleContractError> {
        self.source.validate()?;
        bounded(&self.thread_id, MAX_GOOGLE_PROVIDER_ID_BYTES, "thread_id")?;
        bounded_allow_empty(&self.subject, MAX_TEXT_BYTES, "subject")?;
        bounded_allow_empty(&self.from, MAX_TEXT_BYTES, "from")?;
        bounded_allow_empty(&self.snippet, MAX_TEXT_BYTES, "snippet")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailMessage {
    pub summary: GmailMessageSummary,
    pub body_text: String,
}

impl GmailMessage {
    pub fn validate(&self) -> Result<(), GoogleContractError> {
        self.summary.validate()?;
        bounded_allow_empty(&self.body_text, MAX_GMAIL_BODY_BYTES, "body_text")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct GmailMessagePage {
    pub account_id: ExternalIdentityId,
    pub messages: Vec<GmailMessageSummary>,
    pub next_page_token: Option<String>,
}

impl GmailMessagePage {
    pub fn validate(&self) -> Result<(), GoogleContractError> {
        if self.messages.len() > usize::from(MAX_GOOGLE_PAGE_SIZE) {
            return Err(GoogleContractError::TooManyResults);
        }
        for message in &self.messages {
            message.validate()?;
            if message.source.account_id != self.account_id {
                return Err(GoogleContractError::AccountMismatch);
            }
        }
        if let Some(token) = &self.next_page_token {
            bounded(token, MAX_GOOGLE_PROVIDER_ID_BYTES, "next_page_token")?;
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarTimeRange {
    pub account_id: ExternalIdentityId,
    pub start_ms: i64,
    pub end_ms: i64,
    pub timezone: String,
    pub page_size: u16,
    pub page_token: Option<String>,
}

impl CalendarTimeRange {
    pub fn validate(&self) -> Result<(), GoogleContractError> {
        if self.start_ms < 0
            || self.end_ms <= self.start_ms
            || self.end_ms - self.start_ms > MAX_CALENDAR_RANGE_MS
        {
            return Err(GoogleContractError::InvalidTimeRange);
        }
        bounded(&self.timezone, MAX_TIMEZONE_BYTES, "timezone")?;
        valid_page_size(self.page_size)?;
        if let Some(token) = &self.page_token {
            bounded(token, MAX_GOOGLE_PROVIDER_ID_BYTES, "page_token")?;
        }
        Ok(())
    }
}

impl fmt::Debug for CalendarTimeRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CalendarTimeRange")
            .field("account_id", &self.account_id)
            .field("start_ms", &self.start_ms)
            .field("end_ms", &self.end_ms)
            .field("timezone", &self.timezone)
            .field("page_size", &self.page_size)
            .field(
                "page_token",
                &self.page_token.as_ref().map(|_| "[REDACTED]"),
            )
            .finish()
    }
}

impl fmt::Display for CalendarTimeRange {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "calendar range for {}", self.account_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarEvent {
    pub source: ProviderRecordRef,
    pub calendar_id: String,
    pub summary: String,
    pub location: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub timezone: String,
    pub all_day: bool,
}

impl CalendarEvent {
    pub fn validate(&self) -> Result<(), GoogleContractError> {
        self.source.validate()?;
        bounded(
            &self.calendar_id,
            MAX_GOOGLE_PROVIDER_ID_BYTES,
            "calendar_id",
        )?;
        bounded_allow_empty(&self.summary, MAX_TEXT_BYTES, "summary")?;
        if let Some(location) = &self.location {
            bounded_allow_empty(location, MAX_TEXT_BYTES, "location")?;
        }
        bounded(&self.timezone, MAX_TIMEZONE_BYTES, "timezone")?;
        if self.start_ms < 0 || self.end_ms < self.start_ms {
            return Err(GoogleContractError::InvalidTimeRange);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarEventPage {
    pub account_id: ExternalIdentityId,
    pub events: Vec<CalendarEvent>,
    pub next_page_token: Option<String>,
}

impl CalendarEventPage {
    pub fn validate(&self) -> Result<(), GoogleContractError> {
        if self.events.len() > usize::from(MAX_GOOGLE_PAGE_SIZE) {
            return Err(GoogleContractError::TooManyResults);
        }
        for event in &self.events {
            event.validate()?;
            if event.source.account_id != self.account_id {
                return Err(GoogleContractError::AccountMismatch);
            }
        }
        if let Some(token) = &self.next_page_token {
            bounded(token, MAX_GOOGLE_PROVIDER_ID_BYTES, "next_page_token")?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GoogleContractError {
    InvalidField(&'static str),
    FieldTooLarge(&'static str),
    InvalidPageSize,
    InvalidTimeRange,
    TooManyResults,
    AccountMismatch,
}

impl fmt::Display for GoogleContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(f, "invalid Google field: {field}"),
            Self::FieldTooLarge(field) => write!(f, "Google field too large: {field}"),
            Self::InvalidPageSize => f.write_str("Google page size is outside the allowed range"),
            Self::InvalidTimeRange => f.write_str("Google time range is outside the allowed range"),
            Self::TooManyResults => f.write_str("Google result page exceeds its bound"),
            Self::AccountMismatch => f.write_str("Google record belongs to a different account"),
        }
    }
}

impl std::error::Error for GoogleContractError {}

fn valid_page_size(page_size: u16) -> Result<(), GoogleContractError> {
    if page_size == 0 || page_size > MAX_GOOGLE_PAGE_SIZE {
        Err(GoogleContractError::InvalidPageSize)
    } else {
        Ok(())
    }
}

fn bounded(value: &str, max: usize, field: &'static str) -> Result<(), GoogleContractError> {
    if value.trim().is_empty() {
        return Err(GoogleContractError::InvalidField(field));
    }
    bounded_allow_empty(value, max, field)
}

fn bounded_allow_empty(
    value: &str,
    max: usize,
    field: &'static str,
) -> Result<(), GoogleContractError> {
    if value.len() > max {
        Err(GoogleContractError::FieldTooLarge(field))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn gmail_query_is_bounded_and_redacted() {
        let mut query = GmailQuery {
            account_id: ExternalIdentityId::new(),
            query: "is:unread is:important secret phrase".into(),
            page_size: 20,
            page_token: Some("opaque-page-secret".into()),
        };
        query.validate().unwrap();
        for rendered in [format!("{query:?}"), query.to_string()] {
            assert!(!rendered.contains("secret phrase"));
            assert!(!rendered.contains("opaque-page-secret"));
        }
        query.query = "x".repeat(MAX_GMAIL_QUERY_BYTES + 1);
        assert_eq!(
            query.validate(),
            Err(GoogleContractError::FieldTooLarge("query"))
        );
        query.query = "ok".into();
        query.page_size = MAX_GOOGLE_PAGE_SIZE + 1;
        assert_eq!(query.validate(), Err(GoogleContractError::InvalidPageSize));
    }

    #[test]
    fn calendar_range_is_bounded_and_redacts_page_token() {
        let mut range = CalendarTimeRange {
            account_id: ExternalIdentityId::new(),
            start_ms: 1_000,
            end_ms: 2_000,
            timezone: "Asia/Shanghai".into(),
            page_size: 20,
            page_token: Some("calendar-page-secret".into()),
        };
        range.validate().unwrap();
        assert!(!format!("{range:?}").contains("calendar-page-secret"));
        range.end_ms = range.start_ms + MAX_CALENDAR_RANGE_MS + 1;
        assert_eq!(range.validate(), Err(GoogleContractError::InvalidTimeRange));
    }

    #[test]
    fn imported_records_require_provenance_and_account_consistency() {
        let account_id = ExternalIdentityId::new();
        let source = ProviderRecordRef {
            account_id,
            provider_object_id: "message-1".into(),
            fetched_at_ms: 2_000,
            source_timestamp_ms: 1_000,
            etag_or_history: Some("history-2".into()),
        };
        let summary = GmailMessageSummary {
            source,
            thread_id: "thread-1".into(),
            subject: "subject".into(),
            from: "sender@example.com".into(),
            snippet: "bounded".into(),
            unread: true,
            important: true,
        };
        let page = GmailMessagePage {
            account_id,
            messages: vec![summary.clone()],
            next_page_token: None,
        };
        page.validate().unwrap();
        let wrong = GmailMessagePage {
            account_id: ExternalIdentityId::new(),
            messages: vec![summary],
            next_page_token: None,
        };
        assert_eq!(wrong.validate(), Err(GoogleContractError::AccountMismatch));
    }

    #[test]
    fn serialized_contracts_never_expose_token_fields() {
        let query = GmailQuery {
            account_id: ExternalIdentityId::new(),
            query: "is:unread".into(),
            page_size: 10,
            page_token: None,
        };
        let json = serde_json::to_string(&query).unwrap();
        let decoded: GmailQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, query);
        assert!(!json.contains("access_token"));
        assert!(!json.contains("refresh_token"));
        assert!(!json.contains("authorization"));
    }
}
