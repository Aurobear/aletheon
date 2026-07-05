# Aletheon TUI Redesign — Four-Layer Display Architecture

**Date**: 2026-07-05
**Status**: Draft
**Reference**: Codex TUI (`codex-rs/tui/src/`, ~61,000 lines)

## Motivation

The current TUI has three problems:

1. **Raw API internals leak to the terminal.** JSON-RPC wire format
   (`{"jsonrpc":"2.0","method":"event","params":{"type":"text_delta",...}}`)
   is deserialized as `serde_json::Value` and dispatched by string-matching
   `params.type` in `response.rs:38` and `cli.rs:564`. There is no typed
   protocol boundary.

2. **Tool calls pollute the chat area.** Tool dispatch, raw output, elapsed
   time, and error status are inserted as `ChatRole::System` messages inline
   with the agent's response (`response.rs:120-153`). There is no separable
   tool-call cell with independent collapse/expand behavior.

3. **Code duplication and flat structure.** `format_reflections`,
   `format_genome`, `format_evolution`, and `format_status` are implemented
   identically in `response.rs` (753 lines) and `cli.rs` (922 lines), and the
   response handler grows with every new event type.

## Goal

A four-layer architecture that mirrors Codex's proven design:

```
Layer 1: Protocol     → Strongly-typed ServerNotification, hides JSON-RPC wire format
Layer 2: Display Model → HistoryCell trait, one cell type per conversation artifact
Layer 3: Layout        → Renderable trait + composite layouts (Flex/Column/Row)
Layer 4: Streaming     → Two-region (stable + tail), table holdback, commit animation
```

Non-goals for this design:
- Do NOT refactor `aletheon exec` mode (local agent loop) — it is a separate code path
- Do NOT change the daemon-side `Event` enum or wire protocol — only the TUI client side
- Do NOT touch `acix/` (Agent-Centric Interaction eXperience) module

## Layer 1: Protocol (`tui/protocol.rs`)

### Problem

`response.rs:37-55` parses every JSON-Line into `serde_json::Value`, then
dispatches by string comparison:

```rust
// Current (response.rs:38)
if msg.get("method").and_then(|v| v.as_str()) == Some("event") {
    if let Some(params) = msg.get("params") {
        handle_event(app, params); // string match on "type" field
    }
}
```

`handle_event` (response.rs:70) then does another level of string dispatch:

```rust
match event_type {
    "turn_start" => { ... }
    "thinking_delta" => { ... }
    "text_delta" => { ... }
    "tool_call_start" => { ... }
    // ... 15 more branches
    _ => {} // silences unknown events
}
```

### Design

Replace with a single strongly-typed enum. Deserialize once at the protocol
boundary, convert to an internal `ServerNotification`, and route to typed
handler methods. Unknown or malformed messages are rejected at the boundary.

```rust
// protocol.rs

/// Wire-protocol notification from the aletheon daemon.
/// Each variant corresponds to a `params.type` value in the JSON-RPC event.
#[derive(Debug, Clone)]
pub enum ServerNotification {
    // ── Turn lifecycle ──
    TurnStarted { iteration: usize },
    TurnDone,
    Error { message: String },

    // ── Streaming text ──
    TextDelta { text: String },
    ThinkingDelta { text: String },

    // ── Tool calls ──
    ToolCallStart { call_id: String, tool: String, args: serde_json::Value },
    ToolCallResult { call_id: String, output: String, is_error: bool, elapsed_ms: u64 },

    // ── Bookkeeping ──
    Usage { tokens_in: u64, tokens_out: u64 },
    ContextUpdate { max_tokens: u64, used_tokens: u64 },
    GoalSet { goal: String, sub_goals: Vec<String> },
    ModelSwitch { model: String },

    // ── Awareness / collaboration ──
    AwarenessChanged { level: AwarenessLevel, context: String },
    PlanUpdate { version: u32, plan: String, critique: Option<String>, ready_for_approval: bool },
    SubAgentStatus { agent_id: String, task: String, status: SubAgentStatus },
    ModeChanged { new: CollaborationMode },
    Interrupted,
    BudgetExceeded { limit: u64 },
    CircuitBreakerTripped { reason: String },
    CompactionTriggered,
    Reflection { summary: String },
}
```

