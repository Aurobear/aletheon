# Aletheon Agent-Google Integration — Design (Revised)

> **Status:** Proposed (Revised 2026-07-14)
> **Based on:** `docs/arch/agent-google/` (all six architecture documents)
> **Principle:** Native Cognit remains primary; external systems are adapters, capabilities, channels, or supervised subagents.
> **Key Decision:** Extend existing infrastructure. No parallel shadow systems. No new crates.

## 1. Architecture Overview

Every new concept extends an existing component. There is no separate execution path — channels route messages through `SessionService` into `TurnPipeline::run()`, goals are `AgentProcess` variants with a `GoalSpec` attachment, and OAuth uses the existing `McpOAuthProvider` + `TokenStore`.

```
                            Google Ecosystem
              Gmail / Calendar / Drive / Contacts / Tasks
                                   |
                              OAuth 2.0
                                   |
                                   v
┌──────────────────────────────────────────────────────────────────────┐
│                         Aletheon Server                              │
│                                                                      │
│  ┌────────────────────────────────────────────┐                      │
│  │          Channel Layer                     │                      │
│  │  ┌──────────────┐  ┌────────────────────┐  │                      │
│  │  │ Telegram      │  │ GmailChannel       │  │                      │
│  │  │ (Phase A)     │  │ (Phase G)          │  │                      │
│  │  └──────┬───────┘  └─────────┬──────────┘  │                      │
│  │         │                    │              │                      │
│  │         └────────┬───────────┘              │                      │
│  │                  v                          │                      │
│  │         SessionService                      │                      │
│  │                  │                          │                      │
│  └──────────────────┼──────────────────────────┘                      │
│                     v                                                │
│  ┌─────────────────────────────────────────────────────────────────┐ │
│  │                    TurnPipeline::run()                          │ │
│  │                                                                 │ │
│  │  PreTurn ──> ReActLoop ──> PostTurn                             │ │
│  │     │             │             │                               │ │
│  │     │        SubAgentSpawner     │                               │ │
│  │     │        (agent tool)        │                               │ │
│  │     │             │              │                               │ │
│  │     │    ┌────────┴────────┐    │                               │ │
│  │     │    │  SubAgentRuntime │    │                               │ │
│  │     │    │  (trait impls)   │    │                               │ │
│  │     │    ├─────────────────┤    │                               │ │
│  │     │    │ DeepSeekRuntime  │    │                               │ │
│  │     │    │ PiRuntime        │    │                               │ │
│  │     │    └─────────────────┘    │                               │ │
│  │     │                           │                               │ │
│  │     │               ┌───────────┴──────────┐                    │ │
│  │     │               │   PostTurn hooks     │                    │ │
│  │     │               │   (verification      │                    │ │
│  │     │               │    gates here)       │                    │ │
│  │     │               └──────────────────────┘                    │ │
│  │     │                                                           │ │
│  │  SupervisorTree ─── ProcessTable ─── OperationTable             │ │
│  │  (restart policies)(lifecycle)       (cancellation)             │ │
│  └─────────────────────────────────────────────────────────────────┘ │
│                                                                      │
│  ┌────────────────────────────────────────────┐                      │
│  │         Google Integration                 │                      │
│  │  ┌──────────────────┐  ┌────────────────┐  │                      │
│  │  │ McpOAuthProvider  │  │ GoogleSyncMgr   │  │                      │
│  │  │ + TokenStore     │  │ (cursors+events) │  │                      │
│  │  │ (existing,        │  └────────────────┘  │                      │
│  │  │  configured with  │                       │                      │
│  │  │  Google endpoints)│                       │                      │
│  │  └──────────────────┘                       │                      │
│  │  ┌──────────────────┐  ┌────────────────┐  │                      │
│  │  │ Gmail/Calendar   │  │ SessionGateway  │  │                      │
│  │  │ Capabilities      │  │ .approval_flow  │  │                      │
│  │  │ (MCP tools or     │  │ (existing)       │  │                      │
│  │  │  external MCP     │  │ extended for      │  │                      │
│  │  │  servers)         │  │ send-mail ops)   │  │                      │
│  │  └──────────────────┘  └────────────────┘  │                      │
│  └────────────────────────────────────────────┘                      │
│                                                                      │
│  ┌────────────────────────────────────────────┐                      │
│  │       Existing Unchanged Core              │                      │
│  │  ┌──────────┐ ┌──────────┐ ┌──────────┐   │                      │
│  │  │ Native   │ │ Executive│ │ Agora    │   │                      │
│  │  │ Cognit   │ │ (sandbox,│ │ (scratch-│   │                      │
│  │  │          │ │  policy) │ │  pad)    │   │                      │
│  │  └──────────┘ └──────────┘ └──────────┘   │                      │
│  │  ┌──────────┐ ┌──────────────────────────┐ │                      │
│  │  │ Dasein   │ │ ModelRouter+LlmScheduler │ │                      │
│  │  │(identity │ │ (provider selection)     │ │                      │
│  │  │ values)  │ └──────────────────────────┘ │                      │
│  │  └──────────┘                              │                      │
│  └────────────────────────────────────────────┘                      │
│                                                                      │
└──────────────────────────────────────────────────────────────────────┘
                                   |
                ┌──────────────────┼──────────────────┐
                v                  v                  v
        ┌──────────────┐  ┌──────────────┐  ┌──────────────┐
        │ PostgreSQL   │  │ GBrain       │  │ Tailscale    │
        │ localhost:   │  │ localhost:   │  │ (secure mesh)│
        │ 5432         │  │ 9800         │  │              │
        └──────────────┘  └──────────────┘  └──────────────┘
```

