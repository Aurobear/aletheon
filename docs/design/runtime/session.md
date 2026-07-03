# Session Persistence and Lifecycle

> Migrated from `docs/design/core/session-lifecycle.md` (session persistence and crash recovery sections only) — code paths updated to match actual crate names (base, cognit, corpus, dasein, memory, metacog, interact, runtime)

> Session persistence, EventJournal, crash recovery. Observability sections extracted to [observability.md](observability.md).

**Module:** 10
**Crate:** `runtime`
**Code location:** `runtime/src/impl/session/`
**Related modules:** [react-loop.md](react-loop.md), [observability.md](observability.md)
**Last Updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| SessionStore | Implemented | `runtime/src/impl/session/store.rs` | Session CRUD + metadata |
| EventJournal | Implemented | `runtime/src/impl/session/journal.rs` | JSONL append-only log + SQLite index |
| Session recovery | Partial | `runtime/src/impl/session/journal.rs` | `recover()` exists but unused in practice |
| InterruptManager | Planned | — | Designed but not started |
| CompressionManager | Planned | — | Designed but not started |
| SessionHierarchy | Planned | — | Multi-agent session tree not started |

---

## 1. Overview

Session management and lifecycle is the core infrastructure for Aletheon as a system-level service. Covers session persistence, crash recovery, and the Hook system.

As an always-on daemon, Aletheon must solve three key problems:
1. **Persistence** — After daemon crash or upgrade, session state cannot be lost
2. **Extensibility** — Users need to inject custom logic at key points in the reasoning loop
3. **Diagnosability** — Long-running daemon needs real-time observable internal state

---

## 2. Identified Defects

### P0: Session Persistence and Crash Recovery

**Problem:** daemon crash = everything lost. As a system-level service, daemon may be interrupted by OOM, kernel panic, upgrade restart, etc. All session state (conversation history, current task progress, tool call intermediate results) is in memory, any interruption means complete loss.

### P0: Crash Recovery Boundary Conditions Undefined

**Problem:** The session persistence design uses JSONL event log + SQLite index + `CheckpointBoundary` mechanism, which guarantees consistent recovery points under normal operation. But the design does not explicitly define boundary conditions and recovery protocols for three crash scenarios:

**Scenario A:** Daemon crash, tool call not started — `CheckpointBoundary` already fsynced, can retry tool call after recovery.

**Scenario B:** Daemon crash, tool call in-flight — state semantics unclear, tool-side subprocess fate unknown.

**Scenario C:** Tool side effects produced, daemon crash — tool has modified system state but no record in event log.

---

## 3. Improved Design

### 3.1 Session Persistence — Event Journal + SQLite Index

Core architecture change: use **append-only event log** instead of full blob checkpoint, SQLite only as index and metadata storage.

#### 3.1.1 Event Type System (SessionEvent)

`SessionEvent` is the basic unit of the append-only log. Each event contains `seq` (monotonically incrementing sequence), `correlation_id`, `timestamp`, and `body` (event body).

Event body `SessionEventBody` is a tagged union covering:
- **Lifecycle:** SessionStarted, SessionEnded
- **User interaction:** UserMessage, AssistantMessage
- **Tool execution:** ToolCallStarted, ToolCallCompleted, ToolCallFailed
- **State change:** LoopStateChanged, CoreMemoryChanged, PermissionChanged
- **Context management:** Compacted (pre/post compression summary)
- **Checkpoint boundary:** CheckpointBoundary (consistent recovery point, written before each tool call)
- **Approval flow:** ApprovalRequested, ApprovalResolved
- **Multi-agent:** SubAgentSpawned, SubAgentCompleted

`SessionSource` enum identifies session source: Cli, Daemon, SubAgent, Review, MemoryConsolidation.
`EndReason` enum identifies end reason: UserExit, TaskCompleted, Error, Interrupted, Compression, DaemonShutdown.

#### 3.1.2 Event Log Storage (EventJournal)

