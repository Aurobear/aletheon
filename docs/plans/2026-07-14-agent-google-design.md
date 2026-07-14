# Aletheon Agent-Google Integration — Design (Revised)

> **Status:** Proposed (Revised 2026-07-14)
> **Based on:** `docs/arch/agent-google/` (all six architecture documents)
> **Principle:** Native Cognit remains primary; external systems are adapters, capabilities, channels, or supervised subagents.
> **Key Decision:** No new crates. Extend existing crates (cognit, executive, dasein, agora, mnemosyne, corpus) with modules.

## 2. Architecture Overview

```text
                            Google Ecosystem
              Gmail / Calendar / Drive / Contacts / Tasks
                                   │
                              OAuth 2.0
                                   │
                                   ▼
┌──────────────────────────────────────────────────────────────────────┐
│                         Aletheon Server                              │
│                                                                      │
│  ┌────────────────────────────────────────────┐                      │
│  │          Channel Layer                     │                      │
│  │  ┌──────────────┐  ┌────────────────────┐  │                      │
│  │  │ ChannelCore   │  │ TelegramChannel    │  │                      │
│  │  │ (trait + types)│  │ (long-polling)    │  │                      │
│  │  └──────────────┘  └────────────────────┘  │                      │
│  │  ┌──────────────────────────────────────┐  │                      │
│  │  │ GmailChannel (Phase G)               │  │                      │
│  │  └──────────────────────────────────────┘  │                      │
│  └────────────────────────────────────────────┘                      │
│                                                                      │
│  ┌────────────────────────────────────────────┐                      │
│  │          Goal Runtime                      │                      │
│  │  ┌──────────────┐  ┌────────────────────┐  │                      │
│  │  │ GoalSupervisor│  │ DeepSeekWorker     │  │                      │
│  │  │ (create/tick/ │  │ (GoalWorker impl)  │  │                      │
│  │  │  pause/resume)│  └────────────────────┘  │                      │
│  │  └──────────────┘  ┌────────────────────┐  │                      │
│  │  ┌──────────────┐  │ PiWorker           │  │                      │
│  │  │Verification   │  │ (coding subagent)  │  │                      │
│  │  │ Pipeline      │  └────────────────────┘  │                      │
│  │  └──────────────┘  ┌────────────────────┐  │                      │
│  │  ┌──────────────┐  │ RetryPolicy +      │  │                      │
│  │  │ MemoryExtract │  │ EscalationPolicy   │  │                      │
│  │  └──────────────┘  └────────────────────┘  │                      │
│  └────────────────────────────────────────────┘                      │
│                                                                      │
│  ┌────────────────────────────────────────────┐                      │
│  │         Google Integration                 │                      │
│  │  ┌──────────────┐  ┌────────────────────┐  │                      │
│  │  │ GoogleIdentity│  │ CredentialVault    │  │                      │
│  │  │ (OAuth flow) │  │ (encrypted tokens) │  │                      │
│  │  └──────────────┘  └────────────────────┘  │                      │
│  │  ┌──────────────┐  ┌────────────────────┐  │                      │
│  │  │ GmailCapability│ │ CalendarCapability │  │                      │
│  │  │ (read/search/ │  │ (list/create)      │  │                      │
│  │  │  draft/send)  │  └────────────────────┘  │                      │
│  │  └──────────────┘  ┌────────────────────┐  │                      │
│  │  ┌──────────────┐  │ DriveCapability    │  │                      │
│  │  │GoogleSyncMgr  │  │ (deferred)         │  │                      │
│  │  │ (cursors+     │  └────────────────────┘  │                      │
│  │  │  events)      │                          │                      │
│  │  └──────────────┘                          │                      │
│  └────────────────────────────────────────────┘                      │
│                                                                      │
│  ┌────────────────────────────────────────────┐                      │
│  │       Mnemosyne / GBrain Backend           │                      │
│  │  ┌──────────────┐  ┌────────────────────┐  │                      │
│  │  │ GBrainClient  │  │ IngestionPipeline  │  │                      │
│  │  │ (REST client) │  │ (spawn+buffer+     │  │                      │
│  │  └──────────────┘  │  flush)             │  │                      │
│  │  ┌──────────────┐  └────────────────────┘  │                      │
│  │  │ RecallQuery   │  ┌────────────────────┐  │                      │
│  │  └──────────────┘  │ AgoraProjection    │  │                      │
│  │                    └────────────────────┘  │                      │
│  └────────────────────────────────────────────┘                      │
│                                                                      │
│  ┌────────────────────────────────────────────┐                      │
│  │       Existing Unchanged Core              │                      │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐   │                      │
│  │  │ Native   │ │ Executive│ │ Dasein   │   │                      │
│  │  │ Cognit   │ │ (sandbox,│ │ (identity│   │                      │
│  │  │          │ │  policy) │ │  values) │   │                      │
│  │  └──────────┘ └──────────┘ └──────────┘   │                      │
│  │  ┌──────────┐ ┌──────────────────────────┐ │                      │
│  │  │ Agora    │ │ ModelRouter+LlmScheduler │ │                      │
│  │  │(scratch- │ │ (provider selection)     │ │                      │
│  │  │ pad)     │ └──────────────────────────┘ │                      │
│  │  └──────────┘                              │                      │
│  └────────────────────────────────────────────┘                      │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
                                   │
                ┌──────────────────┼──────────────────┐
                ▼                  ▼                  ▼
        ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
        │ PostgreSQL   │  │ GBrain       │  │ Tailscale    │
        │ localhost:   │  │ localhost:   │  │ (secure mesh)│
        │ 5432         │  │ 9800         │  │              │
        └──────────────┘  └──────────────┘  └──────────────┘
```

## 3. Phase Map

| Phase | Name | Duration | Crates Touched |
|-------|------|----------|----------------|
| A | Channel Core + Telegram | Week 1-2 | executive (channel module) |
| B | Goal Runtime v1 | Week 2-4 | executive (goal module) |
| C | DeepSeek Worker + Retry | Week 4-5 | cognit (worker), executive (retry) |
| D | Pi Coding Subagent | Week 5-6 | corpus (tools), executive (subagent) |
| E | Verification Pipeline | Week 6-7 | executive (verification) |
| F | Google Read-Only + Sync | Week 7-9 | executive (google module) |
| G | Gmail Channel + Approval | Week 9-10 | executive (channel/gmail) |
| H | GBrain Mnemosyne Backend | Week 10-12 | mnemosyne (backends/gbrain) |

