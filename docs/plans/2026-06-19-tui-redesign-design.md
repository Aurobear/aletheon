# Aletheon TUI Redesign — Design Document

**Date:** 2026-06-19
**Status:** Approved
**References:** DeepSeek-Reasonix, Codex CLI, Claude Code, OpenCode

## Problem Statement

The current aletheon TUI has three critical issues:

1. **No intermediate visibility** — Users cannot see the model's thinking process, tool calls, or streaming output. The daemon returns a complete JSON response after the entire ReAct loop finishes, so the TUI shows only a spinner during the wait.

2. **Hanging/freezing** — The daemon's `process_react` is synchronous and blocking. During model inference, the entire handler is occupied. If the model call takes long or hangs, the TUI appears frozen with no feedback.

3. **No multi-agent display** — When sub-agents are spawned (e.g., code-agent, fs-agent), there is no way to see their intermediate steps, thinking, or tool calls in the TUI.

## Design Goals

- **Full thinking chain display** — Show the model's reasoning process in real-time, with collapsible blocks and timeline
- **Streaming output** — Token-by-token display of model responses as they arrive
- **Tool call cards** — Visual cards showing tool name, arguments, status, and collapsible output
- **Multi-agent support** — Sub-agent outputs displayed as folded trees within the main chat flow
- **Non-blocking** — TUI never freezes; all daemon communication is async
- **Backward compatible** — Existing JSON-RPC protocol still works for test scripts and socat clients

## Architecture Overview

```
┌─────────────────────────────────────────────────────┐
│                     TUI (aletheon)                   │
│                                                      │
│  ┌──────────┐  ┌──────────────┐  ┌───────────────┐  │
│  │ ChatWidget│  │StreamController│  │ InputArea    │  │
│  │ (scroll)  │  │ (stable+tail) │  │ (history+tab)│  │
│  └──────────┘  └──────────────┘  └───────────────┘  │
│        ▲              ▲                               │
│        │              │                               │
│  ┌─────┴──────────────┴─────┐                        │
│  │    Event Dispatcher      │                        │
│  │  (thinking/text/tool/...)│                        │
│  └──────────────────────────┘                        │
│        ▲                                             │
│        │ Unix Socket (JSONL events + JSON-RPC)       │
├────────┼─────────────────────────────────────────────┤
│        │           Daemon (aletheond)                 │
│  ┌─────┴──────────────┐                              │
│  │  Request Handler   │                              │
│  │  ┌───────────────┐ │                              │
│  │  │ EventBus      │ │  ← publishes events          │
│  │  └───────────────┘ │                              │
│  │  ┌───────────────┐ │                              │
│  │  │ ReAct Loop    │ │  ← streaming model calls     │
│  │  └───────────────┘ │                              │
│  └────────────────────┘                              │
└──────────────────────────────────────────────────────┘
```

---

## Phase 1: Streaming + Thinking + Fix Hanging

### 1.1 Daemon Event System

**New event types** (published via EventBus, serialized as JSONL over socket):

| Event | Fields | Description |
|-------|--------|-------------|
| `turn_start` | `turn` | New turn begins |
| `thinking_start` | — | Model begins reasoning |
| `thinking_delta` | `text` | Incremental thinking text |
| `thinking_end` | `duration_ms` | Thinking complete, with elapsed time |
| `text_start` | — | Model begins output |
| `text_delta` | `text` | Incremental response text |
| `tool_call_start` | `call_id`, `tool`, `args` | Tool invocation begins |
| `tool_call_progress` | `call_id`, `output` | Streaming tool output |
| `tool_call_result` | `call_id`, `output`, `exit_code` | Tool invocation complete |
| `turn_done` | `turn`, `summary` | Turn complete |
| `error` | `code`, `message` | Error occurred |

**Wire format** (JSONL over existing Unix socket):

```jsonl
{"jsonrpc":"2.0","method":"event","params":{"type":"thinking_delta","text":"Let me analyze..."}}
{"jsonrpc":"2.0","method":"event","params":{"type":"tool_call_start","call_id":"tc_1","tool":"bash","args":"grep -r foo ."}}
{"jsonrpc":"2.0","id":1,"result":{"response":"Hello!","turn":5}}
```

