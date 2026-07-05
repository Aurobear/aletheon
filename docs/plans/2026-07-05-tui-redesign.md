# Aletheon TUI Redesign — Four-Layer Display Architecture

**Date**: 2026-07-05
**Status**: Draft
**Reference**: Codex TUI (`codex-rs/tui/src/`, ~61,000 lines)

## Motivation

The current system has architectural problems that go beyond the TUI:

1. **Implicit protocol — no shared type between daemon and client.** The wire
   protocol between `runtime` and `interact` is defined by two independent string
   matches: `format.rs` builds `{"type": "text_delta", ...}` by hand, and
   `response.rs:70` matches `"text_delta"` by string. Adding an event requires
   manual sync across two crates. There is no compiler guarantee of consistency.

2. **Raw API internals leak to the terminal.** JSON-RPC wire format
   (`{"jsonrpc":"2.0","method":"event","params":{"type":"text_delta",...}}`)
   is deserialized as `serde_json::Value` and dispatched by string-matching
   `params.type` in `response.rs:38` and `cli.rs:564`. Unrecognized events
   are silently dropped (`_ => {}`).

3. **`base::UiEvent` is dead code.** `base/src/events/ui_event.rs` defines a
   21-variant `UiEvent` enum designed for client-side event abstraction. It
   is never instantiated or matched anywhere. The actual client-side protocol
   parses raw JSON strings.

4. **Tool calls pollute the chat area.** Tool dispatch, raw output, elapsed
   time, and error status are inserted as `ChatRole::System` messages inline
   with the agent's response (`response.rs:120-153`). There is no separable
   tool-call cell with independent collapse/expand behavior.

5. **Code duplication.** `format_reflections`, `format_genome`, `format_evolution`,
   `format_status` are implemented identically in `response.rs` (753 lines) and
   `cli.rs` (922 lines).

## Global Output Architecture (Current State)

aletheon has 10 independent output paths, with no shared type definition
between the daemon and client sides:

```
┌────────────────────────────────────────────────────────────────────┐
│                     aletheon binary (main.rs)                       │
│                                                                     │
│  daemon mode ──► runtime ──► Unix socket (JSON-RPC)                │
│    Event (28 variants) ──► format.rs: event_to_json() ──► socket    │
│                                                                     │
│  TUI mode    ──► interact ──► socket read ──► response.rs           │
│                    serde_json::Value ──► string match "type" ──► App│
│                                                                     │
│  exec mode   ──► println!(final_response)   (独立路径，绕开 daemon) │
│  -m msg      ──► interact::cli::single_message() → stdout           │
│  debug *     ──► interact::debug ──► DebugBus (独立通道)            │
│                                                                     │
│  base::UiEvent ← 21 variants, 定义了但从未被使用（死代码）            │
└────────────────────────────────────────────────────────────────────┘
```

Crate dependency:
```
aletheon (binary)
  ├── runtime (daemon, agent loop, event_sink)  ─┐
  │     ├── base                                  │
  │     ├── cognit                                │ 零代码共享
  │     ├── corpus                                │ 只通过 socket
  │     ├── memory                                │ JSON 字符串通信
  │     ├── dasein                                │
  │     └── metacog                              ─┘
  └── interact (TUI, CLI, debug, ACIX)
        ├── base    ← 不依赖 runtime
        └── corpus
```

## Goal

Fix the root cause: **the protocol is implicit**. The `ClientEvent` type lives
in `base` where both `runtime` and `interact` can share it. The daemon
serializes to this type; the client deserializes from it. The compiler
guarantees both sides agree on the schema.

On top of that shared protocol, build a four-layer display architecture
mirroring Codex's design:

```
┌─────────────────────────────────────────────────────────────────┐
│                    Shared Protocol (base crate)                  │
│                                                                  │
│  ClientEvent enum (replaces dead UiEvent)                        │
│  ├── TurnStarted, TurnDone, Error                                │
│  ├── TextDelta, ThinkingDelta                                    │
│  ├── ToolCallStart, ToolCallResult                               │
│  ├── Usage, ContextUpdate, GoalSet, ModelSwitch                  │
│  ├── AwarenessChanged, PlanUpdate, SubAgentStatus, ModeChanged   │
│  └── Interrupted, BudgetExceeded, CircuitBreakerTripped, ...     │
│                                                                  │
│  daemon: Event → ClientEvent → serde_json → socket               │
│  client: socket → serde_json → ClientEvent → ChatWidget          │
├─────────────────────────────────────────────────────────────────┤
│                    Display Architecture (interact crate)          │
│                                                                  │
│  Layer 1: ClientEvent → ChatWidget typed handlers (no strings)   │
│  Layer 2: HistoryCell trait → AgentMessageCell | ExecCell | ... │
│  Layer 3: Renderable trait → FlexRenderable composite layout     │
│  Layer 4: StreamCore → stable/tail regions, table holdback       │
└─────────────────────────────────────────────────────────────────┘
```