`EventJournal` implements append-only event log, inspired by Codex RolloutRecorder. Architecture highlights:

- **Dedicated writer task** — receives write commands via `mpsc::channel`, single-thread sequential writes, avoiding lock contention
- **JSONL + SQLite hybrid** — event bodies stored in JSONL file (append-only, efficient writes), SQLite stores only index metadata (supports fast queries)
- **JournalCmd protocol** — Append (batch append), Persist (fsync flush), Flush (buffer refresh), Shutdown (close log)
- **create()** — create new session log, returns (journal, writer_task_handle)
- **resume()** — restore existing session: replay JSONL file, find consistent state from last CheckpointBoundary, return events needing replay
- **append()** — non-blocking, sends to writer task via channel
- **checkpoint()** — write CheckpointBoundary and fsync, ensuring recovery point persistence
- **WAL checkpoint strategy** — execute `PRAGMA wal_checkpoint(TRUNCATE)` every 50 writes, preventing WAL file unbounded growth

Code location: `runtime/src/impl/session/journal.rs`

#### 3.1.3 Thread Store Abstraction (ThreadStore trait)

`ThreadStore` trait defines core session storage abstraction:

```rust
#[async_trait]
trait ThreadStore: Send + Sync {
    async fn create_session(&self, params: CreateSessionParams) -> Result<SessionHandle>;
    async fn resume_session(&self, session_id: &str) -> Result<ResumeResult>;
    async fn fork_session(&self, parent_id: &str, params: ForkParams) -> Result<SessionHandle>;
    async fn read_session_meta(&self, session_id: &str) -> Result<SessionMeta>;
    async fn list_resumable(&self, limit: usize) -> Result<Vec<SessionSummary>>;
    async fn delete_session(&self, session_id: &str) -> Result<()>;
}
```

Key types:
- `CreateSessionParams` — source, model, cwd, parent_session_id, initial_memory, personality
- `ForkParams` — from_seq (fork point event sequence), source
- `ResumeResult` — handle, replay_events, last_checkpoint
- `SessionHandle` — session_id, journal, writer_task
- `CheckpointState` — loop_state, message_count, token_usage

`LocalThreadStore` is the local filesystem-based implementation, session metadata stored in SQLite.

#### 3.1.4 Initial History State Machine (InitialHistory)

`InitialHistory` enum defines four session startup modes, inspired by Codex's InitialHistory:
- **New** — Brand new session
- **Cleared** — Session with cleared history (new session_id, no prior history)
- **Resumed** — Resume existing session, replay events from last CheckpointBoundary
- **Forked** — Forked from parent session, copies event history

`SessionInitGuard` inspired by Codex `LiveThreadInitGuard`, ensures initialization atomicity: success calls `commit()`, failure discards uncommitted state.

#### 3.1.5 SQLite Schema

SQLite stores four types of data:
- **sessions** — Session metadata (session_id, source, model, cwd, parent/fork relationship, start/end time, token statistics)
- **event_index** — Event index (session_id, seq, correlation_id, event_type, timestamp), does not store complete event body
- **compression_locks** — Compression locks (prevents concurrent compression, TTL-based)
- **memory_blocks** — Core Memory blocks (stored independently, not in event log)

#### 3.1.6 Write Contention Handling

`WriteExecutor` inspired by Hermes jitter retry pattern to solve multi-process concurrent SQLite write contention:
- Uses `BEGIN IMMEDIATE` to acquire write lock at transaction start
- On `SQLITE_BUSY`, random backoff (20-150ms, max 15 retries), avoiding convoy effect

#### 3.1.7 WAL Fallback Detection

Detect WAL effectiveness at startup. NFS/SMB/FUSE and other network filesystems do not support WAL, auto-fallback to DELETE mode + `synchronous=FULL`.

#### 3.1.8 Session Compression / Context Splitting

`CompressionManager` compresses and creates continuation session when context window approaches limit (default 80%):
- Acquire compression lock (TTL-based, prevents deadlock)
- Generate history summary
- End current session (EndReason::Compression)
- Create continuation session, inject summary as first message
- `get_compression_tip()` follows compression chain to find active continuation session

