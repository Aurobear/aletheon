//! First-party Google capability adapters.

pub mod calendar;
pub mod calendar_sync;
pub mod client;
pub mod drive;
pub mod drive_sync;
pub mod gmail;
pub mod gmail_sync;
pub mod oauth;
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
pub use gmail::{
    GmailCapability, GmailIngressCapability, GmailIngressHeader, GmailIngressMessage,
    GmailIngressPart, GoogleGmailAdapter,
};
pub use gmail_sync::{
    GmailHistorySyncConfig, GmailHistorySynchronizer, GmailSyncBatch, GmailSyncHealthEvent,
};
pub use tools::{
    GoogleAccountResolver, GoogleCalendarListTool, GoogleGmailReadTool, GoogleGmailSearchTool,
};