Key design decisions reflected in the diagram:

- Channels (Telegram, Gmail) are thin adapters that route into `SessionService` then `TurnPipeline::run()`. There is no separate goal execution path.
- Goal execution goes through the same `PreTurn -> ReActLoop -> PostTurn` pipeline as every other turn.
- Sub-agents (DeepSeek, Pi) implement the existing `SubAgentRuntime` trait and are spawned by the existing `SubAgentSpawner`. This is the same path used by the `agent` tool today.
- Lifecycle management uses existing `ProcessTable`, `OperationTable`, and `SupervisorTree`.
- Budget constraints use the existing `AdmissionController::admit()`.
- Verification gates are `PostTurn` hooks registered in the existing `HookRegistry`.
- Google OAuth uses existing `McpOAuthProvider` + `TokenStore` configured with Google endpoints.
- Google API capabilities are MCP tools or external MCP server configs — no new code for capability dispatch.
- Approval uses the existing `SessionGateway::approval_flow` extended with send-mail operations.

## 2. Phase Map

| Phase | Name | Duration | Crates Touched |
|-------|------|----------|----------------|
| A | Channel Core + Telegram | Week 1-2 | executive (impl/channel/) |
| B | Goal as AgentProcess Extension | Week 2-4 | fabric (types/goal.rs), executive (service/) |
| C | DeepSeek SubAgentRuntime + Retry | Week 4-5 | cognit (impl/runtime/), executive (supervision/) |
| D | Pi Coding Subagent | Week 5-6 | corpus (tools/), executive (sub_agent/) |
| E | Verification PostTurn Hooks | Week 6-7 | executive (service/ as PostTurnHook) |
| F | Google Read-Only + Sync | Week 7-9 | corpus (tools/mcp/auth.rs config), executive (impl/google/) |
| G | Gmail Channel + Approval | Week 9-10 | executive (impl/channel/gmail.rs), executive (core/session_gateway/) |
| H | GBrain Mnemosyne Backend | Week 10-12 | mnemosyne (backends/gbrain/) |

## 3. Relationship to Existing Infrastructure

This table is the anchor for the design review checklist. Every new concept maps to an existing component — if a concept has no existing component, it must be justified as genuinely net-new.

| New Concept | Existing Component It Extends | File Path | Notes |
|---|---|---|---|
| Channel trait | SessionService entry point | `crates/executive/src/service/turn_service.rs` | Channels route messages into TurnPipeline, not a separate path |
| Goal as agent process | AgentProcess + SpawnSpec | `crates/fabric/src/types/process.rs` | GoalSpec attached as metadata; ProcessTable manages lifecycle |
| Goal state machine | ProcessState enum | `crates/fabric/src/types/process.rs:60` | Extend with AwaitingApproval, AwaitingHuman substates |
| GoalSpec type | New type in fabric | `crates/fabric/src/types/goal.rs` | Net-new data structure; no existing equivalent |
| Goal budget | AdmissionController::admit() | `crates/fabric/src/include/admission.rs` | Budget enforced at admission gate, not a separate system |
| Sub-agent execution | SubAgentRuntime trait | `crates/executive/src/core/sub_agent.rs:47` | DeepSeekRuntime and PiRuntime implement this trait |
| Sub-agent spawning | SubAgentSpawner | `crates/executive/src/core/sub_agent.rs` | Same spawner used by the `agent` tool today |
| Sub-agent lifecycle | ProcessTable + OperationTable | `crates/kernel/src/process/` | Existing kernel infrastructure |
| Retry policy | SupervisorTree restart policies | `crates/kernel/src/supervision/tree.rs` | RestartPolicy, GroupStrategy, SupervisorTree |
| Escalation (worker switch) | SupervisorTree group strategy | `crates/kernel/src/supervision/tree.rs` | RestForOne or OneForAll with different runtime |
| Verification gates | PostTurn hooks | `crates/fabric/src/types/hook.rs:21` | HookPoint::PostTurn, registered in HookRegistry |
| Hook registration | HookRegistry | `crates/executive/src/` | Existing hook system with HookType::Command/Prompt/Event |
| OAuth for Google | McpOAuthProvider + TokenStore | `crates/corpus/src/tools/mcp/auth.rs:263` | Configure with Google endpoints; tokens encrypted at rest |
| Google API capabilities | MCP tools or external MCP servers | `crates/corpus/src/tools/mcp/` | No new capability dispatch code |
| Google sync | GoogleSyncManager (new) | `crates/executive/src/impl/google/sync.rs` | Net-new: incremental cursor + event stream |
| Approval for operations | SessionGateway::approval_flow | `crates/executive/src/core/session_gateway/approval_flow.rs` | Extend with send-mail, git-push operations |
| Approval UI in channels | notification channel (notify_tx) | `crates/executive/src/service/turn_pipeline.rs:55` | Existing approval request pump in ReAct loop |
| LLM interaction | LlmProvider::complete() | `crates/fabric/src/types/llm_types.rs:61` | Not `chat()` — the method is `complete()` |
| Memory/knowledge backend | MemoryService trait | `crates/mnemosyne/src/service.rs:69` | GBrainBackend wraps GBrainClient; implements MemoryService |
| Worktree isolation | New corpus tool | `crates/corpus/src/tools/subagent/` | Net-new: git worktree temp + sandbox enforcement |
| File Sandbox | Existing sandbox infrastructure | `crates/executive/tests/sandbox_first_fail_closed.rs` | SandboxRequirement enum used in admission |
| InboundMessage types | Channel adapter types | `crates/executive/src/impl/channel/core.rs` | Thin wrapper mapping external message to internal format |

