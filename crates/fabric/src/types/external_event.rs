//! Credential-free normalized events emitted by external provider synchronizers.

use super::external_identity::{ExternalIdentityId, IdentityProvider};
use super::google::{
    CalendarEvent, GmailMessageSummary, GoogleContractError, ProviderRecordRef,
    MAX_GOOGLE_PROVIDER_ID_BYTES,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::fmt;
use uuid::Uuid;

pub const EXTERNAL_EVENT_SCHEMA_VERSION: u16 = 1;
const MAX_EXTERNAL_ID_BYTES: usize = MAX_GOOGLE_PROVIDER_ID_BYTES;
const MAX_MIME_BYTES: usize = 256;
const MAX_DRIVE_NAME_BYTES: usize = 8 * 1_024;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ExternalEventId(pub Uuid);

impl ExternalEventId {
    pub fn from_dedup_key(dedup_key: &str) -> Self {
        Self(Uuid::new_v5(&Uuid::NAMESPACE_URL, dedup_key.as_bytes()))
    }
}

impl fmt::Display for ExternalEventId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalObjectRef {
    pub provider: IdentityProvider,
    pub account_id: ExternalIdentityId,
    pub object_id: String,
    pub object_version: String,
}

impl ExternalObjectRef {
    pub fn validate(&self) -> Result<(), ExternalEventError> {
        bounded(&self.object_id, MAX_EXTERNAL_ID_BYTES, "object_id")?;
        bounded(
            &self.object_version,
            MAX_EXTERNAL_ID_BYTES,
            "object_version",
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalContentRef {
    pub artifact_id: String,
    pub sha256: String,
    pub size_bytes: u64,
    pub mime_type: String,
}

impl ExternalContentRef {
    pub fn validate(&self) -> Result<(), ExternalEventError> {
        bounded(&self.artifact_id, MAX_EXTERNAL_ID_BYTES, "artifact_id")?;
        if self.sha256.len() != 64 || !self.sha256.bytes().all(|byte| byte.is_ascii_hexdigit()) {
            return Err(ExternalEventError::InvalidField("sha256"));
        }
        bounded(&self.mime_type, MAX_MIME_BYTES, "mime_type")
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MailChange {
    pub message: GmailMessageSummary,
    pub content: Option<ExternalContentRef>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveFileMetadata {
    pub object: ExternalObjectRef,
    pub name: String,
    pub mime_type: String,
    pub size_bytes: Option<u64>,
    pub modified_at_ms: i64,
    pub selected: bool,
    pub content: Option<ExternalContentRef>,
}

impl DriveFileMetadata {
    pub fn validate(&self) -> Result<(), ExternalEventError> {
        self.object.validate()?;
        bounded_allow_empty(&self.name, MAX_DRIVE_NAME_BYTES, "name")?;
        bounded(&self.mime_type, MAX_MIME_BYTES, "mime_type")?;
        if self.modified_at_ms < 0 || (self.content.is_some() && !self.selected) {
            return Err(ExternalEventError::InvalidField("drive_metadata"));
        }
        if let Some(content) = &self.content {
            content.validate()?;
            if self
                .size_bytes
                .is_some_and(|size| size != content.size_bytes)
            {
                return Err(ExternalEventError::InvalidField("size_bytes"));
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "kind", content = "data", rename_all = "snake_case")]
pub enum GoogleEvent {
    MailReceived(MailChange),
    MailUpdated(MailChange),
    MailDeleted(ExternalObjectRef),
    CalendarEventCreated(CalendarEvent),
    CalendarEventUpdated(CalendarEvent),
    CalendarEventDeleted(ExternalObjectRef),
    DriveFileCreated(DriveFileMetadata),
    DriveFileUpdated(DriveFileMetadata),
    DriveFileDeleted(ExternalObjectRef),
}

impl GoogleEvent {
    pub const fn kind(&self) -> &'static str {
        match self {
            Self::MailReceived(_) => "mail_received",
            Self::MailUpdated(_) => "mail_updated",
            Self::MailDeleted(_) => "mail_deleted",
            Self::CalendarEventCreated(_) => "calendar_event_created",
            Self::CalendarEventUpdated(_) => "calendar_event_updated",
            Self::CalendarEventDeleted(_) => "calendar_event_deleted",
            Self::DriveFileCreated(_) => "drive_file_created",
            Self::DriveFileUpdated(_) => "drive_file_updated",
            Self::DriveFileDeleted(_) => "drive_file_deleted",
        }
    }

    pub fn validate(&self) -> Result<(), ExternalEventError> {
        match self {
            Self::MailReceived(change) | Self::MailUpdated(change) => {
                change.message.validate()?;
                if let Some(content) = &change.content {
                    content.validate()?;
                }
                Ok(())
            }
            Self::MailDeleted(object)
            | Self::CalendarEventDeleted(object)
            | Self::DriveFileDeleted(object) => object.validate(),
            Self::CalendarEventCreated(event) | Self::CalendarEventUpdated(event) => {
                event.validate().map_err(Into::into)
            }
            Self::DriveFileCreated(file) | Self::DriveFileUpdated(file) => file.validate(),
        }
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExternalEventEnvelope {
    pub id: ExternalEventId,
    pub provider: IdentityProvider,
    pub account_id: ExternalIdentityId,
    pub provider_event_id: Option<String>,
    pub object: ExternalObjectRef,
    pub dedup_key: String,
    pub observed_at_ms: i64,
    pub source_timestamp_ms: i64,
    pub provenance: ProviderRecordRef,
    pub payload_hash: String,
    pub schema_version: u16,
    pub event: GoogleEvent,
}

#[derive(Clone, PartialEq, Eq)]
pub struct ExternalEventDraft {
    pub provider: IdentityProvider,
    pub account_id: ExternalIdentityId,
    pub provider_event_id: Option<String>,
    pub object: ExternalObjectRef,
    pub observed_at_ms: i64,
    pub source_timestamp_ms: i64,
    pub provenance: ProviderRecordRef,
    pub event: GoogleEvent,
}

impl ExternalEventEnvelope {
    pub fn from_draft(draft: ExternalEventDraft) -> Result<Self, ExternalEventError> {
        let ExternalEventDraft {
            provider,
            account_id,
            provider_event_id,
            object,
            observed_at_ms,
            source_timestamp_ms,
            provenance,
            event,
        } = draft;
        object.validate()?;
        event.validate()?;
        if provider != object.provider
            || account_id != object.account_id
            || account_id != provenance.account_id
            || observed_at_ms < 0
            || source_timestamp_ms < 0
        {
            return Err(ExternalEventError::AccountOrProviderMismatch);
        }
        provenance.validate()?;
        if let Some(event_id) = &provider_event_id {
            bounded(event_id, MAX_EXTERNAL_ID_BYTES, "provider_event_id")?;
        }
        let event_component = provider_event_id
            .as_deref()
            .unwrap_or(object.object_id.as_str());
        let material = format!(
            "{:?}|{}|{}|{}|{}",
            provider,
            account_id,
            event.kind(),
            event_component,
            object.object_version
        );
        let dedup_key = hex_sha256(material.as_bytes());
        let payload =
            serde_json::to_vec(&event).map_err(|_| ExternalEventError::SerializationFailed)?;
        let payload_hash = hex_sha256(&payload);
        Ok(Self {
            id: ExternalEventId::from_dedup_key(&dedup_key),
            provider,
            account_id,
            provider_event_id,
            object,
            dedup_key,
            observed_at_ms,
            source_timestamp_ms,
            provenance,
            payload_hash,
            schema_version: EXTERNAL_EVENT_SCHEMA_VERSION,
            event,
        })
    }

    pub fn validate(&self) -> Result<(), ExternalEventError> {
        let rebuilt = Self::from_draft(ExternalEventDraft {
            provider: self.provider,
            account_id: self.account_id,
            provider_event_id: self.provider_event_id.clone(),
            object: self.object.clone(),
            observed_at_ms: self.observed_at_ms,
            source_timestamp_ms: self.source_timestamp_ms,
            provenance: self.provenance.clone(),
            event: self.event.clone(),
        })?;
        if self.schema_version != EXTERNAL_EVENT_SCHEMA_VERSION
            || self.id != rebuilt.id
            || self.dedup_key != rebuilt.dedup_key
            || self.payload_hash != rebuilt.payload_hash
        {
            return Err(ExternalEventError::IntegrityMismatch);
        }
        Ok(())
    }
}

impl fmt::Debug for ExternalEventEnvelope {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ExternalEventEnvelope")
            .field("id", &self.id)
            .field("provider", &self.provider)
            .field("account_id", &self.account_id)
            .field("kind", &self.event.kind())
            .field("dedup_key", &self.dedup_key)
            .field("payload", &"[REDACTED]")
            .field("schema_version", &self.schema_version)
            .finish()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ExternalEventError {
    InvalidField(&'static str),
    FieldTooLarge(&'static str),
    AccountOrProviderMismatch,
    IntegrityMismatch,
    SerializationFailed,
    Google(String),
}

impl fmt::Display for ExternalEventError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidField(field) => write!(f, "invalid external event field: {field}"),
            Self::FieldTooLarge(field) => write!(f, "external event field too large: {field}"),
            Self::AccountOrProviderMismatch => f.write_str("external event authority mismatch"),
            Self::IntegrityMismatch => f.write_str("external event integrity mismatch"),
            Self::SerializationFailed => f.write_str("external event serialization failed"),
            Self::Google(message) => write!(f, "external event payload invalid: {message}"),
        }
    }
}

impl std::error::Error for ExternalEventError {}

impl From<GoogleContractError> for ExternalEventError {
    fn from(error: GoogleContractError) -> Self {
        Self::Google(error.to_string())
    }
}

fn bounded(value: &str, max: usize, field: &'static str) -> Result<(), ExternalEventError> {
    if value.is_empty() {
        return Err(ExternalEventError::InvalidField(field));
    }
    bounded_allow_empty(value, max, field)
}

fn bounded_allow_empty(
    value: &str,
    max: usize,
    field: &'static str,
) -> Result<(), ExternalEventError> {
    if value.len() > max {
        Err(ExternalEventError::FieldTooLarge(field))
    } else {
        Ok(())
    }
}

fn hex_sha256(bytes: &[u8]) -> String {
    let digest = Sha256::digest(bytes);
    digest.iter().map(|byte| format!("{byte:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{ExternalIdentityId, ProviderRecordRef};

    fn envelope(version: &str, observed_at_ms: i64) -> ExternalEventEnvelope {
        let account = ExternalIdentityId::new();
        fixture(account, version, observed_at_ms)
    }

    fn fixture(
        account: ExternalIdentityId,
        version: &str,
        observed_at_ms: i64,
    ) -> ExternalEventEnvelope {
        let object = ExternalObjectRef {
            provider: IdentityProvider::Google,
            account_id: account,
            object_id: "message-1".into(),
            object_version: version.into(),
        };
        let provenance = ProviderRecordRef {
            account_id: account,
            provider_object_id: "message-1".into(),
            fetched_at_ms: observed_at_ms,
            source_timestamp_ms: 100,
            etag_or_history: Some(version.into()),
        };
        ExternalEventEnvelope::from_draft(ExternalEventDraft {
            provider: IdentityProvider::Google,
            account_id: account,
            provider_event_id: Some("history-9".into()),
            object,
            observed_at_ms,
            source_timestamp_ms: 100,
            provenance: provenance.clone(),
            event: GoogleEvent::MailReceived(MailChange {
                message: GmailMessageSummary {
                    source: provenance,
                    thread_id: "thread-1".into(),
                    subject: "bounded subject".into(),
                    from: "sender@example.com".into(),
                    snippet: "bounded snippet".into(),
                    unread: true,
                    important: true,
                },
                content: None,
            }),
        })
        .unwrap()
    }

    #[test]
    fn serialization_is_stable_and_credential_free() {
        let event = envelope("v1", 200);
        let json = serde_json::to_string(&event).unwrap();
        let decoded: ExternalEventEnvelope = serde_json::from_str(&json).unwrap();
        assert_eq!(decoded, event);
        decoded.validate().unwrap();
        assert!(!json.contains("access_token"));
        assert!(!json.contains("refresh_token"));
        assert!(!json.contains("Authorization"));
        assert!(!format!("{event:?}").contains("bounded subject"));
    }

    #[test]
    fn dedup_key_ignores_arrival_time_but_changes_with_object_version() {
        let account = ExternalIdentityId::new();
        let first = fixture(account, "v1", 200);
        let retry = fixture(account, "v1", 999);
        let update = fixture(account, "v2", 999);
        assert_eq!(first.dedup_key, retry.dedup_key);
        assert_eq!(first.id, retry.id);
        assert_ne!(first.dedup_key, update.dedup_key);
    }

    #[test]
    fn tampering_and_cross_account_provenance_fail_closed() {
        let mut event = envelope("v1", 200);
        event.payload_hash = "00".repeat(32);
        assert_eq!(event.validate(), Err(ExternalEventError::IntegrityMismatch));

        let account = ExternalIdentityId::new();
        let other = ExternalIdentityId::new();
        let mut mismatch = fixture(account, "v1", 200);
        mismatch.provenance.account_id = other;
        assert_eq!(
            mismatch.validate(),
            Err(ExternalEventError::AccountOrProviderMismatch)
        );
    }

    #[test]
    fn content_is_reference_only_and_unselected_drive_content_is_rejected() {
        let account = ExternalIdentityId::new();
        let content = ExternalContentRef {
            artifact_id: "sha256:artifact".into(),
            sha256: "ab".repeat(32),
            size_bytes: 4,
            mime_type: "text/plain".into(),
        };
        let file = DriveFileMetadata {
            object: ExternalObjectRef {
                provider: IdentityProvider::Google,
                account_id: account,
                object_id: "file-1".into(),
                object_version: "7".into(),
            },
            name: "report.txt".into(),
            mime_type: "text/plain".into(),
            size_bytes: Some(4),
            modified_at_ms: 1,
            selected: false,
            content: Some(content),
        };
        assert!(file.validate().is_err());
    }
}