Deserialization is centralized:

```rust
impl ServerNotification {
    /// Parse one line from the JSON-RPC event stream.
    /// Returns `None` for non-event frames (responses, approvals) or malformed input.
    pub fn from_jsonrpc_line(line: &str) -> Option<Self> {
        let msg: serde_json::Value = serde_json::from_str(line).ok()?;
        if msg.get("method")?.as_str()? != "event" { return None; }
        Self::from_params(&msg["params"])
    }

    fn from_params(params: &serde_json::Value) -> Option<Self> { /* ... */ }
}
```

### Impact

- `handle_event()` (response.rs:70-260) — deleted, replaced by typed match in ChatWidget
- `process_response()` (response.rs:328-374) — split into `ServerNotification` + JSON-RPC response handler
- All `unwrap_or("")` / `unwrap_or(0)` default-value fallbacks — replaced by deserialization errors that log and skip

## Layer 2: Display Model (`tui/history_cell/`)

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
with a string-formatted message. There is no way to render a tool call
differently from a system message, or to collapse/expand tool output.

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

/// User message cell — styled with background tint, prefix "› ".
pub struct UserMessageCell {
    pub content: String,
    rendered: OnceCell<Vec<Line<'static>>>,
}

/// Agent streaming tail cell — mutable during stream, replaced on finalize.
pub struct AgentStreamingTailCell {
    pub content: String,
    pub thinking: Option<String>,  // collapsed reasoning block
    pub thinking_expanded: bool,
}

/// Finalized agent message — owns raw markdown source, re-renders on resize.
pub struct AgentMarkdownCell {
    pub source: String,    // raw markdown
    pub cwd: PathBuf,      // for relativizing file links
}
```

```rust
// history_cell/exec.rs

/// Executed tool call — independent cell in scrollback.
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
    pub collapsed: bool,   // user can toggle with Enter
    pub finished: bool,    // false while streaming output
}
```

```rust
// history_cell/plans.rs, history_cell/approvals.rs, history_cell/status.rs
pub struct PlanCell { ... }
pub struct ApprovalCell { ... }
pub struct StatusEventCell { ... }  // reflections, awareness changes, etc.
```

#### ChatWidget integration

```rust
// chatwidget.rs

pub struct ChatWidget {
    /// Ordered list of every cell in the conversation.
    cells: Vec<Box<dyn HistoryCell>>,

    /// Active streaming cell (if any) — rendered in the tail slot.
    active_stream: Option<Box<dyn HistoryCell>>,

    /// Scroll offset in lines.
    scroll_offset: u16,

    /// Render width cache.
    render_width: u16,
}
```

### Impact

- `chat.rs:ChatMessage` — replaced by `HistoryCell` trait
- `toolcard.rs` (228 lines) — absorbed by `ExecCell`
- Tool call inline formatting (response.rs:120-153) — deleted, tool calls are cells
- `format_reflections` / `format_genome` / `format_evolution` — each becomes a cell type

## Layer 3: Layout (`tui/render/renderable.rs`)

### Problem

`draw.rs:51-111` manually computes layout constraints and renders each widget
inline. This works for a simple 4-section layout (header/chat/input/status) but
cannot compose dynamic elements like inline tool cards, approval overlays, or
pager transitions without ad-hoc conditionals.

### Design

Introduce the `Renderable` trait and composite layout types — directly inspired
by Codex's `render/renderable.rs`:

```rust
// render/renderable.rs

pub trait Renderable {
    fn render(&self, area: Rect, buf: &mut Buffer);
    fn desired_height(&self, width: u16) -> u16;
    fn cursor_pos(&self, area: Rect) -> Option<(u16, u16)> { None }
    fn cursor_style(&self, area: Rect) -> SetCursorStyle { SetCursorStyle::SteadyBar }
}

