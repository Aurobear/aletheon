# Aletheon Personal Agent Integration — Code-Aligned Design

> **Status:** Proposed rewrite, 2026-07-14
> **Requirement sources:** `docs/arch/agent-google/00_README.md` through `06_ALETHEON_NAMING_AND_SYSTEM_IDENTITY.md`
> **Design rule:** Preserve the product intent, but do not claim capabilities that the current code does not provide.

## 1. Scope and requirement anchors

This design retains the following source requirements:

| Requirement | Source |
|---|---|
| Native Cognit remains primary; integrations are channels, capabilities, adapters, or supervised subagents | `docs/arch/agent-google/00_README.md:3-5` |
| First useful loop is Telegram → persistent Goal → worker/Pi → verification → approval → durable outcome | `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:190-206` |
| A Goal is persistent, preserves original intent, and supports pause/resume/cancel | `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:6-44`, `:343-369` |
| Goal execution is bounded; there is no unbounded model loop | `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:106-125` |
| Attempts retain executor, evidence, timing, and result | `docs/arch/agent-google/04_GOAL_RUNTIME_ARCHITECTURE.md:166-181` |
| Pi edits an isolated worktree and does not modify the main worktree directly | `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:94-105` |
| Google starts read-only with encrypted credentials and account binding | `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:128-138` |
| Google synchronization resumes after restart without duplicate events | `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:140-151` |
| Gmail `[GOAL]` creates a draft rather than unrestricted execution | `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:153-162` |
| GBrain is a Mnemosyne backend; it must not mutate Dasein directly | `docs/arch/agent-google/05_IMPLEMENTATION_ROADMAP.md:118-126` |

## 2. Verified code baseline

The implementation must start from these current facts:

| Current capability | Code anchor | Consequence |
|---|---|---|
| Daemon turns use `TurnPipeline`; the separate `TurnService` exposes `submit()` | `crates/executive/src/service/turn_pipeline.rs:40-67`, `crates/executive/src/service/turn_service.rs:14-55` | Channel adapters target a new daemon channel router backed by `TurnPipeline`; there is no `SessionService` |
| Kernel processes have no arbitrary metadata or Goal attachment | `crates/fabric/src/types/process.rs:108-131` | `GoalSpec` cannot be attached to `ProcessRecord` without changing the kernel ABI |
| Executive already has a SQLite `ObjectiveStore` with startup recovery | `crates/executive/src/impl/goal/mod.rs:12-49`, `crates/executive/src/impl/goal/store.rs:74-111`, `crates/executive/src/impl/daemon/handler/init.rs:249-278` | Evolve this store compatibly instead of creating a parallel Goal database |
| `ProcessState` is a generic lifecycle state machine | `crates/fabric/src/types/process.rs:60-90` | Goal business states stay separate from process states |
| `SubAgentSpawner` has one optional global runtime | `crates/executive/src/core/sub_agent.rs:120-130` | Per-agent runtime selection and escalation require a scoped extension |
| `SupervisorTree` only decides restart count/group membership | `crates/kernel/src/supervision/tree.rs:8-51` | Backoff, evidence enrichment, failure classification, and provider switching are not supervisor features |
| `AdmissionController` governs side-effecting capability permits | `crates/fabric/src/include/admission.rs:13-45` | Goal-wide token/cost/attempt accounting needs a Goal budget ledger |
| `TokenStore` writes JSON directly to disk | `crates/corpus/src/tools/mcp/auth.rs:141-205` | Current OAuth persistence is plaintext and is not acceptable for Google refresh tokens |
| Tool approval is pumped through `SocketApprovalGate` and `notify_tx` | `crates/corpus/src/security/socket_approval.rs:15-54`, `crates/executive/src/service/turn_pipeline.rs:604-633` | Approval delivery may be extended, but pending approvals need durable correlation for long-lived Goals |
| `PostTurnPipeline` is context-free, while daemon PostTurn hooks execute elsewhere | `crates/executive/src/service/post_turn.rs:5-14`, `crates/executive/src/service/daemon_turn/post_phases.rs:189-210` | Long-running code verification is a job stage, not a context-free `PostTurnPipeline` hook |
| Mnemosyne exposes `MemoryService` | `crates/mnemosyne/src/service.rs:67-74` | GBrain implements or composes this service under `src/backends/gbrain/` |

## 3. Decisions replacing the previous draft