## 4. Key Changes from Original Design

1. **No parallel systems.** The original design proposed GoalSupervisor, GoalWorker, CredentialVault, and ApprovalManager as standalone components duplicating existing infrastructure. This revision extends existing `SubAgentSpawner`, `McpOAuthProvider`, `SessionGateway::approval_flow`, and `SupervisorTree` instead.

2. **No new crates.** All new modules live inside existing crates (`fabric`, `executive`, `cognit`, `corpus`, `mnemosyne`).

3. **Goal = AgentProcess variant.** Goals are not a separate runtime. They are `AgentProcess` records with a `GoalSpec` attachment. Goal execution goes through `TurnPipeline::run()`, the same path as every other turn.

4. **Goals use existing lifecycle.** The `ProcessTable` tracks goal lifecycle states. The `OperationTable` provides per-task cancellation. The `SupervisorTree` provides restart and escalation policies. No new `GoalStore` or `GoalFrame` persistence layer — process state is the authoritative record.

5. **Sub-agents implement SubAgentRuntime.** DeepSeek and Pi are not `GoalWorker` trait implementations. They implement the existing `SubAgentRuntime` trait and are spawned by the existing `SubAgentSpawner`.

6. **Retry = SupervisorTree policies.** No standalone `RetryPolicy`. The kernel `RestartPolicy` with `max_restarts` is the retry mechanism. Escalation (switching from DeepSeek to GPT/Opus) is a supervisor group strategy with a different runtime.

7. **Verification = PostTurn hooks.** No standalone `VerificationPipeline`. Verification gates are `PostTurn` hooks registered in the existing `HookRegistry`. They fire after the ReAct loop completes, same as all other post-turn processing.

8. **OAuth = McpOAuthProvider configured for Google.** No `CredentialVault` or AES-256-GCM key management. The existing `McpOAuthProvider` supports the OAuth 2.0 authorization code flow and the `TokenStore` provides AES-256-GCM encrypted persistence. Google is simply another OAuth provider configured with Google's endpoints.

9. **Approval = SessionGateway extended.** No `ApprovalManager`. The existing `SessionGateway::approval_flow` handles tool-level approval. It is extended with additional approval categories (send-mail, git-push) that route through the same notification channel used today.

10. **Google APIs = MCP tools.** Gmail, Calendar, and Drive capabilities are exposed as MCP tools (internal) or external MCP servers. No capability-specific dispatch code — the existing MCP transport handles execution.

11. **Phase 0 removed.** The existing Native Cognit entrypoint and DeepSeek path are already stable as of 2026-07-14. New modules extend without rewriting existing paths.

12. **Deployment is continuous, not a separate phase.** Systemd, Docker Compose, backups, and Tailscale are addressed incrementally during Phase H and beyond.

13. **Web dashboard deferred indefinitely.** The first deployment relies entirely on Telegram, CLI/TUI, and Gmail for interaction.

## 5. Security Invariants

These invariants were correct in the original design and carry forward unchanged.

1. Tokens are encrypted at rest (via `TokenStore` AES-256-GCM, key from secrets file outside the repo).
2. Refresh tokens never enter model context (not in GoalSpec, not in any prompt).
3. Tokens never enter GBrain (not stored in the knowledge backend).
4. Logs redact OAuth tokens and sensitive payloads (redaction applied before write).
5. Read and write permissions are distinct scopes (granted incrementally).
6. Destructive operations require approval (send mail, modify calendar, delete drive files).
7. Email sender validation is mandatory before any action (SPF/DKIM-validated or allow-listed).
8. Pi runs in an isolated temporary worktree, never in the main working copy.
9. Pi has no network access by default (enforced via sandbox/namespace).
10. Pi cannot modify Dasein, Executive policy, or any `config/` directory.
11. Pi's stdout and stderr are fully captured and attached to the attempt record.
12. A hard timeout (default: 5 minutes per task) is enforced with SIGKILL.
13. All file changes are collected as a diff that must pass verification before merging.

## 6. Phase A: Channel Core + Telegram

Channels are thin adapters. Their sole responsibility is to receive messages from external systems and route them into the existing `SessionService -> TurnPipeline` path. They are not a separate execution path.

### Code Layout

```
crates/executive/src/impl/channel/
  mod.rs              -- re-exports
  core.rs             -- Channel trait, InboundMessage, OutboundMessage, MessageContent, UserAction
  telegram.rs         -- TelegramChannel: long-polling, command dispatch, offset persistence
  gmail.rs            -- stub; implemented in Phase G
```

### Core Types

The `Channel` trait is the single interface every message channel must implement.

- `channel_id()` returns a unique channel identifier (e.g. "telegram", "gmail").
- `start()` begins receiving messages and returns a stream of `InboundMessage`.
- `send()` sends an outbound message (text, buttons, attachments).
- `ack()` acknowledges receipt of a message so the channel can advance its cursor.

`InboundMessage` uses canonical field names matching the internal protocol:

- `channel_id`: which channel this message arrived on
- `message_id`: unique message identifier within the channel
- `conversation_id`: groups messages into conversations
- `sender_id`: the sender's identity (PrincipalId)
- `content`: the message body (MessageContent enum)
- `timestamp`: when the message was received
- `reply_to_action`: optional reference to an action this message responds to