## 4. Key Changes from Original Design

1. **No new crates.** All new modules live inside existing crates (`executive`, `cognit`, `corpus`, `mnemosyne`) rather than introducing a separate `aletheon-*` crate family. This avoids initial workspace sprawl and keeps integration boundaries obvious.
2. **Phase 0 removed.** The original Phase 0 ("Preserve Native Agent") is no longer a separate phase. The existing Native Cognit entrypoint and DeepSeek path are already stable as of 2026-07-14. New modules extend without rewriting existing paths.
3. **ObjectiveStore extended, not replaced.** The existing `Agora` workspace (scratchpad, blackboard, task_graph) stores active Goal state. A new `GoalStore` within `executive` provides durable persistence, but Agora remains the live working-memory surface.
4. **State machine simplified.** The original 11-state `GoalState` enum collapses to 10: `Clarifying` merges into `Draft` (clarification is the first tick behavior). `Verifying` is not a distinguishable top-level state but a transient phase within `Running`.
5. **Google consolidated into `executive`.** Instead of separate `aletheon-google-core`, `aletheon-google-gmail`, `aletheon-google-calendar`, `aletheon-google-drive` crates, all Google integration lives under `crates/executive/src/impl/google/` as modules.
6. **Deployment is continuous, not a separate phase.** The original Phase 11 ("Deployment Hardening") items (systemd, Docker Compose, backups, Tailscale) are addressed incrementally during Phase H and beyond, not as a gated milestone.
7. **Web dashboard deferred indefinitely.** The original Phase 10 is moved to Non-Goals. The first deployment relies entirely on Telegram, CLI/TUI, and Gmail for interaction.

## 5. Phase A: Channel Core + Telegram

### Code Layout

```text
crates/executive/src/impl/channel/
├── mod.rs              -- re-exports
├── core.rs             -- Channel trait, InboundMessage, OutboundMessage, MessageContent, UserAction
├── telegram.rs         -- TelegramChannel: long-polling, command dispatch, offset persistence
└── gmail.rs            -- stub; implemented in Phase G
```

### Core Types

```rust
/// The Channel trait is the single interface every message channel must implement.
#[async_trait]
pub trait Channel: Send + Sync {
    /// Unique channel identifier (e.g. "telegram", "gmail").
    fn channel_id(&self) -> ChannelId;

    /// Start receiving messages. Returns a stream of InboundMessage.
    async fn start(&mut self) -> Result<BoxStream<'static, InboundMessage>>;

    /// Send an outbound message (text, buttons, attachments).
    async fn send(&self, msg: OutboundMessage) -> Result<()>;

    /// Acknowledge receipt of a message so the channel can advance its cursor.
    async fn ack(&self, msg_id: MessageId) -> Result<()>;
}

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

pub struct OutboundMessage {
    pub conversation: ConversationId,
    pub content: MessageContent,
    pub actions: Vec<UserAction>,
    pub reply_to: Option<MessageId>,
}

pub enum MessageContent {
    Text(String),
    Markdown(String),
    Voice(VoiceRef),
    Image(ImageRef),
    File(FileRef),
}

pub struct UserAction {
    pub id: ActionId,
    pub label: String,
    pub action_type: ActionType,
}

pub enum ActionType {
    Callback { data: String },
    Url { url: String },
    Approve { request_id: String },
    Reject { request_id: String },
}
```

### Telegram Commands

```text
/start           -- register and bind principal
/chat <msg>      -- freeform conversation with Native Cognit
/goal <objective>-- create a persistent Goal
/goals           -- list active Goals
/status <id>     -- show Goal details and progress
/pause <id>      -- suspend a Goal
/resume <id>     -- resume a suspended Goal
/cancel <id>     -- cancel a Goal
/approve <req>   -- approve a pending request
/reject <req>    -- reject a pending request
```

### Integration Notes

- Telegram uses long polling (no public IP required). The poll loop runs in a Tokio task spawned by `TelegramChannel::start()`.
- Identity binding is based on immutable Telegram user ID (`i64`), not username.
- Default policy: only the single configured owner principal is accepted. Unknown users receive a rejection message and are logged.
- Update offset is persisted to disk so restarts do not replay messages.
- The `ack()` method advances the offset after the inbound message has been fully processed by Executive.

### Phase A Acceptance Criteria

1. Owner can send `/start` and receive a greeting in Telegram.
2. Owner can send `/chat <msg>` and receive a response from Native Cognit.
3. Telegram long polling survives a server restart (no message loss or duplication).
4. Unknown Telegram users are rejected with an audit log entry.
5. Offset persistence survives process restart.

## 6. Phase B: Goal Runtime v1

### Goal State Machine

```text
                        ┌──────────┐
                        │  Draft   │
                        └────┬─────┘
                             │ compile intent
                             ▼
                        ┌──────────┐
              ┌────────│ Planned  │────────┐
              │ cancel └────┬─────┘ cancel │
              ▼             │ run          ▼
         ┌──────────┐      ▼         ┌──────────┐
         │Cancelled │ ┌──────────┐   │Cancelled │
         └──────────┘ │ Running  │───┘          │
                      └──┬───┬───┘
                         │   │ pause
          ┌──────────────┘   ▼
          │           ┌──────────┐
          │ complete  │Suspended │
          ▼           └────┬─────┘
    ┌───────────┐          │ resume
    │ Completed │          ▼
    └───────────┘     ┌──────────┐
                      │ Running  │
          ┌───────────┴──────────┘
          │ fail / exhausted
          ▼
    ┌───────────┐     ┌──────────────┐
    │  Failed   │     │AwaitingHuman │
    └───────────┘     └──────┬───────┘
                             │ resolve
                             ▼
                        ┌──────────┐
                        │ Running  │
                        └──────────┘

         ┌───────────┐
         │  Blocked   │ (dependency or capability unavailable)
         └───────────┘
```

