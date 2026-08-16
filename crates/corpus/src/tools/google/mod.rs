//! First-party Google capability adapters.

pub mod calendar;
pub mod calendar_sync;
pub mod client;
pub mod drive;
pub mod drive_sync;
pub mod event;
pub mod gmail;
pub mod gmail_sync;
pub mod oauth;
pub mod source;
pub mod tools;

pub use calendar::{CalendarCapability, GoogleCalendarAdapter};
pub use calendar_sync::{CalendarSyncBatch, CalendarSyncConfig, CalendarSynchronizer};
pub use client::{
    GoogleAccessToken, GoogleApiClient, GoogleApiEndpoints, GoogleApiError, GoogleCredentialSource,
};
pub use drive::GoogleDriveAdapter;
pub use drive_sync::{
    DriveContentArtifact, DriveSyncBatch, DriveSyncConfig, DriveSyncHealthEvent, DriveSynchronizer,
};
pub use event::{
    ExternalContentRef, ExternalEvent, ExternalEventDraft, ExternalEventEnvelope,
    ExternalEventError, ExternalEventId, ExternalFileMetadata, ExternalObjectRef, MailChange,
    EXTERNAL_EVENT_SCHEMA_VERSION,
};
pub use gmail::{
    GmailCapability, GmailIngressCapability, GmailIngressHeader, GmailIngressMessage,
    GmailIngressPart, GoogleGmailAdapter,
};
pub use gmail_sync::{
    GmailHistorySyncConfig, GmailHistorySynchronizer, GmailSyncBatch, GmailSyncHealthEvent,
};
pub use oauth::{
    canonical_google_capability, classify_google_capability, google_capability, google_provider_id,
    is_google_read_capability, is_google_write_capability, validate_google_read_grant,
    GoogleBinding, GoogleCapability,
};
pub use source::{
    CalendarEntry, CalendarEntryPage, CalendarQuery, ExternalChangeBatch, ExternalRecordRef,
    ExternalSourceContractError, MailMessage, MailMessagePage, MailMessageSummary, MailQuery,
    OpaqueCursor, OpaqueProviderObjectId, MAX_EXTERNAL_OBJECT_ID_BYTES,
};
pub use tools::{
    GoogleAccountResolver, GoogleCalendarListTool, GoogleGmailReadTool, GoogleGmailSearchTool,
};