`OutboundMessage` carries:

- `conversation`: target conversation
- `content`: message body
- `actions`: interactive buttons (approve, reject, callback, url)
- `reply_to`: optional reference to a message being replied to

`MessageContent` variants: Text, Markdown, Voice, Image, File.

`UserAction` carries an action id, label, and type. `ActionType` variants: Callback, Url, Approve, Reject.

### Telegram Commands

```
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
- Messages received by any channel are routed into `SessionService` which creates a turn and passes it to `TurnPipeline::run()`. There is no separate "goal execution path" — goals are turns too.

### Phase A Acceptance Criteria

1. Owner can send `/start` and receive a greeting in Telegram.
2. Owner can send `/chat <msg>` and receive a response from Native Cognit.
3. Telegram long polling survives a server restart (no message loss or duplication).
4. Unknown Telegram users are rejected with an audit log entry.
5. Offset persistence survives process restart.

## 7. Phase B: Goal as AgentProcess Extension

A Goal is an `AgentProcess` variant with a `GoalSpec` attachment. Instead of building a new `GoalSupervisor` trait, 10-state FSM, `GoalStore`, and `GoalFrame`, goals extend the existing process infrastructure.

### GoalSpec (new type in fabric)

`GoalSpec` is a new data structure at `crates/fabric/src/types/goal.rs`. It carries:

- `original_intent`: the user's goal text, immutable after creation
- `desired_state`: what the goal should produce
- `constraints`: boundaries on what the goal may do
- `acceptance_criteria`: objective conditions for completion
- `budget`: token and cost caps, wall-clock deadline
- `approval_policy`: which operations require human approval
- `escalation_policy`: when to request intervention or switch models
- `capability_boundary`: what the goal is allowed to invoke
- `workspace_boundary`: filesystem scope (typically a temp worktree)

`GoalSpec` is not a trait. It is serialized data attached to the `AgentProcess` record.

### ProcessState Extension

The existing `ProcessState` enum (`crates/fabric/src/types/process.rs:60`) gains two new substates for goal-specific behavior:

- `AwaitingApproval` — the goal has produced a result that needs human confirmation.
- `AwaitingHuman` — the goal cannot proceed without human input.

These are extensions to the existing state machine, not a separate goal state machine. The standard transitions (Running, Waiting, Stopping, Exited, Failed) still apply.

### Goal Lifecycle via Existing Infrastructure

- **Creation:** A `/goal` command in a channel creates a new `AgentProcess` with `GoalSpec` attached. The process is registered in the `ProcessTable`.
- **Execution:** The goal executes as standard turns through `TurnPipeline::run()`. The `GoalSpec` is injected into the system prompt or as a `GoalSet` event.
- **Tick:** There is no explicit `tick()` method. Tick semantics emerge from the natural turn cycle — each turn advances the goal by one unit of work.
- **Budget:** Each turn passes through `AdmissionController::admit()`, which enforces budget constraints (token caps, cost caps, lease timeouts). No separate budget system.
- **Pause/Resume:** The `ProcessTable` transitions the process to `Waiting` (paused) or back to `Running` (resumed). These map to the existing process lifecycle.
- **Cancellation:** The `OperationTable` cancels the goal's operation scope, which cooperatively stops all sub-tasks via cancellation tokens.
- **Completion/Failure:** The process transitions to `Exited` (completed) or `Failed` (exhausted attempts). These are standard process states.
- **Audit:** Every state transition in the `ProcessTable` is logged. Every turn produces a `TurnMetrics` record. No separate audit log.

### Safety Requirements

Every Goal must carry (via `GoalSpec`):

- Budget limit (token and cost caps).
- Time limit (wall-clock deadline).
- Attempt limit (maximum retries per task before escalation).
- Capability boundary (what the Goal is allowed to invoke).
- Workspace boundary (filesystem scope, typically a temp worktree).
- Pause and cancellation hooks (must be responsive within 5 seconds).
- Approval policy (which operations require human approval).
- Completion criteria (objective evidence conditions).
- Escalation policy (when to request human intervention or change models).

All of these are enforced by existing infrastructure: `AdmissionController` for budget and capabilities, `OperationTable` for cancellation, `SupervisorTree` for retry limits and escalation.

### Phase B Acceptance Criteria

1. A Goal created via `/goal` is a persistent `AgentProcess` that survives restart.
2. Goal execution uses `TurnPipeline::run()`, the same path as chat turns.
3. The original intent text is immutable after creation (stored in `GoalSpec`).
4. Pause/resume/cancel transitions use `ProcessTable` state changes and are recorded in process audit.
5. A Goal that exhausts its attempt limit transitions to `Failed` (standard process state).
6. Goal state is queryable through Telegram `/status` and `/goals`.
7. Budget is enforced by `AdmissionController::admit()` on every turn.

## 8. Phase C: DeepSeek SubAgentRuntime + Retry

The DeepSeek worker is not a `GoalWorker` trait implementation. It implements the existing `SubAgentRuntime` trait and is spawned by the existing `SubAgentSpawner`.

### SubAgentRuntime Trait (existing)

The trait at `crates/executive/src/core/sub_agent.rs:47` is already defined:

```
pub trait SubAgentRuntime: Send + Sync {
    async fn run(&self, task: &str, cancel: CancellationToken) -> Result<String, String>;
}
```

`DeepSeekRuntime` implements this trait. The `task` parameter carries the task description (derived from `GoalSpec`). The `CancellationToken` provides cooperative cancellation.

When the LLM invokes the `agent` tool, `SubAgentSpawner` spawns a task through the kernel `ProcessTable` and `OperationTable`. If a `SubAgentRuntime` is configured, the spawned task runs real LLM + tool work. Otherwise, it waits for cancellation (test/dev mode).

### DeepSeekRuntime Implementation

`DeepSeekRuntime` holds a reference to the LLM provider (configured for DeepSeek) and tool definitions. Its `run()` method:

1. Reads the task description and any evidence from previous attempts.
2. Builds a prompt from the task + evidence.
3. Calls `provider.complete()` (not `chat()` — the method on `LlmProvider` at `crates/fabric/src/types/llm_types.rs:61` is `complete()`).
4. Returns the worker output as a `Result<String, String>`.

### Retry via SupervisorTree

No standalone `RetryPolicy`. Retry uses the existing kernel `SupervisorTree` (`crates/kernel/src/supervision/tree.rs`):

- `RestartPolicy::RestartOnFailure { max_restarts: 3 }` — the sub-agent restarts up to 3 times on failure.
- `GroupStrategy` — when escalation is needed (switch from DeepSeek to GPT/Opus), the supervisor restarts all members of a group with a different runtime configuration.

The default retry sequence:

1. Attempt 1 fails -> same worker receives failure evidence as additional context (restart with enriched task).
2. Same failure class on Attempt 2 -> strategy switch (parameter change or task shrink).
3. Same failure class on Attempt 3 -> supervisor escalates to GPT/Opus runtime (group restart with different provider).
4. Still unresolved after escalation -> process transitions to `AwaitingHuman` or `Failed`.

### Failure Classification

Failure classification remains a helper for deciding restart strategy, not a standalone system:

| FailureClass | Strategy | Next Executor |
|---|---|---|
| Compilation | Restart with compiler error text as evidence | Same runtime (DeepSeek) |
| TestFailure | Restart with test output as evidence | Same runtime (DeepSeek) |
| Timeout | Shrink task scope or increase timeout | Same or GPT/Opus |
| MissingDependency | Add dependency to task context, restart | Same runtime |
| InvalidAssumption | Query memory or ask user, restart with correction | GPT/Opus |
| ArchitectureViolation | Replan or review | GPT/Opus |
| RepeatedFailure | Shrink task, change executor, or escalate | GPT/Opus or AwaitingHuman |

### Phase C Acceptance Criteria

1. DeepSeekRuntime implements `SubAgentRuntime` and is spawnable by `SubAgentSpawner`.
2. Worker output is captured as a process attempt record with token usage and duration.
3. Compilation failures are classified correctly and restarted with compiler evidence.
4. Repeated failures of the same class escalate to GPT/Opus after 3 restarts (SupervisorTree policy).
5. Missing-context failures trigger a MemoryService query or user prompt.
6. PermissionDenied is logged as a policy event and does not retry blindly.
7. Token usage and cost are accounted per attempt via `AdmissionController::settle()`.

## 9. Phase D: Pi Coding Subagent

Pi is a `SubAgentRuntime` implementation, not a `GoalWorker`. It is spawned by `SubAgentSpawner` like any other sub-agent.

### PiRuntime Implements SubAgentRuntime

`PiRuntime::run()` spawns a coding agent in an isolated temporary worktree. It receives a task description, executes code changes, runs verification, and returns a structured result as a `String` (JSON-serialized `PiSubagentReport`).

### PiSubagentReport

The report carries:

- `task_id`: which task this report covers
- `success`: whether the coding attempt succeeded
- `exit_code`: the subprocess exit code
- `stdout_summary`: truncated stdout output
- `stderr_summary`: truncated stderr output
- `files_changed`: list of `ChangedFile` records (path, change_type, lines_added, lines_removed)
- `diff_patch`: optional unified diff of all changes
- `warnings`: non-fatal issues encountered
- `token_usage`: tokens consumed by the coding agent
- `duration`: wall-clock duration

### Worktree Isolation (net-new)

Worktree isolation is genuinely net-new infrastructure — no existing component provides temporary git worktrees with filesystem sandboxing:

1. Pi spawns in an isolated temporary worktree, never in the main working copy.
2. Pi has no network access by default (enforced via sandbox/namespace).
3. Pi cannot modify Dasein, Executive policy, or any `config/` directory.
4. Pi's stdout and stderr are fully captured and attached to the attempt record.
5. A hard timeout (default: 5 minutes per task) is enforced with SIGKILL.
6. All file changes are collected as a diff that must pass verification before merging.

### Supervision

Pi failures are handled by the `SupervisorTree`, same as any other sub-agent. If Pi exhausts its restart limit, the failure propagates to the goal process, which either escalates or transitions to `AwaitingHuman`.

### Phase D Acceptance Criteria

1. PiRuntime implements `SubAgentRuntime` and is spawnable by `SubAgentSpawner`.
2. Pi spawns in an isolated worktree (verify via `git worktree list`).
3. Pi cannot access the main worktree filesystem (enforced by sandbox).
4. Timeout kills the Pi process and the attempt is recorded as Failed (Timeout).
5. Stdout, stderr, and exit status are captured in the `PiSubagentReport`.
6. Diff is collected and does not include files outside the allowed scope.
7. Native Cognit reviews the `PiSubagentReport` before any merge decision.

## 10. Phase E: Verification as PostTurn Hooks

Verification gates are not a standalone `VerificationPipeline` called only from the Goal path. They are `PostTurn` hooks registered in the existing hook system, invoked through `TurnPipeline::run()` after the ReAct loop completes.

### Hook System (existing)

The hook system is defined at `crates/fabric/src/types/hook.rs`. The `HookPoint::PostTurn` variant fires after LLM response is generated. Hooks are registered via `HookRegistry` and can be of type `Command`, `Prompt`, or `Event`.

For verification, each gate is a `HookType::Command` that spawns a verification script. The script receives the diff and task context via stdin and returns a structured result.

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

### Gate Outcomes

Each gate hook returns a `CommandHookResult` (`crates/fabric/src/types/hook_ext.rs:40`) with:

- `block: true` with `block_reason` -> equivalent to `FailBlock` (stop pipeline immediately)
- `modify: true` with `data` -> evidence for retry (equivalent to `FailRetry`)
- `block: false, modify: false` -> gate passed (`Pass`)
- Non-blocking issues -> `inject_message` with warning (`FailAutoFixable`)

### Integration with TurnPipeline

Verification hooks fire during the existing post-turn phase in `TurnPipeline::run()`. The hooks run after the ReAct loop completes and before the turn is committed to session history. If a hook blocks (`Block` result), the turn is aborted and either retried (with evidence) or escalated.

### Phase E Acceptance Criteria

1. All 7 gates run in order for every code-change task (registered as PostTurn hooks).
2. Format failures are non-blocking (recorded but not blocking the turn).
3. Compile failures return the full error text as retry evidence.
4. Test failures provide per-test failure output.
5. DiffScope violations block the task immediately.
6. Hooks run through the standard `HookRegistry` execution path, not a separate pipeline.

## 11. Phase F: Google Read-Only + Sync

Google OAuth uses the existing `McpOAuthProvider` + `TokenStore` infrastructure. There is no `CredentialVault` with separate AES-256-GCM key management — `TokenStore` already provides encrypted persistence.

### OAuth Configuration

`McpOAuthProvider` (`crates/corpus/src/tools/mcp/auth.rs:263`) is configured with Google's OAuth endpoints:

- `auth_url`: `https://accounts.google.com/o/oauth2/v2/auth`
- `token_url`: `https://oauth2.googleapis.com/token`
- `redirect_uri`: localhost callback (e.g. `http://localhost:9801/oauth/callback`)