// ── Composite layouts ──

/// Stack children vertically, each at its desired_height.
pub struct ColumnRenderable {
    children: Vec<RenderableItem>,
}

/// Stack children horizontally with fixed column widths.
pub struct RowRenderable {
    children: Vec<(u16, RenderableItem)>, // (width, child)
}

/// Flutter-inspired flex layout: children with `flex > 0` share remaining space.
pub struct FlexRenderable {
    children: Vec<(u16, RenderableItem)>, // (flex_weight, child)
}

/// Add insets (padding) around a child.
pub struct InsetRenderable {
    child: RenderableItem,
    insets: Insets,
}

/// Enum wrapping owned or borrowed renderable.
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
        flex.push(/*flex*/ 1, self.history_renderable()); // fills remaining space
        flex.push(/*flex*/ 0, self.input_renderable());   // fixed height
        flex.push(/*flex*/ 0, self.status_renderable());  // fixed height
        flex
    }

    fn history_renderable(&self) -> impl Renderable {
        let mut col = ColumnRenderable::new();
        // Scrollback: finalized cells with scroll window
        col.push(self.scrollback_renderable());
        // Active stream tail (if streaming)
        if let Some(cell) = &self.active_stream {
            col.push(RenderableItem::Owned(Box::new(StreamingTailRenderable {
                cell: cell.as_ref(),
                shimmer: self.shimmer_start.elapsed(),
            })));
        }
        // Pending tool cells
        for exec in &self.pending_tool_output {
            col.push(RenderableItem::Owned(Box::new(exec)));
        }
        col
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
    terminal.draw(|f| {
        let area = f.area();
        renderable.render(area, f.buffer_mut());
        // frame recording (unchanged)
    })?;
    Ok(())
}
```

### Impact

- `draw.rs:51-111` — replaced by `FlexRenderable` composition
- `response.rs:handle_event` — approval dialog becomes an `ApprovalCell` in the flow
- All overlay rendering (approval, pager, completion) — composited in `as_renderable()`

## Layer 4: Streaming (`tui/streaming/`)

### Problem

`streaming.rs` has a simple two-phase buffer (thinking + text). It commits the
entire text at once when `turn_done` arrives. There is no incremental commit
to scrollback during streaming, and no handling of special content like tables.

### Design

Replace with `StreamCore` — a two-region model:

```
┌──────────────────────────────────────┐
│ raw_source (accumulated markdown)     │
│   ↓ re-render on every delta          │
│ rendered_lines: Vec<HyperlinkLine>   │
│   ↓ split at enqueued_stable_len      │
├──────────────────────────────────────┤
│ Stable region (committed to queue)    │
│   → commit animation ticks            │
│   → scrollback cells                  │
├──────────────────────────────────────┤
│ Tail region (mutable, transient)      │
│   → StreamingAgentTail cell           │
│   → table holdback keeps tables here  │
└──────────────────────────────────────┘
```

```rust
// streaming/controller.rs

pub struct StreamCore {
    /// Current rendering width.
    width: Option<usize>,
    /// Accumulated raw markdown source.
    raw_source: String,
    /// Full re-render of raw_source at current width.
    rendered_lines: Vec<Line<'static>>,
    /// Number of lines enqueued to the commit animation.
    enqueued_stable_len: usize,
    /// Commit animation queue: (line, enqueue_time).
    queue: VecDeque<(Line<'static>, Instant)>,
    /// Table holdback state — keeps partial tables in tail.
    table_holdback: TableHoldbackState,
    /// Whether the stream is finalized (no more deltas).
    finalized: bool,
}

impl StreamCore {
    /// Push a text delta and re-render.
    pub fn push_delta(&mut self, text: &str);

    /// Commit a batch of stable lines to the animation queue.
    /// Returns the lines to enqueue.
    pub fn commit_stable(&mut self) -> Vec<Line<'static>>;