Events use `method: "event"` (no `id`) to distinguish from JSON-RPC responses. Existing clients that only look for `id`/`result` fields will ignore events.

### 1.2 Streaming Model Calls

**Current flow** (blocking):
```
handler receives "chat" → builds prompt → calls model (blocking) → waits for full response → returns JSON-RPC result
```

**New flow** (streaming):
```
handler receives "chat" → builds prompt → spawns streaming task:
  → emits turn_start
  → streams thinking chunks → emits thinking_delta events
  → emits thinking_end
  → streams text chunks → emits text_delta events
  → on tool call → emits tool_call_start → executes tool → emits tool_call_result
  → loops until done
  → emits turn_done
  → sends final JSON-RPC result (backward compat)
```

The handler spawns `process_react` as a background task. The connection handler concurrently:
1. Forwards events from EventBus to the socket as JSONL
2. Pumps approval requests (existing behavior)
3. Waits for the final result

### 1.3 TUI Event Dispatch

The TUI's main loop changes from:

```rust
// OLD: poll socket only when streaming
if app.streaming { try_read_response(&mut app); }
```

To:

```rust
// NEW: always poll socket (events can arrive anytime)
try_read_socket(&mut app);
```

`try_read_socket` reads JSONL lines and dispatches:
- `method: "event"` → `handle_event(app, params)`
- `method: "approval_request"` → show approval dialog (existing)
- Has `result` or `error` → `process_response(app, msg)` (existing, backward compat)

### 1.4 StreamController

Two-region streaming model (inspired by Codex):

- **Stable region**: Committed content that has been fully received. This enters the scrollback history and is immutable.
- **Tail region**: The currently-streaming content. Updated in-place on each `text_delta` or `thinking_delta` event.

```rust
struct StreamController {
    /// Committed lines (stable, in scrollback)
    committed: Vec<Line>,
    /// Current tail content (mutable, being streamed)
    tail: String,
    /// Whether we're in thinking phase
    thinking: bool,
    /// Thinking start time
    thinking_start: Option<Instant>,
    /// Thinking collapsed state
    thinking_collapsed: bool,
}
```

When `thinking_end` arrives:
- If collapsed (default): replace tail with `"✻ Thought for N.Ns"`
- If expanded (Ctrl+O): keep full thinking text in a bordered block

When `text_start` arrives:
- Commit any thinking summary to stable region
- Start new tail for the response text

When `text_delta` arrives:
- Append to tail, re-render the active cell

When `turn_done` arrives:
- Commit tail to stable region
- Clear tail

### 1.5 ThinkingBlock

Collapsible thinking display:

```
Collapsed (default):
  ✻ Thought for 2.3s

Expanded (Ctrl+O):
  ✻ Thinking...
  │ Let me analyze the user's request. They want to...
  │ First, I need to understand the context...
  │ The key insight is that...
  │ (2.3s)
```

- Uses bounded trailing window (4KB max) to avoid O(n²) rendering
- Only the last 12 visual lines are rendered in the expanded view
- Full text preserved in memory for `/reflect` and archival

### 1.6 ToolCard

Tool call display:

```
  ⏺ Bash(grep -r "foo" .)                    ← header (cyan dot for read)
  │ file1.rs:42: foo bar                      ← output (collapsed to 3 lines)
  │ file2.rs:18: foo baz
  │ (3 lines, Ctrl+B to expand)              ← collapse hint

  ⏺ Write(src/main.rs)                       ← header (green dot for write)
  │ ✓ wrote 142 lines                        ← result summary

  ⏺ Bash(sleep 10)                           ← header (yellow dot for exec)
  │ ⠋ working... 3.2s                        ← spinner while running
```

- Category-colored dots: cyan=read, green=write, yellow=exec, magenta=proc
- Output defaults to collapsed (first 3 lines + "N lines" summary)
- Ctrl+B toggles expand/collapse of the most recent tool output
- Max expanded lines: 200 (cap for very long outputs)

### 1.7 Spinner Animation

Fix the non-animating spinner:

- Call `tick_spinner()` in the main loop before each `draw()`
- 8-frame braille animation at 60ms intervals during streaming
- Status bar shows: `⠋ thinking... │ 2.3s │ 156 tok`
- Idle: no spinner, show connection status