Non-goals:
- Do NOT refactor `aletheon exec` mode — it is a separate code path
- Do NOT change the runtime-internal `Event` enum (28 variants) — it includes
  internal variants (`CompactionStarted`, `MemoryUpdated`, etc.) not needed by the client
- Do NOT touch `acix/` (Agent-Centric Interaction eXperience) module
- Do NOT change `DebugBus` or debug command output paths

## Layer 0: Shared Protocol (`base/src/events/ui_event.rs`)

### Problem

Currently:
- `runtime/format.rs` builds JSON strings by hand: `{"jsonrpc":"2.0","method":"event","params":{"type":"text_delta","text":"..."}}`
- `interact/response.rs` parses `serde_json::Value` and matches `type` by string: `"text_delta" => { ... }`
- `base/ui_event.rs` has a `UiEvent` enum that is never used — dead code

Adding a new event type requires manual sync across 3 files in 2 crates. The
`_ => {}` catch-all silently drops unrecognized events.

### Design

Replace the dead `UiEvent` with `ClientEvent` — a shared protocol type in `base`
that both `runtime` and `interact` depend on:

```rust
// base/src/events/ui_event.rs (rewrite)

/// Client-facing event produced by the daemon and consumed by the TUI/CLI.
///
/// This is the canonical wire-protocol type. Every variant maps to an
/// event notification sent over the Unix socket.
///
/// daemon:  runtime::Event → ClientEvent → serde_json → socket
/// client:  socket → serde_json → ClientEvent → display handler
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientEvent {
    // ── Turn lifecycle ──
    TurnStarted { iteration: usize },
    TurnDone,
    Error { message: String },

    // ── Streaming text ──
    TextDelta { text: String },
    ThinkingDelta { text: String },

    // ── Tool calls ──
    ToolCallStart { call_id: String, tool: String, args: serde_json::Value },
    ToolCallResult {
        call_id: String,
        tool: String,
        output: String,
        is_error: bool,
        elapsed_ms: u64,
    },

    // ── Bookkeeping ──
    Usage { tokens_in: u64, tokens_out: u64 },
    ContextUpdate { max_tokens: u64, used_tokens: u64 },
    GoalSet { goal: String, sub_goals: Vec<String> },
    ModelSwitch { model: String },

    // ── Awareness / collaboration ──
    AwarenessChanged { level: AwarenessLevel, context: String },
    PlanUpdate {
        version: u32,
        plan: String,
        critique: Option<String>,
        ready_for_approval: bool,
    },
    SubAgentStatus {
        agent_id: String,
        task: String,
        status: SubAgentStatus,
    },
    ModeChanged { new: CollaborationMode },

    // ── Limits / interruptions ──
    Interrupted,
    BudgetExceeded { limit: u64 },
    CircuitBreakerTripped { reason: String },
    CompactionTriggered,
    Reflection { summary: String },
}
```

The `#[serde(tag = "type", rename_all = "snake_case")]` attribute means
serde generates the exact same JSON format currently sent on the wire:

```json
{"type": "text_delta", "text": "Hello"}
{"type": "tool_call_start", "call_id": "call_001", "tool": "bash_exec", "args": {...}}
{"type": "turn_start", "iteration": 1}
```

This is a **transparent wire format change** — existing clients parsing
`serde_json::Value` will not break because the JSON is identical.

### Daemon-side: `format.rs` rewrite

Current `event_to_json()` builds JSON by hand with 20 match arms of string
formatting. Replace with typed conversion:

