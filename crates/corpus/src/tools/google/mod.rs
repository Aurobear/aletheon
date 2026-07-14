//! First-party Google capability adapters.

pub mod calendar;
pub mod client;
pub mod gmail;
pub mod oauth;
pub mod tools;

pub use calendar::{CalendarCapability, GoogleCalendarAdapter};
pub use client::{
    GoogleAccessToken, GoogleApiClient, GoogleApiEndpoints, GoogleApiError, GoogleCredentialSource,
};
pub use gmail::{GmailCapability, GoogleGmailAdapter};
pub use tools::{
    GoogleAccountResolver, GoogleCalendarListTool, GoogleGmailReadTool, GoogleGmailSearchTool,
};