The `TokenStore` persists tokens encrypted at rest (AES-256-GCM, key from secrets file). Existing infrastructure — no new vault code.

### Google API Capabilities

Google APIs (Gmail, Calendar, Drive) are exposed as MCP tools or external MCP server configurations. No new capability dispatch code:

- **Internal MCP tools:** Gmail search/read, Calendar list, Drive file access are implemented as MCP tool definitions in `crates/corpus/src/tools/`. The existing MCP transport handles execution.
- **External MCP servers:** Alternatively, Google APIs can be provided by external MCP servers. The existing `McpTransport` connects to them with `McpOAuthProvider` headers.

### GoogleSyncManager (net-new)

`GoogleSyncManager` is genuinely net-new infrastructure — there is no existing component for incremental API polling with cursors. It lives at `crates/executive/src/impl/google/sync.rs`.

The incremental sync strategy:

```
Sync Cursor  (per account, per service: Gmail historyId, Calendar syncToken, Drive changeToken)
     |
     | poll (interval: configurable, default 60s)
     v
Fetch Changes  (incremental since last cursor)
     |
     v
Deduplicate    (messageId / eventId already seen -> skip)
     |
     v
Normalize      (Google-specific -> GoogleEvent enum)
     |
     v
Dispatch       (event -> notification, Goal wakeup, memory proposal, Agora projection)
     |
     v
Advance Cursor (persist new cursor atomically after dispatch)
```

