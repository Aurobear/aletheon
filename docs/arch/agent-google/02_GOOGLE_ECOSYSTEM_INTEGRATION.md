# Google Ecosystem Integration

> **Status:** Proposed  
> **Scope:** Gmail, Calendar, Drive, Contacts, Tasks, OAuth and synchronization

## 1. Architectural Position

Google is an external capability provider and information ecosystem.

Correct path:

```text
Native Cognit
    ↓
CognitiveAction::InvokeCapability
    ↓
Executive permission and approval checks
    ↓
Google Capability Adapter
    ↓
Google API
```

Cognit must never receive raw Google tokens.

## 2. Module Structure

```text
Google Integration
├── Google Identity
├── Google Credential Vault
├── Google Sync Manager
├── Gmail Capability
├── Calendar Capability
├── Drive Capability
├── Contacts Capability
└── Tasks Capability
```

## 3. Identity and Authorization

Responsibilities:

- OAuth authorization code flow;
- access token refresh;
- encrypted refresh token storage;
- scope management;
- multiple Google accounts;
- token revocation;
- identity linking;
- incremental authorization.

```rust
pub struct ExternalIdentity {
    pub id: ExternalIdentityId,
    pub principal: PrincipalId,
    pub provider: IdentityProvider,
    pub provider_subject: String,
    pub email: Option<String>,
}
```

```rust
pub struct CapabilityGrant {
    pub principal: PrincipalId,
    pub provider: ProviderId,
    pub account: ExternalIdentityId,
    pub scopes: Vec<ExternalScope>,
    pub granted_at: Timestamp,
    pub revoked_at: Option<Timestamp>,
}
```

Use least privilege. Begin with read-only Gmail and Calendar access. Request write scopes only when the user asks for write actions.

## 4. Capability Interfaces

```rust
#[async_trait]
pub trait GmailCapability {
    async fn search_messages(
        &self,
        principal: PrincipalId,
        query: GmailQuery,
    ) -> Result<Vec<MessageSummary>>;

    async fn read_message(
        &self,
        principal: PrincipalId,
        id: GmailMessageId,
    ) -> Result<MailMessage>;

    async fn create_draft(
        &self,
        principal: PrincipalId,
        draft: DraftRequest,
    ) -> Result<DraftId>;

    async fn send_message(
        &self,
        principal: PrincipalId,
        request: SendMailRequest,
    ) -> Result<MessageId>;
}
```

```rust
#[async_trait]
pub trait CalendarCapability {
    async fn list_events(
        &self,
        principal: PrincipalId,
        range: TimeRange,
    ) -> Result<Vec<CalendarEvent>>;

    async fn create_event(
        &self,
        principal: PrincipalId,
        request: CreateEventRequest,
    ) -> Result<EventId>;
}
```

## 5. Google Sync Manager

```rust
pub struct GoogleSyncManager {
    pub accounts: AccountRegistry,
    pub cursors: SyncCursorStore,
    pub subscriptions: SubscriptionRegistry,
    pub event_sink: EventSink,
}
```

Responsibilities:

- Gmail history cursor;
- Calendar sync token;
- Drive change cursor;
- subscription renewal;
- retry and backoff;
- event deduplication;
- normalization into Aletheon events;
- local projection updates.

## 6. Gmail Roles

### Information source

- important mail;
- project history;
- commitments and deadlines;
- people and organization context.

### Asynchronous task entry

Suggested subjects:

```text
[ASK]
[GOAL]
[MEMORY]
[DOC]
```

Email-triggered Goals enter `Draft` or `AwaitingHuman` by default.

### Formal output

Aletheon may create drafts or send Goal completion reports, daily summaries, architecture reports and documents. Sending remains approval-controlled.

## 7. Calendar

Calendar may:

- list the daily schedule;
- derive deadlines;
- wake suspended Goals;
- detect conflicts;
- prepare meetings;
- associate events with projects and commitments.

Calendar records are external projections, not direct Mnemosyne truth.

## 8. Drive

Use incremental change tracking instead of rescanning all files.

```text
Drive cursor
    ↓
Changed metadata
    ↓
Policy check
    ↓
Download or ignore
    ↓
Artifact ingestion
    ↓
Mnemosyne/GBrain when valuable
```

Not every Drive file belongs in long-term memory.

## 9. Normalized Events

```rust
pub enum GoogleEvent {
    MailReceived(MailReceived),
    MailUpdated(MailUpdated),
    CalendarEventCreated(CalendarEvent),
    CalendarEventUpdated(CalendarEvent),
    CalendarEventDeleted(ExternalEventId),
    DriveFileCreated(DriveFileMetadata),
    DriveFileUpdated(DriveFileMetadata),
    DriveFileDeleted(ExternalFileId),
    ContactUpdated(ContactRecord),
}
```

Events may update a projection, wake a Goal, create a notification, propose a memory or request approval.

## 10. Data Boundaries

### External Store

Provider data and sync state.

### Agora

Only task-relevant projections such as today's meetings or current Goal documents.

### Mnemosyne/GBrain

Durable knowledge such as decisions, commitments, recurring patterns and lessons.

### Dasein

Identity, values and long-term commitments. Google data may propose changes but cannot commit them automatically.

## 11. Security Invariants

1. Tokens are encrypted at rest.
2. Refresh tokens never enter model context.
3. Tokens never enter GBrain.
4. Logs redact secrets and sensitive payloads.
5. Read and write permissions are distinct.
6. Destructive operations require approval.
7. Email sender validation is mandatory.
8. Events are deduplicated.
9. Every imported fact preserves provenance.
10. Old information retains temporal metadata.

## 12. MVP

First stage:

```text
Google OAuth
Gmail read-only
Calendar read-only
manual refresh
basic mail search
basic event listing
```

Second stage:

```text
Gmail drafts
Calendar writes
selected Drive files
incremental synchronization
Telegram approval
```