| Previous assumption | Decision in this rewrite |
|---|---|
| Goal is an ephemeral `AgentProcess` with an attached `GoalSpec` | Goal is a durable domain object linked to a transient `ProcessId` while running |
| Goal-specific states extend `ProcessState` | Keep `ProcessState` generic; add `GoalState` and `GoalWaitReason` |
| `SupervisorTree` performs retries and model escalation | `SupervisorTree` remains lifecycle-only; `AttemptCoordinator` owns retry/backoff/escalation |
| A single `SubAgentRuntime` can route DeepSeek and Pi | Add a per-spawn `RuntimeId` and runtime registry while preserving the existing default runtime path |
| `AdmissionController` alone enforces all Goal budgets | Use admission for capabilities and a Goal ledger for token, cost, attempt, and wall-clock budgets |
| `TokenStore` is already encrypted | Add an encrypted token-store boundary before saving Google refresh tokens |
| All verification is a generic PostTurn hook | Run deterministic verification as a cancellable job stage; PostTurn publishes the result |
| An in-memory channel plus cursor implies reliable delivery | Persist inbox/dedup/cursor state in SQLite and acknowledge only after durable turn outcome |
| Google background sync can ambiguously mix MCP and REST | MCP is for agent-invoked tools; a dedicated Google client performs cursor-based background sync |
| GBrain helpers alone constitute a backend | Implement the existing `MemoryService` contract and a durable local ingestion spool |

## 4. Target architecture

```text
 Telegram / Gmail
        |
        v
 Channel adapters
        |
        v
 DurableChannelInbox ---- ChannelBindingRepository
        |                         |
        +-----------+-------------+
                    v
              ChannelRouter
                    |
          +---------+----------+
          |                    |
          v                    v
       chat turn           goal command
          |                    |
          v                    v
    TurnPipeline          ObjectiveStore (evolved Goal repository)
                               |
                               v
                        GoalCoordinator
                          bounded tick
                               |
                  +------------+-------------+
                  |                          |
                  v                          v
          AttemptCoordinator          ApprovalRepository
                  |
        +---------+----------+
        |                    |
        v                    v
  RuntimeRegistry       OperationTable
 DeepSeek / Pi /         cancellation
 reviewer runtime
        |
        v
 VerificationService
        |
        v
 Goal transition + durable evidence
        |
        +------> MemoryService ------> SQLite / GBrain

 Native Google tools <-- encrypted OAuth credentials
 GoogleSyncManager -----> DurableChannelInbox / normalized events
```

The architecture deliberately has two state layers:

```text
ObjectiveStore (evolved Goal repository): product continuity across restart
ProcessTable:   live execution lifecycle for the current daemon process
```

Neither replaces the other.

## 5. Channel subsystem

### 5.1 Boundary

Shared channel DTOs live in `fabric`; channel implementations and routing live in `executive`:

```text
crates/fabric/src/types/channel.rs
crates/executive/src/impl/channel/
  mod.rs
  inbox.rs
  binding.rs
  router.rs
  telegram/
  gmail/                 # later milestone
```

`InboundMessage` contains `channel_id`, `message_id`, `conversation_id`, `sender_id`, `content`, `timestamp`, and `reply_to_action`. `OutboundMessage` contains the destination conversation, content, actions, and optional reply reference.

### 5.2 Delivery contract

The router uses a SQLite-backed inbox:

1. Receive provider message.
2. Insert `(channel_id, message_id)` with a unique constraint.
3. Resolve the external identity through `ChannelBindingRepository`.
4. Reject unknown principals before starting a turn.
5. Process messages serially per conversation; different conversations may execute concurrently.
6. Persist the result and outbox entry.
7. Mark inbox row complete and advance the provider cursor in one local transaction.
8. Retry incomplete rows after restart.

This provides local at-least-once processing and deduplication. It does not claim distributed exactly-once delivery.

### 5.3 Initial scope

Telegram MVP supports owner allow-list, `/start`, `/chat`, `/goal`, `/goals`, `/status`, `/pause`, `/resume`, `/cancel`, `/approve`, and `/reject`. Text and buttons are required; file ingestion follows after the text loop is stable.

## 6. Persistent Goal runtime

### 6.1 Domain state

`GoalState` is independent from `ProcessState`:

```text
Draft -> Ready -> Running -> Verifying -> AwaitingApproval -> Completed
                    |            |                |
                    +--------> Blocked <-----------+
                    |
                    +--------> AwaitingHuman

Ready/Running/Blocked/AwaitingHuman/AwaitingApproval <-> Suspended
Any non-terminal state -> Cancelled
Retry exhaustion -> Failed
```