### GoalSupervisor Interface

```rust
#[async_trait]
pub trait GoalSupervisor: Send + Sync {
    /// Create a new Goal from a user specification.
    /// Returns the GoalId for future operations.
    async fn create(
        &self,
        spec: GoalSpecification,
    ) -> Result<GoalId>;

    /// Advance a Goal by one tick. A tick represents a bounded unit of work:
    /// compile intent, plan, dispatch a task, check results, etc.
    /// Returns the transition that occurred (or None if idle).
    async fn tick(
        &self,
        goal_id: GoalId,
    ) -> Result<Option<GoalTransition>>;

    /// Pause an active Goal. State moves to Suspended.
    async fn pause(&self, goal_id: GoalId) -> Result<()>;

    /// Resume a Suspended Goal. State moves back to Running.
    async fn resume(&self, goal_id: GoalId) -> Result<()>;

    /// Cancel a Goal. Irreversible. State moves to Cancelled.
    async fn cancel(&self, goal_id: GoalId) -> Result<()>;

    /// List all Goals for a principal, optionally filtered by state.
    async fn list(
        &self,
        principal: PrincipalId,
        filter: Option<GoalStateFilter>,
    ) -> Result<Vec<GoalSummary>>;
}
```

### GoalFrame Struct

```rust
pub struct GoalFrame {
    pub original_intent: String,
    pub desired_state: DesiredState,
    pub constraints: Vec<Constraint>,
    pub acceptance_criteria: Vec<AcceptanceCriterion>,
    pub current_plan: PlanSummary,
    pub current_task: PlannedTask,
    pub recent_attempts: Vec<AttemptSummary>,
    pub relevant_memories: Vec<MemoryProjection>,
    pub remaining_budget: GoalBudget,
}
```

`MemoryProjection` replaces the original `ScoredMemory` and includes provenance, freshness, and temporal metadata. This ensures workers see memories with explicit source and recency information rather than opaque scores.

### Safety Requirements

Every Goal must carry:

- Budget limit (token and cost caps).
- Time limit (wall-clock deadline).
- Attempt limit (maximum retries per task before escalation).
- Capability boundary (what the Goal is allowed to invoke).
- Workspace boundary (filesystem scope, typically a temp worktree).
- Pause and cancellation hooks (must be responsive within 5 seconds).
- Audit log (every state transition and task dispatch recorded).
- Approval policy (which operations require human approval).
- Completion criteria (objective evidence conditions).
- Escalation policy (when to request human intervention or change models).

### Phase B Acceptance Criteria

1. A Goal created via `/goal` is persisted and survives process restart.
2. The `tick()` loop advances a Goal from Draft through Planned to Running.
3. The original intent text is immutable after creation.
4. `pause` / `resume` / `cancel` transitions work correctly and are recorded in the audit log.
5. A Goal that exhausts its attempt limit transitions to Failed (not an infinite loop).
6. Goal state is queryable through Telegram `/status` and `/goals`.

## 7. Phase C: DeepSeek Worker + Retry

### GoalWorker Trait

```rust
#[async_trait]
pub trait GoalWorker: Send + Sync {
    /// Human-readable name for logs and reports.
    fn name(&self) -> &'static str;

    /// The cognitive role this worker fills.
    fn role(&self) -> CognitiveRole;

    /// Execute one attempt on a task within a Goal.
    /// Receives the full GoalFrame for context.
    async fn execute(
        &self,
        frame: &GoalFrame,
        task: &PlannedTask,
        evidence: &[EvidenceRef],
    ) -> Result<WorkerOutput>;
}

pub struct WorkerOutput {
    pub success: bool,
    pub artifacts: Vec<ArtifactRef>,
    pub evidence: Vec<EvidenceRef>,
    pub summary: String,
    pub token_usage: TokenUsage,
    pub duration: Duration,
}
```

### Failure Classification

```rust
pub enum ClassifiedFailure {
    Compilation { errors: Vec<String> },
    TestFailure { failed: Vec<String> },
    PermissionDenied { required: String },
    Timeout { limit: Duration, actual: Duration },
    MissingDependency { crate_or_tool: String },
    InvalidAssumption { assumption: String },
    ArchitectureViolation { rule: String, detail: String },
    ToolFailure { tool: String, stderr: String },
    ContextInsufficient { missing: String },
    RepeatedFailure { same_failure_count: u32 },
}
```

### Failure Class to Strategy Mapping

| FailureClass | Strategy | Next Executor |
|---|---|---|
| Compilation | Retry with compiler error text as evidence | Same worker (DeepSeek) |
| TestFailure | Retry with test output as evidence | Same worker (DeepSeek) |
| Timeout | Shrink task scope or increase timeout | Same or GPT/Opus |
| MissingDependency | Add dependency to task context, retry | Same worker |
| InvalidAssumption | Query Mnemosyne or ask user, replan | GPT/Opus |
| ArchitectureViolation | Replan or review | GPT/Opus |
| RepeatedFailure | Shrink task, change executor, or escalate | GPT/Opus or AwaitingHuman |

### Retry Defaults

```text
1. Attempt 1 fails → same worker receives failure evidence as additional context.
2. Same failure class on Attempt 2 → strategy switch (parameter change or task shrink).
3. Same failure class on Attempt 3 → escalate to GPT/Opus for root-cause analysis.
4. Still unresolved after escalation → Goal transitions to Blocked (dependency unavailable) or AwaitingHuman (needs user decision).
```

No unbounded retries. The attempt limit is configurable per Goal (default: 3 + 1 escalation).

### Phase C Acceptance Criteria

1. DeepSeek worker receives a properly constructed GoalFrame with original intent visible.
2. Worker output is captured as an Attempt record with token usage and duration.
3. Compilation failures are classified correctly and retried with compiler evidence.
4. Repeated failures of the same class escalate to GPT/Opus after 3 attempts.
5. ContextInsufficient failure triggers a Mnemosyne query or user prompt.
6. PermissionDenied is logged as a policy event and does not retry blindly.
7. Token usage and cost are accounted per attempt and per Goal.

## 8. Phase D: Pi Coding Subagent

### PiSubagentReport Struct

