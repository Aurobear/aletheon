# Observability Stack

> Migrated from `docs/design/observability/observability-stack.md` and observability sections of `docs/design/core/session-lifecycle.md` — code paths updated to aletheon-* crate structure

> Aletheon's diagnostic core as a system-level service, including event classification, Fragment Accumulator, Debug CLI, Prometheus metrics, structured reasoning logs.

**Crate:** `aletheon-runtime`
**Code location:** `aletheon-runtime/src/impl/session/observability/`
**Related modules:** [session.md](session.md)
**Last Updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| EventJournal | Implemented | `aletheon-runtime/src/impl/session/journal.rs` | JSONL append-only log |
| Durable/Ephemeral split | Planned | — | Event classification designed, not started |
| Fragment Accumulator | Planned | `aletheon-runtime/src/impl/session/observability/fragment.rs` | Streaming delta accumulation designed |
| Debug CLI (JSON-RPC) | Planned | — | 8 RPC methods designed |
| Prometheus metrics | Planned | `aletheon-runtime/src/impl/session/observability/metrics.rs` | prometheus-client integration designed |
| ToolTracker | Planned | `aletheon-runtime/src/impl/session/observability/tool_tracker.rs` | Per-callID tool call lifecycle state machine |
| ReasoningLogger | Planned | `aletheon-runtime/src/impl/session/observability/reasoning_logger.rs` | Structured reasoning log |
| EventPublisher | Planned | `aletheon-runtime/src/impl/session/observability/publisher.rs` | Event fan-out to journal, subscribers, metrics |
| FTS5 full-text search | Planned | — | SQLite FTS5 designed |

---

## 1. Durable/Ephemeral Event Classification

Events in the event stream are classified into two types: persistent (Durable) and instant (Ephemeral). Persistent events are written to JSONL logs and can be replayed; instant events are only for real-time display and not persisted.

**EventPersistence enum:** Durable (persist to JSONL, replayable), Ephemeral (real-time push only, not persisted)

**Classification rules:**
- Ephemeral: TextDelta, ReasoningDelta, ToolInputDelta (streaming deltas, real-time display only)
- Durable: all other events

**Extended event bodies (Ephemeral):**
- `TextDelta { message_id, delta }` — text generation delta
- `ReasoningDelta { reasoning_id, delta }` — reasoning process delta
- `ToolInputDelta { call_id, delta }` — tool input delta

**Extended event bodies (Durable):**
- `HookExecuted { hook_name, event_name, result, duration_ms }` — Hook execution record

## 2. Streaming Fragment Accumulator

Streaming deltas (TextDelta/ReasoningDelta/ToolInputDelta) need to accumulate into complete persistent values. Fragment Accumulator collects delta fragments, then flushes as a single Durable event.

Inspired by OpenCode `createLLMEventPublisher.fragments()`, `FragmentAccumulator` maintains three chunk mappings:
- `text_chunks: HashMap<message_id, Vec<delta>>`
- `reasoning_chunks: HashMap<reasoning_id, Vec<delta>>`
- `tool_input_chunks: HashMap<call_id, Vec<delta>>`

Core operations: start_text/append_text/end_text (reasoning and tool input same pattern), and `flush_all()` for flushing all incomplete accumulations on interrupt/error recovery.

Code location: `aletheon-runtime/src/impl/session/observability/fragment.rs`

## 3. Tool Call Lifecycle State Machine

Each tool call tracks complete lifecycle state, preventing duplicate events, detecting inconsistent states, failing incomplete tool calls on interrupt. Inspired by OpenCode per-callID state machine.

**ToolCallState** tracks: assistant_turn_id, name, input_started, input_ended, called, settled, started_at

**ToolTracker** core operations:
- `register(call_id, assistant_turn_id, name)` — register new tool call
- `mark_input_started/ended/called/settled` — state progression
- `unsettled_calls()` — get incomplete tool calls (for interrupt recovery)
- `fail_unsettled(reason)` — generate Failed events for all incomplete tool calls
- `cleanup_settled()` — clean up completed tool calls (prevent memory leak)
- `detect_inconsistencies()` — detect inconsistent states

Code location: `aletheon-runtime/src/impl/session/observability/tool_tracker.rs`

## 4. Structured Reasoning Log (ReasoningLogger)

Difference from EventJournal: EventJournal records session events (for recovery), ReasoningLogger records reasoning process (for debugging and audit).

**ReasoningLogger** features:
- JSONL format, rotated by size (default 100MB), retained 7 days
- Log path: `{base_dir}/reasoning/{session_id}.jsonl`

**ReasoningEntry** contains timestamp, session_id, step, entry_type.

**ReasoningEntryType variants:** LlmRequest, LlmResponse, ToolCallStarted, ToolCallCompleted, ToolCallFailed, Thinking, Checkpoint, HookExecution

Core operations: `log(entry)` writes and checks rotation, `rotate()` renames current file and reopens, `cleanup_old_logs()` cleans logs older than retention days.

Code location: `aletheon-runtime/src/impl/session/observability/reasoning_logger.rs`

## 5. Token Usage Safe Normalization

Inspired by OpenCode `tokens()` helper, defends against NaN/negative values, standardizes to unified structure.

`safe_tokens(value: Option<i64>) -> i64` — filters negative values, None returns 0.

**TokenUsageBreakdown** per-item statistics: input, output, reasoning, cache_read, cache_write. Supports `from_raw()` safe construction, `total()` total, `accumulate()` accumulation.