    /// Get the current tail lines (for the transient streaming cell).
    pub fn tail_lines(&self) -> &[Line<'static>];

    /// Set the render width (triggers re-render of accumulated source).
    pub fn set_width(&mut self, width: usize);

    /// Finalize the stream, commit remaining stable lines, return
    /// (final cell, raw markdown source for source-backed consolidation).
    pub fn finalize(self) -> (Option<AgentMarkdownCell>, String);
}
```

#### Table holdback

When a pipe-table header + separator pair is detected (e.g., `| col1 | col2 |
|---|---|`), all lines from the header onward are forced into the tail region
until the stream finalizes or the table is confirmed complete (non-table line
after at least one data row). This prevents "table being drawn row by row"
flicker.

```rust
// streaming/table_holdback.rs

pub struct TableHoldbackState {
    /// Whether we're currently in a table holdback.
    holding: bool,
    /// Source line index where the table header starts.
    table_start_line: usize,
    /// Whether at least one data row has been seen.
    has_data_row: bool,
}
```

#### Commit animation

Stable lines are drained from the queue with a frame rate-limited commit tick
that feeds one (or a small batch of) line(s) per frame into the scrollback:

```rust
// streaming/commit_tick.rs

pub struct CommitTicker {
    /// Lines waiting to be committed to scrollback.
    queue: VecDeque<(Line<'static>, Instant)>,
    /// Minimum delay before a line is eligible for commit.
    min_delay: Duration,
    /// Maximum lines to commit per tick.
    max_per_tick: usize,
}
```

#### Stream consolidation

When the stream finalizes, contiguous `AgentStreamingTailCell` instances are
collapsed into a single `AgentMarkdownCell` that stores the raw markdown
source and re-renders on terminal resize (rather than storing pre-wrapped
lines at a fixed width).

### Impact

- `streaming.rs:StreamController` (138 lines) — replaced by `StreamCore`
- `response.rs:text_delta` handler — routes deltas through `StreamCore::push_delta()`
- `response.rs:turn_done` handler — calls `StreamCore::finalize()`, sends `ConsolidateAgentMessage` event

## Affected Files (Summary)

| File | Action |
|------|--------|
| `tui/protocol.rs` | **New** — `ServerNotification` enum + deserialization |
| `tui/history_cell/mod.rs` | **New** — `HistoryCell` trait + type registry |
| `tui/history_cell/messages.rs` | **New** — `UserMessageCell`, `AgentMessageCell`, `AgentMarkdownCell`, `AgentStreamingTailCell` |
| `tui/history_cell/exec.rs` | **New** — `ExecCell` (absorbs `toolcard.rs`) |
| `tui/history_cell/plans.rs` | **New** — `PlanCell` |
| `tui/history_cell/approvals.rs` | **New** — `ApprovalCell` (absorbs `approval_dialog.rs`) |
| `tui/history_cell/status.rs` | **New** — `StatusEventCell` |
| `tui/chatwidget.rs` | **New** — `ChatWidget` struct, cell management, `as_renderable()` |
| `tui/chatwidget/rendering.rs` | **New** — Render composition |
| `tui/chatwidget/streaming.rs` | **New** — Stream lifecycle methods |
| `tui/chatwidget/turn_lifecycle.rs` | **New** — Turn start/done state machine |
| `tui/chatwidget/status_state.rs` | **New** — Status bar state management |
| `tui/render/renderable.rs` | **New** — `Renderable` trait + `FlexRenderable` etc. |
| `tui/render/draw.rs` | **Modify** — Simplify to `as_renderable().render()` |
| `tui/streaming/controller.rs` | **Rewrite** — `StreamCore` |
| `tui/streaming/chunking.rs` | **New** — Adaptive chunking (newline-gated) |
| `tui/streaming/commit_tick.rs` | **New** — Commit animation ticker |
| `tui/streaming/table_holdback.rs` | **New** — Table holdback detection |
| `tui/markdown_render.rs` | **Modify** — Table auto-width + fence unwrap + syntax highlight |
| `tui/response.rs` | **Delete** — Logic split into protocol + chatwidget/ |
| `tui/chat.rs` | **Replace** — `ChatMessage` → `HistoryCell` cells |
| `tui/toolcard.rs` | **Delete** — Absorbed by `ExecCell` |
| `tui/approval_dialog.rs` | **Delete** — Absorbed by `ApprovalCell` |
| `tui/cli.rs` | **Reduce** — Remove duplicate format_* functions, keep CLI dispatch only |
| `tui/streaming.rs` | **Replace** — Simple controller → `StreamCore` |
| `tui/mod.rs` | **Modify** — Update module declarations |
| `tui/app/submit.rs` | **Modify** — Use `ServerNotification` types instead of raw JSON |
| `tui/app/lifecycle.rs` | **Modify** — Route through `protocol.rs` |
| `tui/app/key_handler.rs` | **Modify** — Use typed notification to dispatch |

## Files NOT Changed

- `tui/markdown.rs` (422 lines) — pulldown-cmark parser, used by `markdown_render.rs`
- `tui/term_compat.rs` (347 lines) — terminal capability detection, `Theme`, `TermCaps`
- `tui/state.rs` (130 lines) — `AppState` struct, used by status bar
- `tui/render/header.rs`, `tui/render/input_line.rs` — internal render helpers
- `tui/status.rs` (329 lines) — status bar widget (may be simplified, not rewritten)
- `tui/pager.rs` (164 lines) — pager overlay
- `tui/command.rs`, `tui/completion.rs`, `tui/input.rs`, `tui/history_search.rs` — input system
- `tui/skill.rs`, `tui/workflow.rs`, `tui/goal.rs`, `tui/plan_view.rs` — sub-features
- `tui/debug.rs` (1,255 lines) — debug subcommands
- `tui/test_infra.rs` — test support
- `crates/aletheon/src/main.rs` — unchanged
- `crates/runtime/src/impl/daemon/handler/format.rs` — daemon-side event serialization unchanged
- All files under `crates/interact/src/acix/` — unchanged

## Testing Strategy

1. **Unit tests**: `protocol.rs` — round-trip deserialization of every `ServerNotification` variant from sample JSON
2. **Cell rendering tests**: Each `HistoryCell` impl — snapshot test at 80/120/200 column widths
3. **Stream core tests**: `StreamCore` — delta push + table holdback + finalize scenarios
4. **Integration tests**: Existing `test_infra.rs` frame recording infrastructure — capture before/after frames, compare
5. **Smoke test**: `aletheon exec` and `aletheon` TUI modes both launch and display agent output

## Risks

| Risk | Mitigation |
|------|-----------|
| Layout regressions (terminal size edge cases) | Start with exact `FlexRenderable` layout matching current `draw.rs` constraints; add complexity incrementally |
| Cell memory leaks (not clearing on turn_done) | `ChatWidget::clear_active_stream()` called in `turn_lifecycle` handler |
| Table holdback false positives (non-table pipe lines) | Only trigger on header + separator pair (exact `|---|---|` pattern) |
| Markdown re-render performance on resize | `AgentMarkdownCell` only re-renders on resize event, not every frame |

## Implementation Phases

1. **Phase 1: Protocol boundary** — `ServerNotification` enum + `protocol.rs` (no display changes yet)
2. **Phase 2: Renderable layout** — `Renderable` trait + composites, refactor `draw.rs`
3. **Phase 3: HistoryCell + ChatWidget** — Replace `ChatMessage`/`ChatWidget`, introduce cell types
4. **Phase 4: Streaming** — `StreamCore` + commit animation + table holdback
5. **Phase 5: Polish** — Markdown table rendering, syntax highlight, fence unwrap, shimmer
6. **Phase 6: Cleanup** — Delete `response.rs`, `toolcard.rs`, `approval_dialog.rs`; deduplicate `cli.rs`