```rust
// runtime/src/impl/daemon/handler/format.rs (rewrite)

use base::events::ui_event::ClientEvent;

impl From<runtime::Event> for Option<ClientEvent> {
    fn from(event: runtime::Event) -> Option<ClientEvent> {
        match event {
            Event::TurnStarted { iteration } =>
                Some(ClientEvent::TurnStarted { iteration }),
            Event::TextDelta { text } =>
                Some(ClientEvent::TextDelta { text }),
            Event::ToolCallStart { call_id, tool, args } =>
                Some(ClientEvent::ToolCallStart { call_id, tool, args }),
            Event::ToolResult { call_id, tool, output, is_error, elapsed_ms } =>
                Some(ClientEvent::ToolCallResult { call_id, tool, output, is_error, elapsed_ms }),
            // ... each Event variant maps to exactly one ClientEvent variant
            // Internal events intentionally dropped:
            Event::CompactionStarted | Event::CompactionDone |
            Event::MemoryUpdated | Event::PlanModeChanged |
            Event::CacheDiagnostics | Event::Text { .. } |
            Event::Reasoning { .. } | Event::ToolCallComplete { .. } |
            Event::ApprovalRequest { .. } | Event::AskRequest { .. } => None,
        }
    }
}

/// Serialize a ClientEvent into a JSON-RPC 2.0 notification line.
pub fn event_to_json(event: ClientEvent) -> serde_json::Result<String> {
    let notification = serde_json::json!({
        "jsonrpc": "2.0",
        "method": "event",
        "params": event,  // serde(tag = "type") gives us {"type": "...", ...}
    });
    serde_json::to_string(&notification)
}
```

The `From<Event>` implementation is an exhaustive match. Adding a new
`Event` variant forces the developer to decide: map it to a `ClientEvent`
(and add a new variant to the enum), or add it to the `None` list of
internal-only events. No silent drops.

### Client-side: `interact` uses `ClientEvent`

```rust
// interact/src/tui/response.rs → deleted, replaced by:
// interact/src/tui/protocol.rs → receives ClientEvent, not needed
//   because deserialization happens inline in ChatWidget

// interact/src/tui/chatwidget/turn_lifecycle.rs

impl ChatWidget {
    /// Handle a JSON-RPC event line from the daemon socket.
    pub fn handle_jsonrpc_line(&mut self, line: &str) -> Result<(), ProtocolError> {
        let msg: serde_json::Value = serde_json::from_str(line)?;

        // Route by JSON-RPC structure
        if let Some(params) = msg.get("params")
            && msg.get("method").and_then(|v| v.as_str()) == Some("event")
        {
            let event: ClientEvent = serde_json::from_value(params.clone())?;
            self.handle_event(event);
        } else if msg.get("result").is_some() || msg.get("error").is_some() {
            self.handle_response(msg);
        }
        Ok(())
    }

    fn handle_event(&mut self, event: ClientEvent) {
        match event {
            ClientEvent::TurnStarted { iteration } => self.on_turn_started(iteration),
            ClientEvent::TextDelta { text } => self.on_text_delta(text),
            ClientEvent::ToolCallStart { call_id, tool, args } =>
                self.on_tool_call_start(call_id, tool, args),
            ClientEvent::ToolCallResult { call_id, tool, output, is_error, elapsed_ms } =>
                self.on_tool_call_result(call_id, tool, output, is_error, elapsed_ms),
            ClientEvent::TurnDone => self.on_turn_done(),
            // ... every variant handled explicitly — no `_ => {}` catch-all
        }
    }
}
```

### Impact

- `base/src/events/ui_event.rs` — **Rewrite**: dead `UiEvent` → live `ClientEvent`
- `runtime/src/impl/daemon/handler/format.rs` — **Rewrite**: manual JSON → `Event → ClientEvent` + serde
- `interact/src/tui/response.rs:handle_event` — **Delete**: string match → typed match in ChatWidget
- `interact/src/tui/cli.rs:single_message` — **Modify**: string match → typed match on `ClientEvent`
- Crate `interact` — now uses `base::ClientEvent` for all daemon communication

## Layer 1: Display Model (`tui/history_cell/`)

### Problem

The current `ChatMessage` (chat.rs:19-24) is a flat struct:

```rust
pub struct ChatMessage {
    pub role: Role,       // User | Assistant | System
    pub content: String,
    rendered: Vec<Line<'static>>,  // pre-rendered, cached
}
```

Everything — tool calls, errors, reflections — is forced into `Role::System`
with a string-formatted message.

### Design

Introduce the `HistoryCell` trait: every distinct display artifact in the
conversation is a concrete cell type.