## 6. Event Schema Version Control

**JournalHeader** carries format_version, session_id, created_at, schema_version, for forward-compatible schema evolution.

Version history:
- v1: Initial version, basic lifecycle, tool, state change events
- v2: Added streaming delta events (TextDelta/ReasoningDelta/ToolInputDelta)
- v3: Added Hook execution events (HookExecuted)

Rules: New fields don't require version bump; deleted or renamed fields require version bump with migration logic.

On replay: First line may be JournalHeader, skip unparseable lines (forward compatibility), warn on incompatible new schema versions.

## 7. Debug CLI — Unix Socket JSON-RPC Protocol

Debug CLI communicates with daemon via Unix socket, using JSON-RPC 2.0 protocol.

**Supported RPC methods:**

| Method | Description | Response Type |
|--------|-------------|---------------|
| `session.status` | Get current session state | JSON (active_sessions, current_step, token_usage, pending_approvals) |
| `session.subscribe` | Subscribe to event stream (with filters) | Streaming SessionEvent |
| `session.replay` | Replay session history | Streaming durable SessionEvent |
| `hooks.list` | List registered hooks | JSON array |
| `metrics.snapshot` | Get metrics snapshot | JSON |
| `memory.status` | Get memory state | JSON (blocks, total_size) |
| `reasoning.recent` | Get last N reasoning steps | JSON array |
| `reasoning.follow` | Stream reasoning log (like tail -f) | Streaming ReasoningEntry |

## 8. Prometheus Metrics Export

Uses `prometheus-client` crate to expose `/metrics` endpoint, default listen `127.0.0.1:9090`.

**Metric categories:**

| Category | Metrics |
|----------|---------|
| Inference performance | inference_duration (Histogram), llm_call_duration (Histogram) |
| Token consumption | tokens_input/output/reasoning/cache_read/cache_write_total (Counter) |
| Tool calls | tool_calls_total/success/failed (Counter), tool_call_duration (Histogram) |
| Sessions | active_sessions (Gauge), sessions_created/resumed/compressed_total (Counter) |
| Hooks | hook_executions_total (Counter), hook_execution_duration (Histogram), hook_blocks_total (Counter) |
| Checkpoints | checkpoint_duration (Histogram), checkpoints_total (Counter) |
| System | memory_usage_bytes (Gauge), journal_size_bytes (Gauge), db_size_bytes (Gauge) |

Core operations: `record_inference(duration, usage)`, `record_tool_call(tool_name, duration, success)`, `record_hook_execution(duration, blocked)`

Code location: `aletheon-runtime/src/impl/session/observability/metrics.rs`

## 9. Integration with SessionStore EventJournal

**EventPublisher** decouples event producers (SessionLoop) and consumers (EventJournal, DebugCLI, MetricsExporter).

Architecture: Receive SessionEvent -> push to real-time subscribers (all events) -> Durable events write to JSONL log -> update metrics -> update FragmentAccumulator and ToolTracker.

`add_live_subscriber()` returns `mpsc::Receiver<SessionEvent>` for Debug CLI streaming output. `cleanup_subscribers()` removes disconnected subscribers.

Code location: `aletheon-runtime/src/impl/session/observability/publisher.rs`

## 10. FUSE Integration

**ReasoningFuseMount** exposes reasoning logs as read-only filesystem:
- Path: `/mnt/agent/logs/reasoning/{session_id}.jsonl`
- Supports: readdir (list .jsonl files), open/read (read log content), getattr (file metadata)
- Does not support write operations

## 11. CLI Command Reference

```bash
# View current session state
aletheon-cli debug status

# Stream reasoning log
aletheon-cli debug follow-reasoning [--filter "Thinking|ToolCall*"]

# View last N reasoning steps
aletheon-cli debug recent-steps [--n 20]

# View memory usage
aletheon-cli debug memory

# View Hook registration state
aletheon-cli debug hooks

# View Prometheus metrics
aletheon-cli debug metrics

# Replay specified session history
aletheon-cli debug replay --session-id <id> [--from-seq 0]

# Subscribe to real-time event stream
aletheon-cli debug subscribe [--filter "ToolCall*"] [--ephemeral]
```

---

## Implementation Summary

**Code locations:**
- `aletheon-runtime/src/impl/session/observability/fragment.rs` — FragmentAccumulator
- `aletheon-runtime/src/impl/session/observability/metrics.rs` — MetricsExporter
- `aletheon-runtime/src/impl/session/observability/publisher.rs` — EventPublisher
- `aletheon-runtime/src/impl/session/observability/reasoning_logger.rs` — ReasoningLogger
- `aletheon-runtime/src/impl/session/observability/tool_tracker.rs` — ToolTracker

**Key types/traits designed:**
- `EventPersistence` — Durable/Ephemeral event classification
- `FragmentAccumulator` — streaming delta accumulation
- `ToolTracker` — per-callID tool call lifecycle state machine
- `ReasoningLogger` — structured reasoning log with rotation and retention
- `TokenUsageBreakdown` — safe token usage normalization
- `JournalHeader` — event schema versioning
- `DebugCli` — Unix socket JSON-RPC 2.0 client with 8 RPC methods
- `MetricsExporter` — Prometheus metrics exporter
- `EventPublisher` — event fan-out to journal, live subscribers, metrics, and state trackers