```rust
pub struct PiSubagentReport {
    pub task_id: TaskId,
    pub success: bool,
    pub exit_code: i32,
    pub stdout_summary: String,
    pub stderr_summary: String,
    pub files_changed: Vec<ChangedFile>,
    pub diff_patch: Option<String>,
    pub warnings: Vec<String>,
    pub token_usage: TokenUsage,
    pub duration: Duration,
}

pub struct ChangedFile {
    pub path: RelativePath,
    pub change_type: ChangeType,  // Added, Modified, Deleted
    pub lines_added: u32,
    pub lines_removed: u32,
}
```

### Security Invariants

1. Pi runs in an **isolated temporary worktree**, never in the main working copy.
2. Pi has **no network access** by default (enforced via sandbox/namespace).
3. Pi cannot modify Dasein, Executive policy, or any `config/` directory.
4. Pi's stdout and stderr are **fully captured** and attached to the Attempt record.
5. A **hard timeout** (default: 5 minutes per task) is enforced with SIGKILL.
6. All file changes are collected as a **diff** that must pass verification before merging.

### Phase D Acceptance Criteria

1. Pi spawns in an isolated worktree (verify via `git worktree list`).
2. Pi cannot access the main worktree filesystem (enforced by sandbox).
3. Timeout kills the Pi process and the attempt is recorded as Failed (Timeout).
4. Stdout, stderr, and exit status are captured in the SubagentReport.
5. Diff is collected and does not include files outside the allowed scope.
6. Native Cognit reviews the SubagentReport before any merge decision.

## 9. Phase E: Verification Pipeline

### Verification Gates

| Gate | Type | Check Description |
|---|---|---|
| Format | Auto | `cargo fmt --check` on changed files |
| Compile | Auto | `cargo check --workspace` (may be scoped to affected crates) |
| Test | Auto | `cargo test` for affected crates (unit + integration) |
| Clippy | Auto | `cargo clippy -- -D warnings` on changed crates |
| DiffScope | Policy | Changed files must be within the task's allowed scope |
| Architecture | Review | No crate dependency inversion, no forbidden imports |
| CapabilityPolicy | Review | No capability expansion without approval |

### VerificationAction Enum

```rust
pub enum VerificationAction {
    /// Gate passed. Continue to the next gate.
    Pass,

    /// Gate failed but is auto-fixable. Record failure and proceed.
    FailAutoFixable { reason: String },

    /// Gate failed. Return evidence to the worker for retry.
    FailRetry { reason: String, evidence: String },

    /// Gate failed. This task cannot proceed. Escalate or block.
    FailBlock { reason: String },
}
```

The verification pipeline runs as a linear sequence. Each gate produces a `VerificationAction`. `FailBlock` stops the pipeline immediately. `FailRetry` returns evidence for the next worker attempt. `FailAutoFixable` records the failure but continues (useful for non-blocking lints).

### Phase E Acceptance Criteria

1. All 7 gates run in order for every code-change task.
2. Format failures are auto-fixable (recorded but not blocking).
3. Compile failures return the full error text as retry evidence.
4. Test failures provide per-test failure output.
5. DiffScope violations block the task immediately (FailBlock).

## 10. Phase F: Google Read-Only + Sync

### Security Architecture

```text
┌─────────────────────────────────────────────────────────┐
│                   Google API Boundary                    │
│                                                         │
│  Google APIs (external, over TLS)                       │
│       │                                                 │
│       ▼                                                 │
│  ┌─────────────────────────────────────────────┐        │
│  │           GoogleDriver (barrier)            │        │
│  │                                             │        │
│  │  - OAuth token management                   │        │
│  │  - Credential vault (encrypted at rest)     │        │
│  │  - Request sanitization                     │        │
│  │  - Rate limiting and retry                  │        │
│  │  - Response normalization                   │        │
│  │                                             │        │
│  │  INVARIANT: tokens never cross this line →  │        │
│  └─────────────────────────────────────────────┘        │
│       │                                                 │
│       ▼  (no tokens, only CapabilityGrant references)    │
│  ┌─────────────────────────────────────────────┐        │
│  │          Aletheon Internal                  │        │
│  │                                             │        │
│  │  GoogleIdentity    GoogleSyncManager        │        │
│  │  GmailCapability   CalendarCapability       │        │
│  │  DriveCapability   CredentialVault          │        │
│  └─────────────────────────────────────────────┘        │
└─────────────────────────────────────────────────────────┘
```

### Security Invariants

1. Tokens are encrypted at rest (using a local key derived from a secrets file outside the repo).
2. Refresh tokens never enter model context (not in GoalFrame, not in any prompt).
3. Tokens never enter GBrain (not stored in the knowledge backend).
4. Logs redact OAuth tokens and sensitive payloads (redaction applied before write).
5. Read and write permissions are distinct scopes (granted incrementally).
6. Destructive operations require approval (send mail, modify calendar, delete drive files).
7. Email sender validation is mandatory before any action (SPF/DKIM-validated or allow-listed).

### Incremental Sync Strategy

```text
┌────────────────┐
│ Sync Cursor    │  (per account, per service: Gmail historyId, Calendar syncToken, Drive changeToken)
└───────┬────────┘
        │ poll (interval: configurable, default 60s)
        ▼
┌────────────────┐
│ Fetch Changes  │  (incremental since last cursor)
└───────┬────────┘
        │
        ▼
┌────────────────┐
│ Deduplicate    │  (messageId / eventId already seen → skip)
└───────┬────────┘
        │
        ▼
┌────────────────┐
│ Normalize      │  (Google-specific → GoogleEvent enum)
└───────┬────────┘
        │
        ▼
┌────────────────┐
│ Dispatch       │  (event → notification, Goal wakeup, memory proposal, Agora projection)
└───────┬────────┘
        │
        ▼
┌────────────────┐
│ Advance Cursor │  (persist new cursor atomically after dispatch)
└────────────────┘
```

### Normalized Events to Actions

