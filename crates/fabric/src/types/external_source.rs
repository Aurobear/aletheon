//! Bounded, credential-free contracts for external information sources.

use super::external_identity::ExternalIdentityId;
use serde::{Deserialize, Serialize};
use std::fmt;
use std::ops::Deref;

pub const MAX_MAIL_QUERY_BYTES: usize = 1_024;
pub const MAX_EXTERNAL_PAGE_SIZE: u16 = 100;
pub const MAX_MAIL_BODY_BYTES: usize = 256 * 1_024;
pub const MAX_CALENDAR_RANGE_MS: i64 = 366 * 24 * 60 * 60 * 1_000;
pub const MAX_EXTERNAL_OBJECT_ID_BYTES: usize = 1_024;
const MAX_TEXT_BYTES: usize = 8 * 1_024;
const MAX_TIMEZONE_BYTES: usize = 128;

macro_rules! opaque_external_id {
    ($name:ident, $field:literal) => {
        #[derive(Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
        #[serde(transparent)]
        pub struct $name(String);

        impl $name {
            pub fn new(value: impl Into<String>) -> Result<Self, ExternalSourceContractError> {
                let value = value.into();
                bounded(&value, MAX_EXTERNAL_OBJECT_ID_BYTES, $field)?;
                if value.chars().any(char::is_control) {
                    return Err(ExternalSourceContractError::InvalidField($field));
                }
                Ok(Self(value))
            }

            pub fn as_str(&self) -> &str {
                &self.0
            }
        }

        impl Deref for $name {
            type Target = str;

            fn deref(&self) -> &Self::Target {
                self.as_str()
            }
        }

        impl fmt::Debug for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(concat!(stringify!($name), "([OPAQUE])"))
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                f.write_str(self.as_str())
            }
        }
    };
}

opaque_external_id!(OpaqueProviderObjectId, "provider_object_id");
opaque_external_id!(OpaqueCursor, "cursor");

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalRecordRef {
    pub account_id: ExternalIdentityId,
    pub provider_object_id: OpaqueProviderObjectId,
    pub fetched_at_ms: i64,
    pub source_timestamp_ms: i64,
    pub etag_or_history: Option<OpaqueCursor>,
}