```rust
// history_cell/mod.rs

pub trait HistoryCell: std::fmt::Debug + Send + Sync + Any {
    /// Styled lines for the main chat display (rich mode).
    fn display_lines(&self, width: u16, caps: &TermCaps) -> Vec<Line<'static>>;

    /// Plain-text lines for transcript export / pager.
    fn raw_lines(&self, width: u16, caps: &TermCaps) -> Vec<Line<'static>>;

    /// Desired height at the given width (0 = no intrinsic preference).
    fn desired_height(&self, width: u16) -> u16 { 0 }

    /// Whether this cell is a continuation of a streaming message
    /// (cell should be updated in-place rather than creating a new one).
    fn is_stream_continuation(&self) -> bool { false }

    /// Size hint: how many physical lines (before wrapping) this cell consumes.
    fn raw_line_count(&self) -> usize { 0 }
}
```

#### Concrete cell types

```rust
// history_cell/messages.rs

pub struct UserMessageCell { pub content: String }
pub struct AgentStreamingTailCell {
    pub content: String,
    pub thinking: Option<String>,
    pub thinking_expanded: bool,
}
pub struct AgentMarkdownCell {
    pub source: String,
    pub cwd: PathBuf,
}
```

```rust
// history_cell/exec.rs

pub struct ExecCell {
    pub call_id: String,
    pub command: String,
    pub tool: String,
    pub args: serde_json::Value,
    pub output: String,
    pub stdout: String,
    pub stderr: String,
    pub exit_code: i32,
    pub is_error: bool,
    pub elapsed_ms: u64,
    pub collapsed: bool,
    pub finished: bool,
}
```

```rust
// history_cell/plans.rs, history_cell/approvals.rs, history_cell/status.rs
pub struct PlanCell { ... }
pub struct ApprovalCell { ... }
pub struct StatusEventCell { ... }
```

#### ChatWidget integration

```rust
// chatwidget.rs

pub struct ChatWidget {
    cells: Vec<Box<dyn HistoryCell>>,
    active_stream: Option<Box<dyn HistoryCell>>,
    scroll_offset: u16,
    render_width: u16,
}
```

ChatWidget consumes `ClientEvent` and maps each variant to a cell transition:

| `ClientEvent` | Cell action |
|--------------|-------------|
| `TurnStarted` | Reset active stream, start new turn |
| `TextDelta` | Push to `AgentStreamingTailCell` |
| `ThinkingDelta` | Update `thinking` field on streaming cell |
| `ToolCallStart` | Create `ExecCell`, insert before active stream |
| `ToolCallResult` | Finalize `ExecCell`, mark finished |
| `TurnDone` | Flush stream → consolidate to `AgentMarkdownCell` |
| `Usage` | Update token counter (internal state, no cell) |
| `Reflection` | Create `StatusEventCell` with summary |
| `Error` | Create `StatusEventCell` with error styling |

### Impact

- `chat.rs:ChatMessage` — replaced by `HistoryCell` trait
- `toolcard.rs` (228 lines) — absorbed by `ExecCell`
- `response.rs:120-153` — tool call formatting deleted
- `format_reflections` / `format_genome` / `format_evolution` — each becomes a cell type

## Layer 2: Layout (`tui/render/renderable.rs`)

### Problem

`draw.rs:51-111` manually computes layout constraints and renders each widget
inline. Cannot compose dynamic elements (tool cards, approval overlays, pager)
without ad-hoc conditionals.

### Design

```rust
// render/renderable.rs

pub trait Renderable {
    fn render(&self, area: Rect, buf: &mut Buffer);
    fn desired_height(&self, width: u16) -> u16;
    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> { None }
    fn cursor_style(&self, area: Rect) -> SetCursorStyle { SetCursorStyle::SteadyBar }
}

// Composite layouts:
pub struct ColumnRenderable { children: Vec<RenderableItem> }
pub struct RowRenderable { children: Vec<(u16, RenderableItem)> }
pub struct FlexRenderable { children: Vec<(u16, RenderableItem)> } // flex_weight
pub struct InsetRenderable { child: RenderableItem, insets: Insets }

pub enum RenderableItem {
    Owned(Box<dyn Renderable>),
    Borrowed(&'static dyn Renderable),
}
```

#### Usage

```rust
impl ChatWidget {
    pub fn as_renderable(&self) -> impl Renderable {
        let mut flex = FlexRenderable::new();
        flex.push(1, self.history_renderable()); // flex 1: fills remaining space
        flex.push(0, self.input_renderable());   // flex 0: fixed height
        flex.push(0, self.status_renderable());  // flex 0: fixed height
        flex
    }
}
```

`draw.rs` simplifies to:

```rust
pub fn draw_with_recorder<B: Backend>(
    terminal: &mut Terminal<B>,
    app: &mut App,
    frame_recorder: &mut Option<FrameRecorder>,
) -> anyhow::Result<()> {
    let renderable = app.chatwidget.as_renderable();
    terminal.draw(|f| renderable.render(f.area(), f.buffer_mut()))?;
    Ok(())
}
```