`GoalWaitReason` records whether a Goal waits for approval, human input, an external event, or retry backoff. A running Goal's `ProcessId` is optional and may change after restart.

### 6.2 Persistence

The existing `ObjectiveStore` and `objectives` table are evolved compatibly into the Goal repository; no parallel `GoalRepository` or `goals` table is introduced. SQLite remains authoritative for:

- immutable original intent and approved `GoalSpec`;
- current Goal state and version;
- plan/task records;
- attempts and evidence references;
- budget ledger;
- pending approval references;
- active process linkage;
- event journal.

Updates use optimistic versions and append a journal event in the same transaction. On daemon start, non-terminal Goals are recovered; a stale `ProcessId` is cleared and a new process is created only when execution resumes.

### 6.3 Bounded execution

`GoalCoordinator::tick(goal_id)` advances at most one bounded state transition or one attempt. It must not contain an unbounded “run until done” loop. Scheduling another tick is explicit and respects pause, cancellation, deadline, and backoff.

### 6.4 Budget enforcement

The Goal budget ledger reserves and settles:

- total input/output tokens;
- estimated or actual cost;
- attempt count;
- wall-clock deadline.

`AdmissionController` continues to gate side-effecting capability calls. The coordinator checks both systems before execution.

## 7. Runtime selection, retry, and escalation

`RuntimeRegistry` resolves a `RuntimeId` to an `Arc<dyn SubAgentRuntime>`. `SubAgentSpawner` retains its current default-runtime API for compatibility and gains a per-spawn runtime path.

Each attempt stores:

- runtime ID and model/provider name;
- immutable input snapshot;
- token/cost usage;
- start/end timestamps;
- cancellation/timeout state;
- stdout/stderr or model response;
- structured failure class;
- evidence supplied to the next attempt.

`AttemptCoordinator`, not `SupervisorTree`, applies the policy:

```text
transient failure -> bounded exponential backoff -> same runtime
compile/test failure -> retry with verification evidence
tool failure -> retry only when policy marks it retryable
auth/policy failure -> no blind retry; block or request human action
repeated failure -> select escalation runtime
escalation exhausted -> AwaitingHuman or Failed
```

`SupervisorTree` still limits kernel-process restarts caused by unexpected process failure; it does not choose models or mutate attempt context.

## 8. Pi worktree and verification lifecycle

Pi execution is a supervised job:

```text
Create temporary worktree
 -> start sandboxed Pi runtime
 -> capture output and exit status
 -> collect changed files and diff
 -> run VerificationService
 -> persist report and evidence
 -> request approval
 -> on approval, apply through a separate controlled operation
 -> clean worktree
```

Requirements:

- no direct edits to the main worktree;
- no network by default;
- fail closed when an isolation backend is unavailable;
- timeout and cancellation terminate the whole process group;
- verification uses `tokio::process::Command`, bounded output, cancellation, and per-command timeouts;
- required checks are format, compile/check, relevant tests, diff scope, and capability policy;
- architecture/clippy findings may initially be advisory;
- failed worktrees have a retention TTL, count cap, and disk budget.

The daemon PostTurn hook may publish the completed verification report, but it does not itself run the long-lived commands.

## 9. Approval model

`SocketApprovalGate` remains the synchronous gate used inside a live tool turn. Long-lived Goals additionally use `ApprovalRepository` so approval requests survive process restart and can be correlated across Telegram/Gmail.

Every request has an ID, Goal/attempt reference, operation category, risk, human-readable summary, expiration, status, and one-time resolution record. Expiration and delivery failure deny by default. Telegram is the trusted approval channel in the first milestone; Gmail can deliver reports but does not become a trusted approval source until sender verification is proven.

## 10. Google integration

### 10.1 Credential boundary

`McpOAuthProvider` is reused for the protocol flow, but Google refresh tokens must not be passed to the current plaintext `TokenStore`.

Introduce a token persistence abstraction with an encrypted implementation. The encryption key comes from an external secret file with restrictive permissions or an OS credential store; it is never stored beside ciphertext. Existing plaintext stores are rejected or migrated explicitly—never silently treated as encrypted.

### 10.2 Active tools versus background sync

- Agent-invoked Gmail/Calendar/Drive operations are native `ToolRegistry` capabilities and pass through the same admission and approval path as MCP wrappers.
- `GoogleSyncManager` is a dedicated provider client for Gmail history IDs, Calendar sync tokens, and later Drive change tokens.
- Both paths share the OAuth credential provider, normalized DTOs, redaction rules, and account binding.
- Sync cursors and dedup IDs are persisted before claiming restart safety.