### Normalized Events to Actions

| GoogleEvent Variant | Aletheon Action |
|---|---|
| MailReceived(from=known_sender) | Classify subject tag ([GOAL]/[ASK]/etc.), create Goal draft or notification |
| MailReceived(from=unknown) | Log, optionally notify user; no automatic execution |
| CalendarEventCreated | Project into Agora, check for deadline implications |
| CalendarEventUpdated | Update Agora projection, wake Goals waiting on this event |
| CalendarEventDeleted | Remove Agora projection |
| DriveFileCreated | Policy check -> ingest artifact -> optionally propose to memory |
| DriveFileUpdated | Same as created (update existing artifact) |
| DriveFileDeleted | Remove local artifact reference |

### MVP Scope

**Sub-phase F-1 (Read-Only Manual):**
- Google OAuth authorization code flow via `McpOAuthProvider`.
- Encrypted credential storage via `TokenStore`.
- Gmail read-only MCP tools (search, read message, list inbox).
- Calendar read-only MCP tools (list events in time range).
- Account binding to principal.
- Manual refresh (user-triggered, no background sync yet).
- Telegram queries: "what's on my calendar today?", "show unread important mail".

**Sub-phase F-2 (Incremental Sync):**
- Gmail history cursor (watch/poll for new messages).
- Calendar sync token (poll for event changes).
- Normalized event stream via `GoogleSyncManager`.
- Deduplication based on provider IDs.
- Automatic Agora projection updates.
- Telegram notifications for new important mail.
- Goal wakeup (a Goal can register interest in a Calendar event or email thread).

### Phase F Acceptance Criteria