| GoogleEvent Variant | Aletheon Action |
|---|---|
| MailReceived(from=known_sender) | Classify subject tag ([GOAL]/[ASK]/etc.), create Goal draft or notification |
| MailReceived(from=unknown) | Log, optionally notify user; no automatic execution |
| CalendarEventCreated | Project into Agora, check for deadline implications |
| CalendarEventUpdated | Update Agora projection, wake Goals waiting on this event |
| CalendarEventDeleted | Remove Agora projection |
| DriveFileCreated | Policy check → ingest artifact → optionally propose to Mnemosyne |
| DriveFileUpdated | Same as created (update existing artifact) |
| DriveFileDeleted | Remove local artifact reference |

### MVP Scope

**Sub-phase F-1 (Read-Only Manual):**
- Google OAuth authorization code flow.
- Encrypted credential storage.
- Gmail read-only (search, read message, list inbox).
- Calendar read-only (list events in time range).
- Account binding to principal.
- Manual refresh (user-triggered, no background sync yet).
- Telegram queries: "what's on my calendar today?", "show unread important mail".

**Sub-phase F-2 (Incremental Sync):**
- Gmail history cursor (watch/poll for new messages).
- Calendar sync token (poll for event changes).
- Normalized event stream.
- Deduplication based on provider IDs.
- Automatic Agora projection updates.
- Telegram notifications for new important mail.
- Goal wakeup (a Goal can register interest in a Calendar event or email thread).

### Phase F Acceptance Criteria

1. OAuth flow completes and returns a valid access token without exposing the refresh token to logs.
2. Encrypted credentials survive process restart and are not readable as plaintext on disk.
3. `GmailCapability::search_messages` returns summaries for a query like "from:boss@company.com".
4. `CalendarCapability::list_events` returns today's events.
5. Manual refresh does not duplicate previously seen messages or events.
6. (F-2) Incremental sync detects new Gmail messages within 2 poll intervals.
7. (F-2) Calendar sync token survives restart and resumes without full re-scan.
8. (F-2) A Goal registered to watch for a calendar event is woken when the event is created.

## 11. Phase G: Gmail Channel + Approval

### Email Classification

| Subject Tag | Action | Default Behavior |
|---|---|---|
| `[GOAL] <objective>` | Create Goal draft | Enters Draft state; Telegram notification with approve/reject |
| `[ASK] <question>` | Create inquiry | Native Cognit responds; answer drafted as reply (approval required to send) |
| `[MEMORY] <text>` | Propose memory | Mnemosyne ingestion proposed; user confirms via Telegram |
| `[DOC] <text>` | Document import | Artifact stored; optionally indexed in GBrain |
| (no tag) | Classify via LLM | Low-confidence → notify user; High-confidence (known sender) → draft Goal or reply |

### ApprovalRequest Struct

```rust
pub struct ApprovalRequest {
    pub id: ApprovalId,
    pub goal_id: Option<GoalId>,
    pub operation: ApprovedOperation,
    pub risk_level: RiskLevel,  // Low, Medium, High
    pub summary: String,
    pub detail: String,
    pub artifacts: Vec<ArtifactRef>,
    pub created_at: Timestamp,
    pub expires_at: Timestamp,  // auto-reject after timeout
}
```

### Operations Requiring Approval

| Operation | Risk | Condition |
|---|---|---|
| `SendMail` | High | Always require approval |
| `DeleteFile` | High | Always require approval |
| `ModifyCalendar` | Medium | Approval unless from a pre-authorized Goal |
| `GitPush` | High | Always require approval |
| `CapabilityExpansion` | High | Always require approval |
| `DaseinModification` | Critical | Always require approval + confirmation prompt |

### Phase G Acceptance Criteria

1. `[GOAL]` email creates a Goal draft that appears in Telegram for approval.
2. `[ASK]` email triggers a Native Cognit response; the draft reply is shown before sending.
3. Unknown senders create a notification but no automatic Goal or response.
4. `ApprovalRequest` presents approve/reject buttons in Telegram.
5. Expired approval requests auto-reject and log the event.
6. `SendMail` requires explicit human approval (no autonomous sending).
7. Email message ID deduplication prevents duplicate Goal creation.

## 12. Phase H: GBrain Mnemosyne Backend + Service Integration

### Docker Compose Service Definitions

```yaml
# docker-compose.yml (in project root)
version: "3.9"

services:
  postgres:
    image: postgres:16-alpine
    container_name: aletheon-postgres
    restart: unless-stopped
    environment:
      POSTGRES_USER: ${GBRAIN_POSTGRES_USER:-gbrain}
      POSTGRES_PASSWORD: ${GBRAIN_POSTGRES_PASSWORD}
      POSTGRES_DB: ${GBRAIN_POSTGRES_DB:-gbrain}
    ports:
      - "127.0.0.1:5432:5432"
    volumes:
      - postgres_data:/var/lib/postgresql/data
    healthcheck:
      test: ["CMD-SHELL", "pg_isready -U ${GBRAIN_POSTGRES_USER:-gbrain}"]
      interval: 5s
      timeout: 5s
      retries: 5

  gbrain:
    image: ghcr.io/aurobear/gbrain:latest
    container_name: aletheon-gbrain
    restart: unless-stopped
    depends_on:
      postgres:
        condition: service_healthy
    environment:
      GBRAIN_DATABASE_URL: postgresql://${GBRAIN_POSTGRES_USER:-gbrain}:${GBRAIN_POSTGRES_PASSWORD}@postgres:5432/${GBRAIN_POSTGRES_DB:-gbrain}
      GBRAIN_LISTEN_ADDR: 0.0.0.0:9800
      GBRAIN_API_KEY: ${GBRAIN_API_KEY}
      GBRAIN_LOG_LEVEL: ${GBRAIN_LOG_LEVEL:-info}
    ports:
      - "127.0.0.1:9800:9800"
    healthcheck:
      test: ["CMD-SHELL", "curl -f http://localhost:9800/health || exit 1"]
      interval: 10s
      timeout: 5s
      retries: 5

volumes:
  postgres_data:
```

Environment file (`gbrain.env`):

```env
GBRAIN_POSTGRES_USER=gbrain
GBRAIN_POSTGRES_PASSWORD=<generated-secret>
GBRAIN_POSTGRES_DB=gbrain
GBRAIN_API_KEY=<generated-secret>
GBRAIN_LOG_LEVEL=info
```