### 10.3 Rollout

Google starts with OAuth plus manual read-only Gmail/Calendar queries. Background synchronization is a later milestone. Gmail `[GOAL]` produces a Draft Goal and requires Telegram confirmation before execution.

## 11. GBrain integration

GBrain is an optional supplemental service behind the existing `MemoryService` contract. Local `DefaultMemoryService` remains authoritative; Mnemosyne owns normalized memory/page semantics and the SQLite ingestion spool, while Executive adapts those operations to the verified GBrain HTTP MCP boundary.

The transport reuses the retained Corpus MCP manager and the pinned `query`, `search`, `get_page`, and `put_page` schemas. It does not invent REST endpoints or write GBrain's internal database. The integration also contains configuration, health state, page mapping, and a SQLite-backed ingestion spool. When GBrain is unavailable or its required schema drifts:

- recall degrades to the existing local memory service or an empty supplemental result;
- ingestion remains durably queued locally;
- recovery flushes queued entries with bounded retries;
- malformed 4xx entries move to a dead-letter state rather than retrying forever;
- GBrain never mutates Dasein.

GBrain is not a prerequisite for Telegram or the Goal MVP.

## 12. Security invariants

1. Unknown channel identities do not start LLM turns.
2. Refresh tokens never enter prompts, logs, Goal records, or GBrain.
3. Persisted OAuth tokens are encrypted before Google integration is enabled.
4. Destructive Google actions and applying code changes require explicit approval.
5. Approval timeout, missing delivery, or lost responder resolves to denial.
6. Pi cannot use the main worktree and fails closed without isolation.
7. Goal and attempt budgets are checked before work and settled afterward.
8. Logs redact tokens, authorization headers, email bodies marked sensitive, and secrets.
9. Every external event and approval is deduplicated by a stable provider/request ID.
10. Dasein mutation is outside this project scope.

## 13. Milestones and dependency order

| Milestone | Deliverable | Depends on |
|---|---|---|
| M0 | Baseline regression tests and schema conventions | none |
| M1 | Durable Telegram chat vertical slice | M0 |
| M2 | One persistent bounded Goal with restart recovery | M1 |
| M3 | Per-agent runtimes, attempts, retry, and escalation | M2 |
| M4 | Pi worktree execution and deterministic verification | M3 |
| M5 | Durable approvals and controlled code-apply flow | M4 |
| M6 | Encrypted Google OAuth and manual read-only tools | M1 |
| M7 | Cursor-based Google sync and Gmail Draft Goal channel | M2, M6 |
| M8 | Optional GBrain `MemoryService` backend | M2 |
| M9 | Deployment hardening, backup, health, and quotas | prior enabled milestones |

M6 and M8 may proceed after their prerequisites without blocking the coding-agent vertical slice.

## 14. Acceptance criteria

### First usable release

1. Existing CLI/TUI/daemon tests remain green.
2. Owner Telegram messages enter the current daemon turn path and receive responses.
3. Duplicate Telegram updates do not create duplicate turns.
4. One approved Goal survives daemon restart with immutable original intent.
5. Each tick performs bounded work and respects pause/cancel/deadline.
6. A DeepSeek or equivalent worker attempt records evidence and usage.
7. Pi modifies only a temporary worktree.
8. Required verification evidence exists before approval.
9. Applying the change is a separately approved operation.
10. Restart does not lose Goal, attempt, approval, inbox, or cursor state.

### Google release gate

1. A token-store test proves refresh tokens are not readable as plaintext on disk.
2. Read-only Gmail and Calendar queries work through admitted capabilities.
3. Sync cursors, once enabled, resume without duplicate normalized events.
4. `[GOAL]` email creates only a Draft Goal and requires trusted-channel approval.

### GBrain release gate

1. `GBrainBackend` satisfies `MemoryService` contract tests.
2. Outage does not block core Goal execution.
3. Ingestion survives daemon and GBrain restarts without silent drops.
4. Recalled entries retain provenance, freshness, and temporal validity.

## 15. Explicit non-goals

- multi-user SaaS;
- native mobile application;
- public unauthenticated endpoints;
- unrestricted autonomous writes to Google services;
- automatic merge/push without approval;
- distributed Goal scheduling;
- replacing Native Cognit;
- rewriting GBrain in Rust;
- automatic Dasein mutation;
- implementing every Google product in the first release.
