# Channel and Mobile Communication Architecture

> **Status:** Proposed  
> **Decision:** A dedicated mobile app is not required for the first release.

## 1. Channel Roles

```text
Telegram
├── real-time conversation
├── /goal commands
├── progress updates
├── approval buttons
├── notifications
├── voice, image and file input
└── simple status queries

Gmail
├── asynchronous task entry
├── long-form input
├── formal reports
├── drafts and sending
└── project information source

Web / PWA
├── Goal DAG
├── logs
├── diffs
├── memory inspection
├── configuration
└── complex approvals

CLI / TUI
├── local development
├── debugging
├── administration
└── direct system inspection
```

## 2. Channel Core

```rust
pub struct InboundMessage {
    pub id: MessageId,
    pub channel: ChannelId,
    pub principal: PrincipalId,
    pub conversation: ConversationId,
    pub content: MessageContent,
    pub attachments: Vec<ArtifactRef>,
    pub received_at: Timestamp,
    pub reply_to: Option<MessageId>,
}
```

```rust
pub struct OutboundMessage {
    pub conversation: ConversationId,
    pub content: MessageContent,
    pub actions: Vec<UserAction>,
    pub reply_to: Option<MessageId>,
}
```

Channel adapters translate provider-specific messages into Aletheon protocol types.

## 3. Telegram

Recommended commands:

```text
/start
/chat
/goal <objective>
/goals
/status <goal-id>
/pause <goal-id>
/resume <goal-id>
/cancel <goal-id>
/approve <request-id>
/reject <request-id>
```

### Transport

Use long polling for the first home deployment.

Advantages:

- no public IP;
- no webhook;
- no inbound port;
- compatible with private home deployment.

Move to webhook only for public or multi-user deployments.

### Identity

Bind by immutable Telegram user ID, not username.

```rust
pub struct TelegramPrincipalBinding {
    pub telegram_user_id: i64,
    pub principal_id: PrincipalId,
    pub status: BindingStatus,
}
```

Default policy: only the configured owner is accepted.

## 4. Approval Model

Telegram is the preferred approval surface.

Example:

```text
Pi changed 4 files.
Tests passed.
Risk: medium.

[Apply] [View Diff] [Request Revision] [Reject]
```

Require approval for:

- sending mail;
- deleting files;
- modifying Calendar;
- Git push or merge;
- dangerous shell commands;
- capability expansion;
- Dasein modification;
- high-cost budget expansion.

## 5. Gmail Channel

```text
Incoming email
    ↓
Gmail Adapter
    ↓
InboundMessage
    ↓
classification
    ↓
conversation, Goal draft, document import or memory proposal
```

Security:

- sender allowlist;
- attachment limits;
- no direct execution from unknown mail;
- Telegram approval for high-risk requests;
- message ID deduplication.

## 6. Web/PWA

A Web/PWA is optional and should focus on complex inspection:

- Goal list and details;
- task DAG;
- attempts;
- diffs;
- approvals;
- memory search;
- system health.

It talks only to Aletheon APIs, never directly to Pi, GBrain or Google.

## 7. API and Events

Suggested HTTP operations:

```text
POST /api/v1/messages
POST /api/v1/goals
GET  /api/v1/goals
GET  /api/v1/goals/{id}
POST /api/v1/goals/{id}/pause
POST /api/v1/goals/{id}/resume
POST /api/v1/approvals/{id}/approve
POST /api/v1/approvals/{id}/reject
GET  /api/v1/artifacts/{id}
```

Suggested real-time events:

```rust
pub enum ServerEvent {
    MessageDelta(MessageDelta),
    MessageCompleted(MessageCompleted),
    GoalCreated(GoalSnapshot),
    GoalUpdated(GoalSnapshot),
    TaskStarted(TaskSnapshot),
    TaskCompleted(TaskSnapshot),
    SubagentStarted(SubagentStatus),
    SubagentCompleted(SubagentReportSummary),
    ApprovalRequested(ApprovalRequest),
    Notification(Notification),
    Error(ProtocolError),
}
```

## 8. Attachments

```rust
pub struct ArtifactRef {
    pub id: ArtifactId,
    pub owner: PrincipalId,
    pub media_type: String,
    pub size: u64,
    pub source: ArtifactSource,
    pub storage: ArtifactStorageRef,
}
```

Provider attachments enter the artifact system before entering model context.

## 9. Reliability

Required:

- Telegram update offset persistence;
- Gmail message deduplication;
- idempotent Goal creation;
- outbound retry;
- delivery status;
- correlation IDs;
- bounded downloads;
- audit logs;
- graceful restart.

## 10. MVP Acceptance

- owner can chat through Telegram;
- `/goal` creates a persistent Goal;
- progress appears in Telegram;
- approval buttons work;
- files can be submitted;
- CLI/TUI and Telegram share core sessions;
- duplicate updates do not duplicate Goals;
- Gmail creates Goal drafts;
- unknown Telegram users and senders are rejected.