### 1.8 Input Enhancements

- **Command history**: Up/Down arrows navigate last 50 commands
- **Tab completion**: `/ref<Tab>` → shows `/reflect`, `/reflect_now`, `/resume`
  - Floating popup with matches
  - Up/Down to select, Enter to confirm, Esc to cancel
  - Single match: direct completion
- **Ctrl+O**: Toggle thinking block expand/collapse
- **Ctrl+B**: Toggle tool output expand/collapse
- **Ctrl+C**: Clear input (double-press to quit)
- **Ctrl+D**: Quit (when input empty)
- **Ctrl+L**: Clear chat

---

## Phase 2: Multi-Agent Folded Tree (High-Level)

### 2.1 Daemon Events

New events for sub-agent lifecycle:

| Event | Fields | Description |
|-------|--------|-------------|
| `subagent_start` | `agent_id`, `task` | Sub-agent spawned |
| `subagent_delta` | `agent_id`, `type`, `text` | Sub-agent thinking/text/tool event |
| `subagent_done` | `agent_id`, `status`, `summary` | Sub-agent completed |

### 2.2 TUI Display

Sub-agent output rendered as folded blocks within the main chat flow:

```
  │ main agent response text...

  ⏺ Agent(code-agent): "Find all Rust files"     ← sub-agent card
  │ ├── thinking: analyzed directory structure...  ← folded by default
  │ ├── tool: glob("**/*.rs") → 42 files
  │ └── completed in 3.2s
  │ (Ctrl+B to expand)

  │ main agent continues...
```

- Sub-agent cards use the same folding mechanism as tool cards
- Expanded view shows the full sub-agent conversation (thinking + tools + response)
- Max sub-agents displayed: configurable (default 10)

---

## Phase 3: Context Caching (High-Level)

### 3.1 Cache-Stable Prefix

- System prompt + tool definitions + memory prefix: byte-stable across turns
- Memory changes appended as tail notes, not injected into prefix
- Reasoning content never re-uploaded (cost optimization)

### 3.2 Token Budget

- Track token usage per turn
- Inject `<token_budget>` info into context
- Auto-trigger `/compact` when approaching model context limit

### 3.3 Model Integration

- Support DeepSeek prefix cache (system prompt unchanged → cache hit)
- Support OpenAI automatic prefix caching
- Cache-stable prefix length logged for debugging

---

## File Changes Summary (Phase 1)

| File | Change |
|------|--------|
| `crates/aletheon-runtime/src/impl/daemon/handler.rs` | Add event emission, streaming model calls |
| `crates/aletheon-body/src/impl/ui/mod.rs` | Event dispatch, StreamController, ThinkingBlock, ToolCard |
| `crates/aletheon-body/src/impl/ui/chat.rs` | ChatWidget: active-cell, folding, streaming updates |
| `crates/aletheon-body/src/impl/ui/streaming.rs` | NEW: StreamController implementation |
| `crates/aletheon-body/src/impl/ui/thinking.rs` | NEW: ThinkingBlock widget |
| `crates/aletheon-body/src/impl/ui/toolcard.rs` | NEW: ToolCard widget |
| `crates/aletheon-body/src/impl/ui/completion.rs` | NEW: Tab completion popup |
| `crates/aletheon-body/src/impl/ui/status.rs` | Fix spinner animation, add token count |
| `crates/aletheon-body/src/impl/ui/input.rs` | Command history, integration with main loop |
| `crates/aletheon-body/src/impl/ui/term_compat.rs` | Theme additions for thinking/tool colors |

## Success Criteria (Phase 1)

- [ ] User sees "✻ Thinking... (N.Ns)" while model is reasoning
- [ ] User sees streaming text as model generates response
- [ ] Tool calls show as cards with name, args, and collapsible output
- [ ] Spinner animates smoothly during all waiting states
- [ ] Tab completion works for all slash commands
- [ ] Up/Down arrows navigate command history
- [ ] TUI never freezes/hangs during model inference
- [ ] Existing socat/test scripts still work (backward compatible)
- [ ] Ctrl+O toggles thinking block, Ctrl+B toggles tool output