### GBrain REST API Contract

**Base URL:** `http://localhost:9800/api/v1`

All requests include header: `Authorization: Bearer <GBRAIN_API_KEY>`

#### `GET /health`

Response `200`:
```json
{
  "status": "ok",
  "database": "connected",
  "uptime_seconds": 3600
}
```

#### `POST /knowledge/ingest`

Request:
```json
{
  "entries": [
    {
      "source": "goal_completion",
      "goal_id": "goal_abc123",
      "content_type": "architecture_decision",
      "title": "Adopt async trait for GoalSupervisor",
      "body": "The GoalSupervisor trait uses #[async_trait] because ...",
      "tags": ["goal-runtime", "architecture", "async"],
      "provenance": {
        "goal_id": "goal_abc123",
        "task_id": "task_007",
        "principal": "owner",
        "timestamp": "2026-07-14T10:00:00Z"
      },
      "temporal": {
        "valid_from": "2026-07-14T10:00:00Z",
        "valid_until": null,
        "is_current": true
      }
    }
  ]
}
```

Response `201`:
```json
{
  "ingested": 1,
  "knowledge_ids": ["k_001"]
}
```

#### `POST /knowledge/recall`

Request:
```json
{
  "query": "How should GoalSupervisor handle async execution?",
  "filters": {
    "tags": ["goal-runtime", "architecture"],
    "content_types": ["architecture_decision"],
    "is_current": true,
    "limit": 10
  }
}
```

Response `200`:
```json
{
  "results": [
    {
      "knowledge_id": "k_001",
      "title": "Adopt async trait for GoalSupervisor",
      "body": "The GoalSupervisor trait uses #[async_trait] because ...",
      "relevance": 0.92,
      "provenance": {
        "goal_id": "goal_abc123",
        "task_id": "task_007",
        "principal": "owner",
        "timestamp": "2026-07-14T10:00:00Z"
      },
      "temporal": {
        "valid_from": "2026-07-14T10:00:00Z",
        "valid_until": null,
        "is_current": true
      }
    }
  ],
  "total_matches": 1
}
```

#### `POST /knowledge/expire`

Request:
```json
{
  "knowledge_id": "k_001",
  "reason": "superseded_by",
  "superseded_by": "k_002",
  "timestamp": "2026-07-20T10:00:00Z"
}
```

Response `200`:
```json
{
  "status": "expired",
  "knowledge_id": "k_001",
  "valid_until": "2026-07-20T10:00:00Z"
}
```

#### `GET /knowledge/{knowledge_id}`

Response `200`:
```json
{
  "knowledge_id": "k_001",
  "source": "goal_completion",
  "content_type": "architecture_decision",
  "title": "Adopt async trait for GoalSupervisor",
  "body": "The GoalSupervisor trait uses #[async_trait] because ...",
  "tags": ["goal-runtime", "architecture", "async"],
  "provenance": {
    "goal_id": "goal_abc123",
    "task_id": "task_007",
    "principal": "owner",
    "timestamp": "2026-07-14T10:00:00Z"
  },
  "temporal": {
    "valid_from": "2026-07-14T10:00:00Z",
    "valid_until": null,
    "is_current": true
  },
  "created_at": "2026-07-14T10:00:00Z",
  "updated_at": "2026-07-14T10:00:00Z"
}
```

#### `POST /knowledge/search`

Request:
```json
{
  "query": "goal runtime retry",
  "limit": 5,
  "filters": {
    "is_current": true
  }
}
```

Response `200`:
```json
{
  "results": [
    {
      "knowledge_id": "k_003",
      "title": "Retry policy: 3 attempts + escalation",
      "body": "The default retry policy ...",
      "relevance": 0.88,
      "provenance": { "...": "..." },
      "temporal": { "...": "..." }
    }
  ],
  "total_matches": 1
}
```

### GBrainClient Struct

```rust
pub struct GBrainClient {
    base_url: Url,
    api_key: SecretString,
    client: reqwest::Client,
    health: Arc<AtomicBool>,
}

impl GBrainClient {
    pub fn new(config: &GBrainConfig) -> Result<Self>;

    /// Check backend health. Updates the internal health flag.
    pub async fn health_check(&self) -> Result<bool>;

    /// Ingest one or more knowledge entries.
    pub async fn ingest(&self, entries: Vec<KnowledgeEntry>) -> Result<IngestResponse>;

    /// Recall knowledge by semantic query with filters.
    pub async fn recall(&self, request: RecallRequest) -> Result<RecallResponse>;

    /// Full-text search with optional filters.
    pub async fn search(&self, request: SearchRequest) -> Result<SearchResponse>;

    /// Mark a knowledge entry as expired/superseded.
    pub async fn expire(&self, request: ExpireRequest) -> Result<ExpireResponse>;

    /// Fetch a single knowledge entry by ID.
    pub async fn get_knowledge(&self, knowledge_id: &str) -> Result<KnowledgeEntry>;

    /// Returns true if the last health check was successful.
    pub fn is_healthy(&self) -> bool;
}
```

### Configuration [gbrain] TOML Section

```toml
[gbrain]
# REST API base URL
base_url = "http://localhost:9800/api/v1"

# API key for authentication (loaded from env or secrets file)
api_key_env = "GBRAIN_API_KEY"

# Ingestion pipeline settings
ingestion_buffer_size = 100        # max entries before forced flush
ingestion_flush_interval_secs = 30 # periodic flush interval
ingestion_max_retries = 3          # retries per flush batch
ingestion_retry_backoff_ms = 1000  # base backoff

# Recall defaults
recall_default_limit = 10
recall_max_limit = 50

# Health check
health_check_interval_secs = 30
health_check_timeout_secs = 5

# Graceful degradation: if disabled, recall returns empty
enabled = true
```

### Startup Ordering