impl ExternalRecordRef {
    pub fn validate(&self) -> Result<(), ExternalSourceContractError> {
        bounded(
            self.provider_object_id.as_str(),
            MAX_EXTERNAL_OBJECT_ID_BYTES,
            "provider_object_id",
        )?;
        if self.fetched_at_ms < 0 || self.source_timestamp_ms < 0 {
            return Err(ExternalSourceContractError::InvalidField("timestamp"));
        }
        if let Some(marker) = &self.etag_or_history {
            bounded(
                marker.as_str(),
                MAX_EXTERNAL_OBJECT_ID_BYTES,
                "etag_or_history",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailQuery {
    pub account_id: ExternalIdentityId,
    pub query: String,
    pub page_size: u16,
    pub page_token: Option<OpaqueCursor>,
}

impl MailQuery {
    pub fn validate(&self) -> Result<(), ExternalSourceContractError> {
        bounded(&self.query, MAX_MAIL_QUERY_BYTES, "query")?;
        valid_page_size(self.page_size)?;
        if let Some(token) = &self.page_token {
            bounded(token.as_str(), MAX_EXTERNAL_OBJECT_ID_BYTES, "page_token")?;
        }
        Ok(())
    }
}

impl fmt::Debug for MailQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("MailQuery")
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

impl fmt::Display for MailQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "mail query for {} (limit {})",
            self.account_id, self.page_size
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailMessageSummary {
    pub source: ExternalRecordRef,
    pub thread_id: OpaqueProviderObjectId,
    pub subject: String,
    pub from: String,
    pub snippet: String,
    pub unread: bool,
    pub important: bool,
}

impl MailMessageSummary {
    pub fn validate(&self) -> Result<(), ExternalSourceContractError> {
        self.source.validate()?;
        bounded(
            self.thread_id.as_str(),
            MAX_EXTERNAL_OBJECT_ID_BYTES,
            "thread_id",
        )?;
        bounded_allow_empty(&self.subject, MAX_TEXT_BYTES, "subject")?;
        bounded_allow_empty(&self.from, MAX_TEXT_BYTES, "from")?;
        bounded_allow_empty(&self.snippet, MAX_TEXT_BYTES, "snippet")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailMessage {
    pub summary: MailMessageSummary,
    pub body_text: String,
}

impl MailMessage {
    pub fn validate(&self) -> Result<(), ExternalSourceContractError> {
        self.summary.validate()?;
        bounded_allow_empty(&self.body_text, MAX_MAIL_BODY_BYTES, "body_text")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailMessagePage {
    pub account_id: ExternalIdentityId,
    pub messages: Vec<MailMessageSummary>,
    pub next_page_token: Option<OpaqueCursor>,
}

impl MailMessagePage {
    pub fn validate(&self) -> Result<(), ExternalSourceContractError> {
        if self.messages.len() > usize::from(MAX_EXTERNAL_PAGE_SIZE) {
            return Err(ExternalSourceContractError::TooManyResults);
        }
        for message in &self.messages {
            message.validate()?;
            if message.source.account_id != self.account_id {
                return Err(ExternalSourceContractError::AccountMismatch);
            }
        }
        if let Some(token) = &self.next_page_token {
            bounded(
                token.as_str(),
                MAX_EXTERNAL_OBJECT_ID_BYTES,
                "next_page_token",
            )?;
        }
        Ok(())
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarQuery {
    pub account_id: ExternalIdentityId,
    pub start_ms: i64,
    pub end_ms: i64,
    pub timezone: String,
    pub page_size: u16,
    pub page_token: Option<OpaqueCursor>,
}

impl CalendarQuery {
    pub fn validate(&self) -> Result<(), ExternalSourceContractError> {
        if self.start_ms < 0
            || self.end_ms <= self.start_ms
            || self.end_ms - self.start_ms > MAX_CALENDAR_RANGE_MS
        {
            return Err(ExternalSourceContractError::InvalidTimeRange);
        }
        bounded(&self.timezone, MAX_TIMEZONE_BYTES, "timezone")?;
        valid_page_size(self.page_size)?;
        if let Some(token) = &self.page_token {
            bounded(token.as_str(), MAX_EXTERNAL_OBJECT_ID_BYTES, "page_token")?;
        }
        Ok(())
    }
}

impl fmt::Debug for CalendarQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("CalendarQuery")
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

impl fmt::Display for CalendarQuery {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "calendar range for {}", self.account_id)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarEntry {
    pub source: ExternalRecordRef,
    pub calendar_id: OpaqueProviderObjectId,
    pub summary: String,
    pub location: Option<String>,
    pub start_ms: i64,
    pub end_ms: i64,
    pub timezone: String,
    pub all_day: bool,
}

impl CalendarEntry {
    pub fn validate(&self) -> Result<(), ExternalSourceContractError> {
        self.source.validate()?;
        bounded(
            self.calendar_id.as_str(),
            MAX_EXTERNAL_OBJECT_ID_BYTES,
            "calendar_id",
        )?;
        bounded_allow_empty(&self.summary, MAX_TEXT_BYTES, "summary")?;
        if let Some(location) = &self.location {
            bounded_allow_empty(location, MAX_TEXT_BYTES, "location")?;
        }
        bounded(&self.timezone, MAX_TIMEZONE_BYTES, "timezone")?;
        if self.start_ms < 0 || self.end_ms < self.start_ms {
            return Err(ExternalSourceContractError::InvalidTimeRange);
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CalendarEntryPage {
    pub account_id: ExternalIdentityId,
    pub events: Vec<CalendarEntry>,
    pub next_page_token: Option<OpaqueCursor>,
}

impl CalendarEntryPage {
    pub fn validate(&self) -> Result<(), ExternalSourceContractError> {
        if self.events.len() > usize::from(MAX_EXTERNAL_PAGE_SIZE) {
            return Err(ExternalSourceContractError::TooManyResults);
        }
        for event in &self.events {
            event.validate()?;
            if event.source.account_id != self.account_id {
                return Err(ExternalSourceContractError::AccountMismatch);
            }
        }
        if let Some(token) = &self.next_page_token {
            bounded(
                token.as_str(),
                MAX_EXTERNAL_OBJECT_ID_BYTES,
                "next_page_token",
            )?;
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalSourceContractError {
    InvalidField(&'static str),
    FieldTooLarge(&'static str),
    InvalidPageSize,
    InvalidTimeRange,
    TooManyResults,
    AccountMismatch,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalChangeBatch<T> {
    pub account_id: ExternalIdentityId,
    pub changes: Vec<T>,
    pub next_cursor: Option<OpaqueCursor>,
}

impl<T> ExternalChangeBatch<T> {
    pub fn validate_size(&self) -> Result<(), ExternalSourceContractError> {
        if self.changes.len() > usize::from(MAX_EXTERNAL_PAGE_SIZE) {
            return Err(ExternalSourceContractError::TooManyResults);
        }
        Ok(())
    }
}

impl fmt::Display for ExternalSourceContractError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(f, "invalid external source field: {field}"),
            Self::FieldTooLarge(field) => write!(f, "external source field too large: {field}"),
            Self::InvalidPageSize => {
                f.write_str("external source page size is outside the allowed range")
            }
            Self::InvalidTimeRange => {
                f.write_str("external source time range is outside the allowed range")
            }
            Self::TooManyResults => f.write_str("external source result page exceeds its bound"),
            Self::AccountMismatch => {
                f.write_str("external source record belongs to a different account")
            }
        }
    }
}

impl std::error::Error for ExternalSourceContractError {}

fn valid_page_size(page_size: u16) -> Result<(), ExternalSourceContractError> {
    if page_size == 0 || page_size > MAX_EXTERNAL_PAGE_SIZE {
        Err(ExternalSourceContractError::InvalidPageSize)
    } else {
        Ok(())
    }
}

fn bounded(
    value: &str,
    max: usize,
    field: &'static str,
) -> Result<(), ExternalSourceContractError> {
    if value.trim().is_empty() {
        return Err(ExternalSourceContractError::InvalidField(field));
    }
    bounded_allow_empty(value, max, field)
}

fn bounded_allow_empty(
    value: &str,
    max: usize,
    field: &'static str,
) -> Result<(), ExternalSourceContractError> {
    if value.len() > max {
        Err(ExternalSourceContractError::FieldTooLarge(field))
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn mail_query_is_bounded_and_redacted() {
        let mut query = MailQuery {
            account_id: ExternalIdentityId::new(),
            query: "is:unread is:important secret phrase".into(),
            page_size: 20,
            page_token: Some(OpaqueCursor::new("opaque-page-secret").unwrap()),
        };
        query.validate().unwrap();
        for rendered in [format!("{query:?}"), query.to_string()] {
            assert!(!rendered.contains("secret phrase"));
            assert!(!rendered.contains("opaque-page-secret"));
        }
        query.query = "x".repeat(MAX_MAIL_QUERY_BYTES + 1);
        assert_eq!(
            query.validate(),
            Err(ExternalSourceContractError::FieldTooLarge("query"))
        );
        query.query = "ok".into();
        query.page_size = MAX_EXTERNAL_PAGE_SIZE + 1;
        assert_eq!(
            query.validate(),
            Err(ExternalSourceContractError::InvalidPageSize)
        );
    }

    #[test]
    fn calendar_range_is_bounded_and_redacts_page_token() {
        let mut range = CalendarQuery {
            account_id: ExternalIdentityId::new(),
            start_ms: 1_000,
            end_ms: 2_000,
            timezone: "Asia/Shanghai".into(),
            page_size: 20,
            page_token: Some(OpaqueCursor::new("calendar-page-secret").unwrap()),
        };
        range.validate().unwrap();
        assert!(!format!("{range:?}").contains("calendar-page-secret"));
        range.end_ms = range.start_ms + MAX_CALENDAR_RANGE_MS + 1;
        assert_eq!(
            range.validate(),
            Err(ExternalSourceContractError::InvalidTimeRange)
        );
    }

    #[test]
    fn imported_records_require_provenance_and_account_consistency() {
        let account_id = ExternalIdentityId::new();
        let source = ExternalRecordRef {
            account_id,
            provider_object_id: OpaqueProviderObjectId::new("message-1").unwrap(),
            fetched_at_ms: 2_000,
            source_timestamp_ms: 1_000,
            etag_or_history: Some(OpaqueCursor::new("history-2").unwrap()),
        };
        let summary = MailMessageSummary {
            source,
            thread_id: OpaqueProviderObjectId::new("thread-1").unwrap(),
            subject: "subject".into(),
            from: "sender@example.com".into(),
            snippet: "bounded".into(),
            unread: true,
            important: true,
        };
        let page = MailMessagePage {
            account_id,
            messages: vec![summary.clone()],
            next_page_token: None,
        };
        page.validate().unwrap();
        let wrong = MailMessagePage {
            account_id: ExternalIdentityId::new(),
            messages: vec![summary],
            next_page_token: None,
        };
        assert_eq!(
            wrong.validate(),
            Err(ExternalSourceContractError::AccountMismatch)
        );
    }

    #[test]
    fn serialized_contracts_never_expose_token_fields() {
        let query = MailQuery {
            account_id: ExternalIdentityId::new(),
            query: "is:unread".into(),
            page_size: 10,
            page_token: None,
        };
        let json = serde_json::to_string(&query).unwrap();
        let decoded: MailQuery = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, query);
        assert!(!json.contains("access_token"));
        assert!(!json.contains("refresh_token"));
        assert!(!json.contains("authorization"));
    }
}