### Impact

- `draw.rs:51-111` — replaced by `FlexRenderable` composition
- All overlay rendering — composited in `as_renderable()`

## Layer 3: Streaming (`tui/streaming/`)

### Problem

`streaming.rs` has a simple two-phase buffer (thinking + text). It commits
the entire text at once on `turn_done`. No incremental commit, no table awareness.

### Design

Replace with `StreamCore` — two-region model:

```
┌──────────────────────────────────────┐
│ raw_source (accumulated markdown)     │
│   ↓ re-render on every delta          │
│ rendered_lines                        │
│   ↓ split at enqueued_stable_len      │
├──────────────────────────────────────┤
│ Stable region → commit animation queue│
│   → scrollback cells                  │
├──────────────────────────────────────┤
│ Tail region (mutable, transient)      │
│   → StreamingAgentTail cell           │
│   → table holdback                    │
└──────────────────────────────────────┘
```

```rust
// streaming/controller.rs

pub struct StreamCore {
    width: Option<usize>,
    raw_source: String,
    rendered_lines: Vec<Line<'static>>,
    enqueued_stable_len: usize,
    queue: VecDeque<(Line<'static>, Instant)>,
    table_holdback: TableHoldbackState,
    finalized: bool,
}

impl StreamCore {
    pub fn push_delta(&mut self, text: &str);
    pub fn commit_stable(&mut self) -> Vec<Line<'static>>;
    pub fn tail_lines(&self) -> &[Line<'static>];
    pub fn set_width(&mut self, width: usize);
    pub fn finalize(self) -> (Option<AgentMarkdownCell>, String);
}
```

#### Table holdback

Pipe-table header + separator pair (`| a | b |\n|---|---|`) forces all
lines from header onward into the tail until a non-table line after at
least one data row. Prevents row-by-row table flicker.

#### Commit animation

Stable lines drain from the queue with a frame rate-limited tick:

```rust
pub struct CommitTicker {
    queue: VecDeque<(Line<'static>, Instant)>,
    min_delay: Duration,
    max_per_tick: usize,
}
```

#### Stream consolidation

On finalize, contiguous streaming cells collapse into a single
`AgentMarkdownCell` with raw markdown source — re-renders on terminal resize
instead of storing pre-wrapped lines at a fixed width.

### Impact

- `streaming.rs:StreamController` — replaced by `StreamCore`
- `response.rs:text_delta` handler — routes through `ClientEvent::TextDelta`

## Affected Files (Summary)

### `base` crate (shared protocol)

| File | Action |
|------|--------|
| `base/src/events/ui_event.rs` | **Rewrite** — dead `UiEvent` → live `ClientEvent` with `#[serde(tag = "type")]` |

### `runtime` crate (daemon-side serialization)

| File | Action |
|------|--------|
| `runtime/src/impl/daemon/handler/format.rs` | **Rewrite** — manual JSON → `Event → ClientEvent` + serde serialization |

### `interact` crate (TUI/CLI display)

| File | Action |
|------|--------|
| `tui/history_cell/mod.rs` | **New** — `HistoryCell` trait |
| `tui/history_cell/messages.rs` | **New** — `UserMessageCell`, `AgentMessageCell`, `AgentMarkdownCell`, `AgentStreamingTailCell` |
| `tui/history_cell/exec.rs` | **New** — `ExecCell` (absorbs `toolcard.rs`) |
| `tui/history_cell/plans.rs` | **New** — `PlanCell` |
| `tui/history_cell/approvals.rs` | **New** — `ApprovalCell` (absorbs `approval_dialog.rs`) |
| `tui/history_cell/status.rs` | **New** — `StatusEventCell` |
| `tui/chatwidget.rs` | **New** — `ChatWidget`, typed `ClientEvent` handler |
| `tui/chatwidget/rendering.rs` | **New** — `as_renderable()` composition |
| `tui/chatwidget/streaming.rs` | **New** — Stream lifecycle methods |
| `tui/chatwidget/turn_lifecycle.rs` | **New** — Turn state machine |
| `tui/chatwidget/status_state.rs` | **New** — Status bar state |
| `tui/render/renderable.rs` | **New** — `Renderable` trait + composite layouts |
| `tui/render/draw.rs` | **Modify** — simplify to `as_renderable().render()` |
| `tui/streaming/controller.rs` | **Rewrite** — `StreamCore` |
| `tui/streaming/chunking.rs` | **New** — Adaptive chunking |
| `tui/streaming/commit_tick.rs` | **New** — Commit animation ticker |
| `tui/streaming/table_holdback.rs` | **New** — Table holdback detection |
| `tui/markdown_render.rs` | **Modify** — Table auto-width + fence unwrap + syntax highlight |
| `tui/response.rs` | **Delete** — split into protocol (base) + chatwidget/ |
| `tui/chat.rs` | **Replace** — `ChatMessage` → `HistoryCell` cells |
| `tui/toolcard.rs` | **Delete** — absorbed by `ExecCell` |
| `tui/approval_dialog.rs` | **Delete** — absorbed by `ApprovalCell` |
| `tui/cli.rs` | **Reduce** — remove duplicate format_*, keep CLI dispatch |
| `tui/streaming.rs` | **Replace** — simple controller → `StreamCore` |
| `tui/mod.rs` | **Modify** — update module declarations |
| `tui/app/submit.rs` | **Modify** — use `ClientEvent` types |
| `tui/app/lifecycle.rs` | **Modify** — route through typed handler |
| `tui/app/key_handler.rs` | **Modify** — typed notification dispatch |