```text
1. PostgreSQL starts (systemd or docker compose).
   ├── Health check: pg_isready returns 0.
   └── GBrain waits for this condition.

2. GBrain starts.
   ├── Runs database migrations.
   ├── Health check: GET /health returns 200.
   └── Aletheon waits for this condition.

3. Aletheon starts.
   ├── GBrainClient::new() reads [gbrain] config.
   ├── On startup, health_check() verifies backend reachable.
   ├── If health check fails → GBrainBackend enters degraded mode (recall returns empty, ingest queues locally).
   └── If health check later succeeds → GBrainBackend recovers automatically.

Systemd ordering (aletheon.service):
   After=docker.service postgresql.service
   Requires=docker.service
```

### GBrainBackend Implementation

```rust
pub struct GBrainBackend {
    client: GBrainClient,
    ingestion: IngestionPipeline,
    config: GBrainConfig,
}

#[async_trait]
impl MnemosyneBackend for GBrainBackend {
    async fn store(&self, entry: MnemosyneEntry) -> Result<KnowledgeId> {
        // Convert MnemosyneEntry → KnowledgeEntry, queue in ingestion pipeline.
        self.ingestion.enqueue(entry.into()).await
    }

    async fn recall(&self, query: &str, context: &RecallContext) -> Result<Vec<MemoryProjection>> {
        if !self.client.is_healthy() {
            // Degraded mode: return empty, log warning.
            tracing::warn!("GBrain unhealthy; recall returning empty");
            return Ok(vec![]);
        }
        let request = RecallRequest::from_query(query, context);
        let response = self.client.recall(request).await?;
        Ok(response.results.into_iter().map(MemoryProjection::from).collect())
    }

    async fn expire(&self, knowledge_id: &KnowledgeId, reason: ExpiryReason) -> Result<()> {
        let request = ExpireRequest { knowledge_id: knowledge_id.clone(), reason, timestamp: now() };
        self.client.expire(request).await?;
        Ok(())
    }

    async fn health(&self) -> Result<BackendHealth> {
        self.client.health_check().await?;
        Ok(BackendHealth {
            backend: "gbrain".into(),
            healthy: self.client.is_healthy(),
            ingestion_queue_depth: self.ingestion.queue_depth(),
        })
    }
}
```

### IngestionPipeline

```rust
pub struct IngestionPipeline {
    buffer: Arc<Mutex<Vec<KnowledgeEntry>>>,
    flush_tx: tokio::sync::mpsc::UnboundedSender<()>,
    client: GBrainClient,  // clone of the shared client
}

impl IngestionPipeline {
    /// Spawn a background task that:
    /// 1. Watches for flush signals (buffer full or timer expired).
    /// 2. Batches entries and calls client.ingest().
    /// 3. Retries on transient failures with exponential backoff.
    /// 4. Logs persistent failures (entries remain in local buffer for retry).
    pub fn spawn(client: GBrainClient, config: &GBrainConfig) -> Self;

    /// Queue a single entry. Triggers flush if buffer is full.
    pub async fn enqueue(&self, entry: KnowledgeEntry) -> Result<()>;

    /// Force immediate flush of all buffered entries.
    pub async fn flush(&self) -> Result<usize>;

    /// Number of entries currently buffered.
    pub fn queue_depth(&self) -> usize;
}
```

The pipeline uses:
- `spawn` → background Tokio task with `tokio::select!` on flush signal and interval timer.
- `buffer` → `Arc<Mutex<Vec<KnowledgeEntry>>>` with a configurable max size (default 100).
- `flush` → drains the buffer, calls `client.ingest()`, retries on transient HTTP errors (5xx, connection refused), re-queues entries on persistent failure.

### MemoryExtraction from Goal Completion

```rust
/// Called when a Goal reaches Completed state.
/// Extracts structured knowledge from the Goal and its attempts.
pub async fn extract_memories_from_goal(
    goal: &Goal,
    attempts: &[Attempt],
    mnemosyne: &dyn MnemosyneBackend,
) -> Result<Vec<KnowledgeId>> {
    let mut entries = Vec::new();

    // Architecture decisions extracted from review tasks.
    for attempt in attempts.iter().filter(|a| a.task.executor == ExecutorRef::Reviewer) {
        if let Some(decisions) = parse_architecture_decisions(&attempt.result.summary) {
            entries.extend(decisions.into_iter().map(|d| KnowledgeEntry {
                source: "goal_completion".into(),
                goal_id: Some(goal.id.clone()),
                content_type: "architecture_decision".into(),
                title: d.title,
                body: d.body,
                tags: d.tags,
                provenance: Provenance::from_attempt(goal, attempt),
                temporal: TemporalMeta::current(),
            }));
        }
    }

    // Lessons learned from failure patterns.
    for attempt in attempts.iter().filter(|a| !a.result.success) {
        if let Some(lesson) = extract_lesson(&attempt.result) {
            entries.push(KnowledgeEntry {
                source: "failure_lesson".into(),
                goal_id: Some(goal.id.clone()),
                content_type: "lesson".into(),
                title: lesson.title,
                body: lesson.body,
                tags: vec!["lesson".into(), "failure".into()],
                provenance: Provenance::from_attempt(goal, attempt),
                temporal: TemporalMeta::current(),
            });
        }
    }

    // Goal outcome summary.
    entries.push(KnowledgeEntry {
        source: "goal_completion".into(),
        goal_id: Some(goal.id.clone()),
        content_type: "goal_outcome".into(),
        title: format!("Goal completed: {}", goal.intent.objective),
        body: format!(
            "Goal {} completed in {} attempts. Outcome: {}",
            goal.id, attempts.len(), goal.outcome_summary()
        ),
        tags: vec!["goal".into(), "outcome".into()],
        provenance: Provenance::from_goal(goal),
        temporal: TemporalMeta::current(),
    });

    let ids = mnemosyne.store_batch(entries).await?;
    Ok(ids)
}
```

### Graceful Degradation Strategy

| Scenario | Behavior |
|---|---|
| GBrain unreachable at Aletheon startup | `health_check()` fails; `GBrainBackend` enters degraded mode; recall returns empty; ingest buffers locally (up to buffer_size) |
| GBrain unreachable mid-operation (recall) | Recall returns empty immediately (no blocking); warning logged |
| GBrain unreachable mid-operation (ingest) | Entries remain in pipeline buffer; retry on next flush interval |
| GBrain returns 5xx on ingest | Retry with exponential backoff (up to `ingestion_max_retries`); if exhausted, log error, entries remain buffered |
| GBrain returns 4xx on ingest | Log error, drop the malformed entry, continue with remaining batch |
| GBrain recovers after outage | Next `health_check_interval` detects healthy; flush buffered entries; resume normal recall |