1. OAuth flow completes and returns a valid access token without exposing the refresh token to logs.
2. Encrypted credentials survive process restart and are not readable as plaintext on disk.
3. Gmail MCP tool `search_messages` returns summaries for a query like "from:boss@company.com".
4. Calendar MCP tool `list_events` returns today's events.
5. Manual refresh does not duplicate previously seen messages or events.
6. (F-2) Incremental sync detects new Gmail messages within 2 poll intervals.
7. (F-2) Calendar sync token survives restart and resumes without full re-scan.
8. (F-2) A Goal registered to watch for a calendar event is woken when the event is created.

## 12. Phase G: Gmail Channel + Approval

The Gmail channel is net-new channel infrastructure (no existing email channel). However, approval for operations uses the existing `SessionGateway::approval_flow` extended with new operation types, not a standalone `ApprovalManager`.

### GmailChannel

`GmailChannel` implements the `Channel` trait. It receives emails via `GoogleSyncManager` and converts them to `InboundMessage` records with canonical field names:

- `channel_id`: "gmail"
- `message_id`: the email's Message-ID header
- `conversation_id`: derived from thread ID or In-Reply-To chain
- `sender_id`: email address mapped to PrincipalId
- `content`: email body as `MessageContent::Markdown`
- `timestamp`: email Date header
- `reply_to_action`: optional action reference if this email is a reply

Messages are routed through `SessionService` into `TurnPipeline::run()`, just like Telegram messages.

### Email Classification

| Subject Tag | Action | Default Behavior |
|---|---|---|
| `[GOAL] <objective>` | Create Goal draft | Enters as AgentProcess with GoalSpec; Telegram notification with approve/reject |
| `[ASK] <question>` | Create inquiry | Native Cognit responds; answer drafted as reply (approval required to send) |
| `[MEMORY] <text>` | Propose memory | MemoryService ingestion proposed; user confirms via Telegram |
| `[DOC] <text>` | Document import | Artifact stored; optionally indexed in GBrain |
| (no tag) | Classify via LLM | Low-confidence -> notify user; High-confidence (known sender) -> draft Goal or reply |

### Approval: Extending SessionGateway

The existing `SessionGateway::approval_flow` (`crates/executive/src/core/session_gateway/approval_flow.rs`) handles tool-level approval requests. It is extended with:

- New approval categories: SendMail, DeleteFile, ModifyCalendar, GitPush, CapabilityExpansion, DaseinModification.
- Risk-level classification (Low, Medium, High, Critical).
- Expiration timeout (auto-reject after timeout).
- Notification routing through the existing `notify_tx` channel (used for TUI events today).

### Operations Requiring Approval

| Operation | Risk | Condition |
|---|---|---|
| SendMail | High | Always require approval |
| DeleteFile | High | Always require approval |
| ModifyCalendar | Medium | Approval unless from a pre-authorized Goal |
| GitPush | High | Always require approval |
| CapabilityExpansion | High | Always require approval |
| DaseinModification | Critical | Always require approval + confirmation prompt |

Approval requests present approve/reject buttons in Telegram (via the existing notification channel). The approval pump in `TurnPipeline::run()` already handles pending approval notifications — this extends that mechanism with the new operation types.

### Phase G Acceptance Criteria

1. `[GOAL]` email creates a Goal draft that appears in Telegram for approval.
2. `[ASK]` email triggers a Native Cognit response; the draft reply is shown before sending.
3. Unknown senders create a notification but no automatic Goal or response.
4. Approval requests present approve/reject buttons in Telegram via the existing notification channel.
5. Expired approval requests auto-reject and log the event.
6. SendMail requires explicit human approval (no autonomous sending).
7. Email message ID deduplication prevents duplicate Goal creation.
8. `InboundMessage` uses canonical field names: `channel_id`, `message_id`, `conversation_id`, `sender_id`, `content`, `timestamp`, `reply_to_action`.

## 13. Phase H: GBrain Mnemosyne Backend + Service Integration

Phase H is structurally correct from the original design. `GBrainBackend` implements the `MemoryService` trait (not a separate `MnemosyneBackend` trait), and wraps the `GBrainClient` REST client.

### GBrainClient

The `GBrainClient` is a REST client for the GBrain HTTP API. It holds:

- `base_url`: GBrain API base URL (e.g. `http://localhost:9800/api/v1`)
- `api_key`: API key for authentication (loaded from env or secrets file)
- `client`: `reqwest::Client` with connection pooling
- `health`: atomic flag reflecting last health check status

Methods: `health_check()`, `ingest()`, `recall()`, `search()`, `expire()`, `get_knowledge()`, `is_healthy()`.

### GBrainBackend Implements MemoryService

`GBrainBackend` wraps `GBrainClient` and an `IngestionPipeline`. It implements the `MemoryService` trait (`crates/mnemosyne/src/service.rs:69`).

The `record()` method converts `ExperienceEvent` to `KnowledgeEntry` and queues it in the ingestion pipeline. The `recall()` method queries GBrain and returns results. If GBrain is unhealthy, recall returns empty (graceful degradation).

### IngestionPipeline

The `IngestionPipeline` uses a background Tokio task with `tokio::select!` on flush signal and interval timer. It buffers entries in `Arc<Mutex<Vec<KnowledgeEntry>>>` with a configurable max size (default 100). Flush drains the buffer, calls `client.ingest()`, retries on transient HTTP errors (5xx, connection refused), and re-queues entries on persistent failure.

### Graceful Degradation Strategy