## Files NOT Changed

- `tui/markdown.rs` — pulldown-cmark parser, used by `markdown_render.rs`
- `tui/term_compat.rs` — terminal capability detection, `Theme`, `TermCaps`
- `tui/state.rs` — `AppState` struct
- `tui/render/header.rs`, `tui/render/input_line.rs` — internal render helpers
- `tui/status.rs` — status bar widget
- `tui/pager.rs` — pager overlay
- `tui/command.rs`, `tui/completion.rs`, `tui/input.rs`, `tui/history_search.rs` — input system
- `tui/skill.rs`, `tui/workflow.rs`, `tui/goal.rs`, `tui/plan_view.rs` — sub-features
- `tui/debug.rs` — debug subcommands (uses DebugBus, separate path)
- `tui/test_infra.rs` — test support
- `crates/aletheon/src/main.rs` — unchanged
- All files under `crates/interact/src/acix/` — unchanged
- `runtime/src/core/event_sink.rs` — `Event` enum unchanged (internal only)
- Debug bus and all debug command paths

## Testing Strategy

1. **Wire format compatibility**: Round-trip `ClientEvent` ↔ JSON in both directions, verify output matches existing format
2. **Event mapping**: `Event → ClientEvent` conversion tested for every variant — confirm internal events return `None`, client events map correctly
3. **Cell rendering snapshots**: Each `HistoryCell` at 80/120/200 column widths
4. **Stream core unit tests**: Delta push + table holdback + finalize scenarios
5. **Integration**: `test_infra.rs` frame recording — capture before/after frames
6. **End-to-end**: `sudo bash setup.sh` + `aletheon -m` and `aletheon exec` + TUI smoke test

## Risks

| Risk | Mitigation |
|------|-----------|
| Wire format change breaks existing clients | `#[serde(tag = "type", rename_all = "snake_case")]` produces identical JSON to current `event_to_json()` output |
| `ClientEvent` variants out of sync with `Event` | Exhaustive match in `From<Event>` forces compiler to flag new variants |
| `interact` now depends on `base::ClientEvent` type | Already depends on `base` for `ChatRole`, `TermCaps`, etc. No new dependency |
| Layout regressions | Start with `FlexRenderable` matching current `draw.rs` constraints; add incrementally |
| Cell memory leaks | `ChatWidget::clear_active_stream()` on `TurnStarted` |
| Table holdback false positives | Only trigger on header + separator pair (`|---|---|`) |

## Implementation Phases

1. **Phase 0: Shared protocol** — `ClientEvent` in `base`, `From<Event>` in `format.rs` (wire format unchanged, no display changes)
2. **Phase 1: Protocol boundary** — `ChatWidget::handle_event(match ClientEvent)` replaces `response.rs` string dispatch
3. **Phase 2: Renderable layout** — `Renderable` trait + composites, refactor `draw.rs`
4. **Phase 3: HistoryCell + ChatWidget** — Replace `ChatMessage`, introduce cell types
5. **Phase 4: Streaming** — `StreamCore` + commit animation + table holdback
6. **Phase 5: Polish** — Markdown table rendering, syntax highlight, fence unwrap
7. **Phase 6: Cleanup** — Delete `response.rs`, `toolcard.rs`, `approval_dialog.rs`; deduplicate `cli.rs`