#### 3.1.9 Interrupt/Resume Protocol

`SessionInterrupt` inspired by LangGraph `GraphInterrupt`, for human-in-the-loop breakpoints (approval, confirmation, input):
- `InterruptReason` — ApprovalRequired, UserInputRequired, HumanBreakpoint
- `PendingWrite` — Separates write and apply, supports atomic commit
- `InterruptManager` — raise() triggers interrupt and persists, resume() restores and applies pending_writes

#### 3.1.10 Multi-Agent Session Hierarchy

`SessionHierarchy` supports parent-child session hierarchy, for sub-agent, review thread, memory consolidation:
- create_child() — create child session
- get_tree() — recursively get session hierarchy tree
- get_ancestors() — get ancestor chain (for context passing)

#### 3.1.11 Declarative Schema Management

`SchemaManager` inspired by Hermes declarative schema coordination:
- Single `SCHEMA_SQL` defines target schema
- `_reconcile_columns()` auto-detects and adds missing columns at startup
- No version migration chain: column additions don't need version numbers, data transformations still need version-gated migration

---

## 4. Crash Recovery Protocol — Three Scenarios

### 4.1 Extended Tool Call State Marker

Add `ToolCallInFlight` state (call_id, tool_name, args, child_pid, started_at) to `SessionEventBody`, enabling recovery logic to distinguish scenario A (not started) from scenario B (state unknown).

Write sequence becomes:
```
CheckpointBoundary -> ToolCallStarted -> ToolCallInFlight -> execute tool -> ToolCallCompleted/Failed
```

### 4.2 Scenario A Recovery Protocol (after checkpoint, tool not started)

Replay to last CheckpointBoundary -> check for ToolCallStarted without ToolCallInFlight -> auto-retry tool call (default) or ask user (configurable).

### 4.3 Scenario B Recovery Protocol (tool in-flight)

Replay to last CheckpointBoundary -> check for ToolCallInFlight without Completed/Failed -> query subprocess PID still alive -> present recovery options to user (retry/skip/terminate).

### 4.4 Scenario C Recovery Protocol (side effects produced)

Replay to last CheckpointBoundary -> query audit log to determine if tool was executed -> backfill ToolCallCompleted event -> if side effects are rollbackable, ask user.

### 4.5 Audit Log as Secondary Recovery Source

Security model's audit log is written immediately after tool execution, closer to real-time than session event log. Recovery protocol queries audit log when session log is incomplete to determine actual tool call results. Requires adding `call_id` field to audit log for correlation.

### 4.6 Orphan Process Management

Daemon scans all in-flight tool calls at startup, handles orphan processes based on configured policy:
- **Wait** — wait for subprocess to complete (timeout 30s)
- **Terminate** — terminate subprocess
- **Detach** — mark as detached, no longer managed

Configuration (`/etc/aletheon/aletheon.toml`):

```toml
[lifecycle.crash_recovery]
orphan_policy = "terminate"
orphan_wait_timeout = 30
auto_retry_inflight = false
use_audit_as_fallback = true
```

---

## Implementation Summary

**Code locations:**
- `runtime/src/impl/session/store.rs` — SessionStore (session CRUD + metadata)
- `runtime/src/impl/session/journal.rs` — EventJournal (JSONL append-only log + SQLite index), session recovery

**Key types/traits implemented:**
- `SessionStore` — session CRUD operations and metadata management
- `EventJournal` — append-only JSONL event log with SQLite indexing
- `SessionEvent` / `SessionEventBody` — event type system
- `ThreadStore` trait — session storage abstraction (create/resume/fork/list/delete)
- `CheckpointBoundary` — consistent recovery point marker

**Planned (not started):**
- InterruptManager — human-in-the-loop interrupt/resume protocol
- CompressionManager — context window compression and session chaining
- SessionHierarchy — multi-agent session tree management
