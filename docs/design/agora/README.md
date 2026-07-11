# Agora Crate — Shared Cognitive Workspace

> The shared cognitive workspace (blackboard) — working memory that reasoning
> operates on within a single turn/session. Session-isolated, in-memory,
> never persistent by itself.

**Crate:** `agora`
**Source:** `crates/agora/src/`
**RFC:** [RFC-014 Agora Architecture](../../architecture/RFC-014-Agora-Architecture.md), [RFC-017 Aletheon Primitives](../../architecture/RFC-017-Aletheon-Primitives.md)

---

## Purpose

Agora is **not** long-term memory and **not** a planner. It is the active
cognitive environment in which reasoning occurs during a turn: hypotheses,
evidence, intermediate conclusions, task decomposition, attention focus, and
the reasoning trace all live here while the turn is in progress.

Principles (RFC-014 §"Principles"):
- Shared but scoped
- Session isolated
- Fast access (in-memory)
- Never persistent by itself — persistence is Mnemosyne's job

## Position in the layering

```
Cognit ↓ Agora ↓ Corpus
```

Cognit (the reasoner) reads from and writes to Agora as it thinks; Corpus
(tool execution) results are recorded into Agora's trace. Agora itself never
talks to storage — Mnemosyne is the only subsystem that persists cognitive
state across restarts (RFC-017 §1 invariants).

## Crate Structure

```
crates/agora/src/
├── lib.rs           — crate root, re-exports
├── workspace.rs      — Workspace: one session's aggregated cognitive state
├── blackboard.rs     — Blackboard: JSON key-value shared area
├── attention.rs      — Attention: current focus + ranked priorities
├── task_graph.rs     — TaskGraph: sub-task nodes, dependencies, status
├── trace.rs          — Trace: append-only reasoning/tool-output log
├── scratchpad.rs     — Scratchpad: task-level ephemeral k/v store (migrated from mnemosyne)
└── ops.rs            — AgoraRegistry: per-session registry, implements AgoraOps
```

### Modules

- **`Blackboard`** (`blackboard.rs`) — a JSON key-value area for hypotheses,
  evidence, and intermediate conclusions. Absorbs what earlier drafts called
  observation/artifact/context. Supports `set`/`get`/`remove`/`merge` (object
  patch) and `to_json()` for snapshotting.
- **`Attention`** (`attention.rs`) — the workspace's current focus (`Option<String>`)
  and a ranked `priorities: Vec<String>` list. `set_focus()` promotes a focus
  to the front of the priority list (dedup); `clear_focus()` clears the
  current focus without touching history.
- **`TaskGraph`** (`task_graph.rs`) — a directed graph of `TaskNode`s keyed by
  id, each with a `status` (`Pending`/`Running`/`Done`/`Failed`) and `deps`.
  `ready()` returns pending nodes whose dependencies are all `Done`.
- **`Trace`** (`trace.rs`) — an append-only log of `TraceEntry { kind, content }`,
  e.g. `"tool_output"`, `"sub_agent"`, `"reasoning"`. This is the reasoning
  trace RFC-014 calls out; it is what the Reflector consumes after a turn.
- **`Scratchpad`** (`scratchpad.rs`) — a task-level ephemeral key/value store
  keyed by `agent_id`/`task_id`, with a `RetentionPolicy` (`Discard`,
  `ArchiveToAgent`, `ArchiveToSession`) describing what happens to its
  entries when the task completes. **Migrated here from `mnemosyne`** per
  RFC-014; it is a standalone type in this crate — `Workspace` does not
  currently aggregate it (see Known gaps below).
- **`Workspace`** (`workspace.rs`) — one session's cognitive workspace,
  aggregating `blackboard`, `attention`, `task_graph`, and `trace`.
  `snapshot()` serializes the whole workspace to JSON (session id,
  blackboard contents, attention focus/priorities, task count, trace
  length) for debugging or committing to Mnemosyne. `clear()` resets all
  state but keeps the session id.

## AgoraOps

`AgoraRegistry` (`ops.rs`) owns one `Workspace` per session id (`HashMap<String, Workspace>`
behind a `tokio::sync::Mutex`) and implements the `AgoraOps` trait
(`fabric::ops::AgoraOps`):

| Method | Signature | Behavior |
|--------|-----------|----------|
| `publish` | `(session, key, value) -> Result<()>` | Sets a key on the session's blackboard (creates the workspace if absent). |
| `recall` | `(session, key) -> Result<Option<Value>>` | Reads a key from the session's blackboard; `None` if the session or key doesn't exist. |
| `update` | `(session, patch: Value) -> Result<()>` | Merges a JSON object patch into the session's blackboard. |
| `snapshot` | `(session) -> Result<Value>` | Returns `Workspace::snapshot()` for the session, or `Value::Null` if the session doesn't exist. |
| `clear` | `(session) -> Result<()>` | Clears the session's workspace state (no-op if the session doesn't exist). |
| `trace` | `(session, kind, content: Value) -> Result<()>` | Appends an entry to the session's trace (creates the workspace if absent). |

All boundary payloads are `serde_json::Value` today, not the typed
`fabric::primitives::cognitive` objects (Hypothesis/Evidence/etc.) — see
RFC-018 §2 (D2) for the tracked gap between the primitive vocabulary and
what actually crosses the wire.

## Lifecycle: recall → publish → reason → trace → snapshot → commit

Per RFC-017 §4 (composition of one turn) and RFC-014's "Lifecycle" section:

```
Input → Context Build
  → Mnemosyne.recall()                       (past experience)
  → AgoraOps::publish(session, key, value)    (recall injection onto the blackboard)
  → Reasoning (Planner → Reasoner; Hypothesis/Evidence land on the blackboard)
  → Tool execution (Corpus) → AgoraOps::trace(session, "tool_output"/"sub_agent", ...)
  → Reflection
  → AgoraOps::snapshot(session) → Mnemosyne.store()   (commit)
```

In other words: memories recalled from Mnemosyne are *published* into Agora
at the start of a turn so reasoning can see them; everything produced during
the turn (hypotheses, tool results, sub-agent results) accumulates on the
blackboard/trace; at turn end the workspace is *snapshotted* and handed to
Mnemosyne to persist. Agora itself is cleared or discarded across restarts —
it holds no state that survives on its own.

## Known gaps / in-progress work

Per RFC-018 (§3 "Agora shared workspace" gap, Phase 1 roadmap item, current
as of 2026-07-10):
- Only `turn_input` is published in the live daemon path today; tool outputs
  and sub-agent results are not yet routinely written to the trace.
- `snapshot()` output was only logged, not yet persisted to Mnemosyne via
  `MnemosyneOps::store()` — closing this loop is tracked as RFC-018 Phase 1.
- `Scratchpad` is migrated into this crate but not yet wired into `Workspace`
  as a field; it exists as a standalone type constructed directly by callers
  that need task-level scratch space.

## Related Docs

- [RFC-014 Agora Architecture](../../architecture/RFC-014-Agora-Architecture.md)
- [RFC-017 Aletheon Primitives](../../architecture/RFC-017-Aletheon-Primitives.md) — `AgoraOps` trait definition, cognitive/communication primitives
- [RFC-018 Refactor-Debt Reconciliation](../../architecture/RFC-018-Refactor-Debt-Reconciliation.md) — Agora persistence gap and roadmap
- [mnemosyne/README.md](../mnemosyne/README.md) — where Agora snapshots are committed