| Scenario | Behavior |
|---|---|
| GBrain unreachable at Aletheon startup | `health_check()` fails; backend enters degraded mode; recall returns empty; ingest buffers locally |
| GBrain unreachable mid-operation (recall) | Recall returns empty immediately (no blocking); warning logged |
| GBrain unreachable mid-operation (ingest) | Entries remain in pipeline buffer; retry on next flush interval |
| GBrain returns 5xx on ingest | Retry with exponential backoff (up to `ingestion_max_retries`); if exhausted, log error, entries remain buffered |
| GBrain returns 4xx on ingest | Log error, drop the malformed entry, continue with remaining batch |
| GBrain recovers after outage | Next `health_check_interval` detects healthy; flush buffered entries; resume normal recall |

### Docker Compose Service Definitions

PostgreSQL and GBrain services defined in `docker-compose.yml` at project root. PostgreSQL on `127.0.0.1:5432`, GBrain on `127.0.0.1:9800`. GBrain depends on PostgreSQL with `condition: service_healthy`. Environment file `gbrain.env` provides secrets.

### Startup Ordering

```
1. PostgreSQL starts (systemd or docker compose).
   Health check: pg_isready returns 0. GBrain waits for this condition.

2. GBrain starts.
   Runs database migrations.
   Health check: GET /health returns 200. Aletheon waits for this condition.

3. Aletheon starts.
   GBrainBackend reads [gbrain] config.
   On startup, health_check() verifies backend reachable.
   If health check fails -> GBrainBackend enters degraded mode.
   If health check later succeeds -> GBrainBackend recovers automatically.
```

### Configuration [gbrain] TOML Section

```
[gbrain]
base_url = "http://localhost:9800/api/v1"
api_key_env = "GBRAIN_API_KEY"
ingestion_buffer_size = 100
ingestion_flush_interval_secs = 30
ingestion_max_retries = 3
ingestion_retry_backoff_ms = 1000
recall_default_limit = 10
recall_max_limit = 50
health_check_interval_secs = 30
health_check_timeout_secs = 5
enabled = true
```

### Memory Extraction from Goal Completion

When a Goal reaches completed state, structured knowledge is extracted:

- Architecture decisions from review tasks.
- Lessons learned from failure patterns.
- Goal outcome summary.

Each entry includes provenance (goal_id, task_id, principal, timestamp) and temporal metadata (valid_from, valid_until, is_current). This is called as a PostTurn hook or as part of goal completion processing.

### Code Layout

```
crates/mnemosyne/src/backends/gbrain/
  mod.rs                 -- re-exports
  client.rs              -- GBrainClient (REST client with health tracking)
  types.rs               -- KnowledgeEntry, RecallRequest, RecallResponse, etc.
  backend.rs             -- GBrainBackend (MemoryService impl)
  pipeline.rs            -- IngestionPipeline (spawn, buffer, flush, retry)
  config.rs              -- GBrainConfig (from [gbrain] TOML section)
  degradation.rs         -- DegradedMode state machine
  extraction.rs          -- Memory extraction from Goal completion

docker-compose.yml       -- (project root) PostgreSQL + GBrain services
gbrain.env               -- (project root) secrets and config for GBrain
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
11. Memory extraction from a completed goal produces at least one knowledge entry.
12. Current and obsolete architecture decisions can be distinguished (temporal validity).

## 14. First Milestone: Complete Vertical Slice

The first milestone is complete when all of the following work end-to-end:

```
 1. User sends `/goal fix current cargo check errors` in Telegram.
 2. Aletheon creates a Goal as an AgentProcess with GoalSpec attached.
 3. Native Cognit compiles intent into a GoalSpecification.
 4. The goal executes as turns through TurnPipeline::run().
 5. SubAgentSpawner dispatches a coding task to DeepSeekRuntime.
 6. DeepSeekRuntime analyzes the failure and returns output.
 7. If the task involves code changes, PiRuntime is dispatched as a sub-agent.
 8. PiRuntime edits in an isolated worktree and returns a PiSubagentReport.
 9. PostTurn verification hooks run all applicable gates on the diff.
10. If verification passes, an approval request is sent to Telegram.
11. User approves (or rejects) via Telegram inline button.
12. On approval, changes are merged; Goal process transitions to Exited.
13. MemoryService records the Goal outcome, architecture decisions, and lessons.
14. A server restart does not lose the Goal, its state, or its attempt history.
```

## 15. Non-Goals

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

## 16. Crate Impact Summary

| Crate | New Modules | Phase |
|---|---|---|
| `fabric` | `types/goal.rs` (GoalSpec, goal types) | B |
| `executive` | `impl/channel/` (core, telegram, gmail), `impl/google/sync.rs` (GoogleSyncManager), verification hooks (PostTurn hook definitions), extended approval_flow | A, E, F, G |
| `cognit` | `impl/runtime/` (DeepSeekRuntime, PiRuntime as SubAgentRuntime impls) | C, D |
| `corpus` | `tools/subagent/` (worktree, sandbox), MCP tool definitions for Gmail/Calendar | D, F |
| `mnemosyne` | `backends/gbrain/` (client, types, backend, pipeline, config, degradation, extraction) | H |
| `agora` | Projection for Google events (gmail_projection, calendar_projection) | F |
| `dasein` | No new modules (Google data may propose but not commit changes) | -- |
| `kernel` | No new modules (existing EventBus, supervision, task_group sufficient) | -- |
| `interact` | No new modules (existing CLI/TUI sufficient) | -- |