### Integration Testing Strategy

| # | Test Scenario | Method |
|---|---|---|
| 1 | GBrain health check returns ok | Unit: mock HTTP server responds 200 |
| 2 | GBrain health check fails (connection refused) | Unit: mock server not started; verify degraded mode |
| 3 | Ingest single knowledge entry | Integration: real GBrain container, verify GET returns entry |
| 4 | Ingest batch of 50 entries | Integration: verify all 50 retrievable |
| 5 | Recall returns relevant results for known query | Integration: ingest known data, query, assert relevance > 0.7 |
| 6 | Expired knowledge excluded from recall | Integration: expire entry, recall, assert not in results |
| 7 | IngestionPipeline flushes on buffer full | Unit: fill buffer to max, assert automatic flush triggered |
| 8 | IngestionPipeline retries on transient 503 | Unit: mock server returns 503 twice then 201; assert all entries ingested |
| 9 | GBrainBackend recovers from degraded mode | Integration: stop GBrain, verify degraded, restart, verify recovery |

### Revised Code Layout

```text
crates/mnemosyne/src/backends/gbrain/
├── mod.rs                 -- re-exports
├── client.rs              -- GBrainClient (REST client with health tracking)
├── types.rs               -- KnowledgeEntry, RecallRequest, RecallResponse, etc.
├── backend.rs             -- GBrainBackend (MnemosyneBackend impl)
├── pipeline.rs            -- IngestionPipeline (spawn, buffer, flush, retry)
├── config.rs              -- GBrainConfig (from [gbrain] TOML section)
├── degradation.rs         -- DegradedMode state machine
├── extraction.rs          -- MemoryExtraction from Goal completion
├── testing.rs             -- Test helpers (mock server, fixtures)

docker-compose.yml         -- (project root) PostgreSQL + GBrain services
gbrain.env                 -- (project root) secrets and config for GBrain
```

### Phase H Acceptance Criteria

1. `GBrainClient::health_check()` returns true when GBrain is reachable and healthy.
2. `GBrainClient::ingest()` successfully stores a knowledge entry retrievable via `get_knowledge()`.
3. `GBrainClient::recall()` returns relevant results for a semantic query (relevance score present).
4. `GBrainClient::expire()` marks an entry as no longer current; subsequent recall excludes it.
5. `IngestionPipeline` buffers entries and flushes on a periodic interval.
6. `IngestionPipeline` triggers an immediate flush when the buffer reaches `ingestion_buffer_size`.
7. `IngestionPipeline` retries on transient HTTP errors (5xx) up to `ingestion_max_retries`.
8. `GBrainBackend` enters degraded mode (recall returns empty) when GBrain is unreachable.
9. `GBrainBackend` recovers from degraded mode when GBrain becomes healthy again.
10. `docker-compose up` starts PostgreSQL then GBrain with correct health check ordering.
11. `extract_memories_from_goal()` produces at least one knowledge entry from a completed Goal.
12. Current and obsolete architecture decisions can be distinguished (temporal validity).

## 13. First Milestone: Complete Vertical Slice

The first milestone is complete when all of the following work end-to-end:

```text
 1. User sends `/goal fix current cargo check errors` in Telegram.
 2. Aletheon creates and persists a Goal in Draft state.
 3. Native Cognit compiles intent into a GoalSpecification.
 4. GoalSupervisor::tick() transitions Goal from Draft → Planned → Running.
 5. GoalSupervisor dispatches a PlannedTask to the DeepSeekWorker.
 6. DeepSeekWorker analyzes the failure and produces a WorkerOutput.
 7. If the task involves code changes, PiWorker is dispatched with the output.
 8. PiWorker edits in an isolated worktree and returns a SubagentReport.
 9. VerificationPipeline runs all applicable gates on the diff.
10. If verification passes, an ApprovalRequest is sent to Telegram.
11. User approves (or rejects) via Telegram inline button.
12. On approval, changes are merged; Goal transitions to Completed.
13. Mnemosyne/GBrain records the Goal outcome, architecture decisions, and lessons.
14. A server restart does not lose the Goal, its state, or its attempt history.
```

## 14. Non-Goals

The following are explicitly deferred beyond the first complete vertical slice:

1. Native Android/iOS mobile app.
2. Multi-user SaaS deployment or tenant isolation.
3. Unrestricted agent autonomy (no approval-bypass mode).
4. All Google products at once (Drive, Contacts, Tasks deferred).
5. Rust rewrite of GBrain (GBrain remains a separate Python/FastAPI service).
6. Replacing Native Cognit with Pi or any subagent.
7. Distributed scheduling across multiple hosts.
8. Local large-model inference (all models accessed via API).
9. Public internet exposure (Tailscale-only remote access initially).
10. Automatic Dasein mutation (identity/value changes require human confirmation).
11. Web/PWA dashboard for Goal inspection, DAG visualization, or memory search.

## 15. Crate Impact Summary

| Crate | New Modules | Phase |
|---|---|---|
| `executive` | `channel/` (core, telegram, gmail), `goal/` (supervisor, state, store, frame), `google/` (identity, vault, gmail_cap, calendar_cap, sync), `verification/` (pipeline, gates), `approval/` (model, policy) | A, B, E, F, G |
| `cognit` | `impl/worker/` (traits, deepseek, pi, output), `impl/retry/` (policy, escalation) | C, D |
| `corpus` | `tools/subagent/` (pi_launcher, worktree, sandbox) | D |
| `mnemosyne` | `backends/gbrain/` (client, types, backend, pipeline, config, degradation, extraction, testing) | H |
| `agora` | `projection/google/` (gmail_projection, calendar_projection) | F |
| `dasein` | No new modules (Google data may propose but not commit changes) | -- |
| `kernel` | No new modules (existing EventBus, task_group sufficient) | -- |
| `fabric` | No new modules (existing IPC sufficient) | -- |
| `interact` | No new modules (existing CLI/TUI sufficient) | -- |
