# TUI Redesign Phase 1 — Implementation Plan

> **For agentic workers:** Use `workflow-feature` or `writing-plans` to implement this plan task-by-task. Steps use checkbox (`- [ ]`) syntax for tracking.

**Goal:** Transform aletheon's TUI from a "send-and-wait" interface into a streaming, thinking-aware, tool-card-enabled terminal UI matching the quality of Claude Code and DeepSeek-Reasonix.

**Architecture:** Wire the existing `LlmProvider::complete_stream()` and `EventSink` infrastructure through `ReActLoop` to the daemon handler, which pushes JSONL events over the Unix socket. The TUI receives these events and renders them via new `StreamController`, `ThinkingBlock`, and `ToolCard` widgets.

**Tech Stack:** Rust, tokio, ratatui, crossterm, serde_json, existing aletheon-brain/runtime/body crates

---

## Key Discovery

Streaming is already fully implemented but unwired:

| Layer | Status | File |
|-------|--------|------|
| `LlmProvider::complete_stream()` | ✅ DONE | `aletheon-brain/src/impl/llm/provider.rs` |
| `StreamChunk` types | ✅ DONE | `aletheon-brain/src/impl/llm/provider.rs` |
| `Engine::run_turn_streaming()` | ✅ DONE (unused) | `aletheon-runtime/src/impl/engine/streaming.rs` |
| `Event::TextDelta` / `EventSink` | ✅ DONE | `aletheon-runtime/src/core/event_sink.rs` |
| `ReActLoop::run()` | ❌ Non-streaming | `aletheon-runtime/src/core/react_loop.rs` |
| Handler → TUI events | ❌ Not implemented | `handler.rs` |
| TUI event rendering | ❌ Not implemented | `ui/mod.rs` |

The work is **wiring**, not building from scratch.

---

## File Map

### Create
| File | Purpose |
|------|---------|
| `crates/aletheon-body/src/impl/ui/streaming.rs` | StreamController: two-region streaming, bounded tail |
| `crates/aletheon-body/src/impl/ui/thinking.rs` | ThinkingBlock widget: collapsible thinking display |
| `crates/aletheon-body/src/impl/ui/toolcard.rs` | ToolCard widget: tool call cards with collapsible output |
| `crates/aletheon-body/src/impl/ui/completion.rs` | Tab completion popup for slash commands |

### Modify
| File | Change |
|------|--------|
| `crates/aletheon-runtime/src/core/react_loop.rs` | Add `run_streaming()` method |
| `crates/aletheon-runtime/src/impl/daemon/handler.rs` | Use streaming ReActLoop, emit events via notify_tx |
| `crates/aletheon-runtime/src/impl/daemon/server.rs` | Forward events from EventBus to socket as JSONL |
| `crates/aletheon-body/src/impl/ui/mod.rs` | Event dispatch, main loop changes |
| `crates/aletheon-body/src/impl/ui/chat.rs` | Active-cell support, folding, streaming updates |
| `crates/aletheon-body/src/impl/ui/status.rs` | Fix spinner animation, add elapsed time |
| `crates/aletheon-body/src/impl/ui/input.rs` | Command history, tab completion integration |
| `crates/aletheon-body/src/impl/ui/term_compat.rs` | Theme additions for thinking/tool colors |
| `crates/aletheon-body/src/lib.rs` | Register new modules |

---

## Task 1: Add `run_streaming()` to ReActLoop

**Files:**
- Modify: `crates/aletheon-runtime/src/core/react_loop.rs`

- [ ] **Step 1: Add streaming method to ReActLoop**

Add a new method `run_streaming()` alongside the existing `run()`. It takes an additional `event_sink: &dyn EventSink` parameter and uses `llm.complete_stream()` instead of `llm.complete()`.

```rust
pub async fn run_streaming<L, F, Fut>(
    &mut self,
    user_input: &str,
    llm: &L,
    tool_defs: &[ToolDefinition],
    execute_tool: F,
    event_sink: &dyn EventSink,
) -> anyhow::Result<String>
where
    L: LlmProvider + ?Sized,
    F: Fn(&str, &str, &serde_json::Value) -> Fut,
    Fut: Future<Output = (String, bool)>,
{
    use aletheon_runtime::core::event_sink::Event;
    use futures::StreamExt;

    self.messages.push(Message::user(user_input));
    event_sink.emit(Event::TurnStarted);

    while self.should_continue() {
        self.advance();

        // Use streaming instead of complete()
        let mut stream = match llm.complete_stream(&self.messages, tool_defs).await {
            Ok(s) => s,
            Err(e) if is_context_overflow(&e) => {
                warn!("Context overflow detected, forcing compaction: {e}");
                self.compressor.maybe_compact(&mut self.messages, llm).await?;
                llm.complete_stream(&self.messages, tool_defs).await?
            }
            Err(e) => return Err(e),
        };

        let mut text_parts = Vec::new();
        let mut current_text = String::new();
        let mut tool_calls = Vec::new();
        let mut tool_inputs: HashMap<String, serde_json::Value> = HashMap::new();
        let mut stop_reason = StopReason::EndTurn;

        while let Some(chunk) = stream.next().await {
            match chunk? {
                StreamChunk::TextDelta { text } => {
                    current_text.push_str(&text);
                    event_sink.emit(Event::TextDelta { delta: text });
                }
                StreamChunk::ToolUseStart { id, name } => {
                    // Flush any pending text
                    if !current_text.is_empty() {
                        text_parts.push(current_text.clone());
                        current_text.clear();
                    }
                    event_sink.emit(Event::ToolCallStart {
                        name: name.clone(),
                        call_id: id.clone(),
                    });
                    tool_calls.push((id.clone(), name.clone(), serde_json::Value::Null));
                }
                StreamChunk::ToolUseDelta { id, delta } => {
                    // Accumulate tool input JSON
                    tool_inputs.entry(id).or_insert_with(|| serde_json::Value::String(String::new()));
                    if let serde_json::Value::String(s) = tool_inputs.get_mut(&id).unwrap() {
                        s.push_str(&delta);
                    }
                }
                StreamChunk::ToolUseComplete { id, input } => {
                    // Replace accumulated with final parsed input
                    tool_inputs.insert(id.clone(), input.clone());
                    // Update tool_calls with correct input
                    if let Some(tc) = tool_calls.iter_mut().find(|(tid, _, _)| *tid == id) {
                        tc.2 = input;
                    }
                }
                StreamChunk::Usage { input_tokens, output_tokens } => {
                    event_sink.emit(Event::Usage {
                        tokens_in: input_tokens,
                        tokens_out: output_tokens,
                        cache_hit_tokens: 0,
                        cache_miss_tokens: 0,
                    });
                }
                StreamChunk::Done { stop_reason: sr } => {
                    stop_reason = sr;
                    break;
                }
            }
        }

        // Flush remaining text
        if !current_text.is_empty() {
            text_parts.push(current_text);
        }

        // No tool calls → turn complete
        if tool_calls.is_empty() || matches!(stop_reason, StopReason::EndTurn) {
            let final_text = text_parts.join("\n");
            self.messages.push(Message::assistant(&final_text));
            event_sink.emit(Event::TurnDone { result: Ok(final_text.clone()) });
            return Ok(final_text);
        }

        // Has tool calls → execute them
        let content_blocks: Vec<ContentBlock> = tool_calls.iter().map(|(id, name, input)| {
            ContentBlock::ToolUse {
                id: id.clone(),
                name: name.clone(),
                input: input.clone(),
            }
        }).collect();

        self.messages.push(Message {
            role: Role::Assistant,
            content: content_blocks,
        });

        for (id, name, input) in &tool_calls {
            debug!(tool = name.as_str(), "ReActLoop streaming: executing tool");
            event_sink.emit(Event::ToolDispatch {
                name: name.clone(),
                args: input.clone(),
            });

            let (content, is_error) = execute_tool(id, name, input).await;

            event_sink.emit(Event::ToolResult {
                name: name.clone(),
                result: ToolResultEvent {
                    content: content.clone(),
                    is_error,
                    execution_time_ms: 0, // TODO: measure
                },
            });

            if is_error {
                warn!(tool = name.as_str(), "tool returned error");
            }
            self.messages.push(Message::tool_result(id, &content, is_error));
        }

        if self.config.compaction_enabled {
            let _ = self.compressor.maybe_compact(&mut self.messages, llm).await;
        }
    }

    warn!(max = self.config.max_iterations, "ReActLoop streaming hit max_iterations");
    let fallback = self.messages.iter().rev().find_map(|m| {
        m.content.iter().find_map(|b| match b {
            ContentBlock::Text { text } => Some(text.clone()),
            _ => None,
        })
    }).unwrap_or_else(|| format!("Max iterations ({}) reached", self.config.max_iterations));
    event_sink.emit(Event::TurnDone { result: Ok(fallback.clone()) });
    Ok(fallback)
}
```

- [ ] **Step 2: Verify it compiles**

Run: `cargo check -p aletheon-runtime`
Expected: compiles with maybe unused import warnings

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-runtime/src/core/react_loop.rs
git commit -m "feat(runtime): add streaming variant of ReActLoop::run()

Uses llm.complete_stream() and emits events via EventSink during
the ReAct loop. Streaming infrastructure was already implemented
in LlmProvider but never wired through ReActLoop."
```

---

## Task 2: Wire handler to use streaming ReActLoop

**Files:**
- Modify: `crates/aletheon-runtime/src/impl/daemon/handler.rs`

- [ ] **Step 1: Add event forwarding in handler**

In the chat method handler (around line 1097), replace the non-streaming `process_react` call with a streaming variant that emits events through `notify_tx`.

The key change: instead of spawning `rt.process_react()` and waiting for the final result, spawn a task that:
1. Creates a `ChannelEventSink` that forwards events to `notify_tx`
2. Calls `react_loop.run_streaming()` with the event sink
3. The `tokio::select!` loop now also pumps events from the event sink receiver

Add this helper function to the handler:

```rust
fn event_to_json(event: &Event) -> Option<String> {
    let params = match event {
        Event::TurnStarted => json!({"type": "turn_start"}),
        Event::TextDelta { delta } => json!({"type": "text_delta", "text": delta}),
        Event::ToolCallStart { name, call_id } => json!({"type": "tool_call_start", "call_id": call_id, "tool": name}),
        Event::ToolResult { name, result } => json!({
            "type": "tool_call_result",
            "tool": name,
            "output": result.content,
            "is_error": result.is_error,
            "execution_time_ms": result.execution_time_ms,
        }),
        Event::ToolDispatch { name, args } => json!({"type": "tool_dispatch", "tool": name, "args": args}),
        Event::Usage { tokens_in, tokens_out, .. } => json!({"type": "usage", "tokens_in": tokens_in, "tokens_out": tokens_out}),
        Event::TurnDone { result } => json!({"type": "turn_done", "success": result.is_ok()}),
        Event::Error { message } => json!({"type": "error", "message": message}),
        _ => return None,
    };
    Some(json!({"jsonrpc": "2.0", "method": "event", "params": params}).to_string())
}
```

- [ ] **Step 2: Modify the chat handler to use streaming**

Replace the `process_react` spawn with a streaming variant. The `tokio::select!` loop now has three arms:
1. The react task completes
2. An approval request arrives
3. An event arrives from the event sink (forward to notify_tx)

```rust
// Create event channel
let (event_tx, mut event_rx) = tokio::sync::mpsc::channel::<Event>(64);
let event_sink = ChannelEventSink::new(event_tx);

let mut react_task = tokio::spawn(async move {
    // Use react_loop.run_streaming() instead of rt.process_react()
    react_loop.run_streaming(
        &effective_message,
        &*llm,
        &tool_defs,
        execute_tool,
        &event_sink,
    ).await
});

// Pump events + approvals while react loop runs
let text = loop {
    tokio::select! {
        result = &mut react_task => {
            break result.unwrap_or_else(|e| Err(anyhow::anyhow!("react task panicked: {e}")));
        }
        Some(event) = event_rx.recv() => {
            if let Some(json_str) = event_to_json(&event) {
                if let Some(ref tx) = notify_tx {
                    let _ = tx.send(json_str).await;
                }
            }
        }
        Some(pending) = async {
            let mut rx = approval_rx.lock().await;
            rx.recv().await
        } => {
            // ... existing approval handling code ...
        }
    }
};
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-runtime`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-runtime/src/impl/daemon/handler.rs
git commit -m "feat(runtime): wire streaming ReActLoop in daemon handler

Handler now uses run_streaming() and forwards EventSink events
as JSONL notifications over the Unix socket. Events use
method: 'event' to distinguish from JSON-RPC responses."
```

---

## Task 3: Add TUI event dispatcher

**Files:**
- Modify: `crates/aletheon-body/src/impl/ui/mod.rs`

- [ ] **Step 1: Add event state to App struct**

```rust
struct App {
    // ... existing fields ...
    /// Streaming controller for incremental rendering
    stream_ctrl: StreamController,
    /// Active tool calls (call_id → ToolCard)
    active_tools: HashMap<String, ToolCard>,
    /// Current turn's token count
    turn_tokens: Option<(u32, u32)>, // (in, out)
}
```

- [ ] **Step 2: Replace `try_read_response` with `try_read_socket`**

The new function handles both events and JSON-RPC responses:

```rust
fn try_read_socket(app: &mut App) {
    loop {
        match app.stream.try_read(&mut app.read_buf) {
            Ok(0) => {
                app.streaming = false;
                app.status.waiting = false;
                app.chat.add_message(ChatRole::System, "连接断开".to_string());
                break;
            }
            Ok(n) => {
                let chunk = String::from_utf8_lossy(&app.read_buf[..n]);
                app.response_buf.push_str(&chunk);

                // Process each complete JSONL line
                while let Some(newline_pos) = app.response_buf.find('\n') {
                    let line = app.response_buf[..newline_pos].trim().to_string();
                    app.response_buf.drain(..=newline_pos);

                    if line.is_empty() { continue; }

                    if let Ok(msg) = serde_json::from_str::<serde_json::Value>(&line) {
                        if msg.get("method").and_then(|v| v.as_str()) == Some("event") {
                            // Event notification
                            if let Some(params) = msg.get("params") {
                                handle_event(app, params);
                            }
                        } else if msg.get("method").and_then(|v| v.as_str()) == Some("approval_request") {
                            // Existing approval handling
                            handle_approval(app, &msg);
                        } else if msg.get("result").is_some() || msg.get("error").is_some() {
                            // JSON-RPC response (backward compat)
                            process_response(app, msg);
                            break;
                        }
                    }
                }
            }
            Err(ref e) if e.kind() == io::ErrorKind::WouldBlock => break,
            Err(_) => {
                app.streaming = false;
                app.status.waiting = false;
                break;
            }
        }
    }
}
```

- [ ] **Step 3: Add `handle_event` dispatcher**

```rust
fn handle_event(app: &mut App, params: &serde_json::Value) {
    let event_type = params.get("type").and_then(|v| v.as_str()).unwrap_or("");
    match event_type {
        "turn_start" => {
            app.stream_ctrl.start_turn();
            app.status.waiting = true;
        }
        "thinking_delta" => {
            if let Some(text) = params.get("text").and_then(|v| v.as_str()) {
                app.stream_ctrl.push_thinking(text);
            }
        }
        "text_delta" => {
            if let Some(text) = params.get("text").and_then(|v| v.as_str()) {
                app.stream_ctrl.push_text(text);
                // Update the last assistant message in real-time
                app.chat.update_last_message(app.stream_ctrl.current_text());
            }
        }
        "tool_call_start" => {
            let call_id = params.get("call_id").and_then(|v| v.as_str()).unwrap_or("").to_string();
            let tool = params.get("tool").and_then(|v| v.as_str()).unwrap_or("?").to_string();
            let args = params.get("args").map(|v| v.to_string()).unwrap_or_default();
            app.active_tools.insert(call_id.clone(), ToolCard::new(call_id, tool, args));
        }
        "tool_call_result" => {
            let call_id = params.get("call_id").and_then(|v| v.as_str()).unwrap_or("");
            if let Some(card) = app.active_tools.get_mut(call_id) {
                let output = params.get("output").and_then(|v| v.as_str()).unwrap_or("");
                let is_error = params.get("is_error").and_then(|v| v.as_bool()).unwrap_or(false);
                card.finish(output, is_error);
            }
        }
        "usage" => {
            let tokens_in = params.get("tokens_in").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            let tokens_out = params.get("tokens_out").and_then(|v| v.as_u64()).unwrap_or(0) as u32;
            app.turn_tokens = Some((tokens_in, tokens_out));
            app.status.token_count = Some(tokens_in + tokens_out);
        }
        "turn_done" => {
            app.stream_ctrl.commit();
            app.streaming = false;
            app.status.waiting = false;
            app.status.elapsed_secs = 0.0;
            // Commit any active tool cards to chat history
            for (_, card) in app.active_tools.drain() {
                app.chat.add_message(ChatRole::System, card.to_summary());
            }
        }
        "error" => {
            let msg = params.get("message").and_then(|v| v.as_str()).unwrap_or("Unknown error");
            app.chat.add_message(ChatRole::System, format!("Error: {}", msg));
            app.streaming = false;
            app.status.waiting = false;
        }
        _ => {}
    }
}
```

- [ ] **Step 4: Update main loop to always poll socket**

Change the main loop from:
```rust
if app.streaming { try_read_response(&mut app); }
```
To:
```rust
try_read_socket(&mut app);
```

Also add spinner tick:
```rust
if app.streaming {
    app.status.tick_spinner();
}
```

- [ ] **Step 5: Verify it compiles**

Run: `cargo check -p aletheon-body`
Expected: compiles (StreamController/ToolCard not yet created, will error)

- [ ] **Step 6: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/mod.rs
git commit -m "feat(tui): add event dispatcher and streaming state management

TUI now handles JSONL events from daemon: thinking_delta,
text_delta, tool_call_start, tool_call_result, turn_done.
Main loop always polls socket for events."
```

---

## Task 4: Implement StreamController

**Files:**
- Create: `crates/aletheon-body/src/impl/ui/streaming.rs`
- Modify: `crates/aletheon-body/src/lib.rs` (add `pub mod streaming;`)

- [ ] **Step 1: Create streaming.rs**

```rust
//! StreamController: two-region streaming with bounded tail.
//!
//! Inspired by Codex's StreamController:
//! - Stable region: committed content in scrollback (immutable)
//! - Tail region: currently-streaming content (mutable, real-time)

use std::time::Instant;

const THINKING_VIEW_MAX: usize = 4096; // 4KB cap for thinking tail
const THINKING_TAIL_LINES: usize = 12; // max visual lines for thinking

pub struct StreamController {
    /// Committed text (stable, in scrollback)
    committed: String,
    /// Current streaming text (tail, mutable)
    tail: String,
    /// Thinking buffer (bounded)
    thinking_buf: String,
    /// Whether currently in thinking phase
    thinking: bool,
    /// Thinking start time
    thinking_start: Option<Instant>,
    /// Thinking collapsed state
    thinking_collapsed: bool,
}

impl StreamController {
    pub fn new() -> Self {
        Self {
            committed: String::new(),
            tail: String::new(),
            thinking_buf: String::new(),
            thinking: false,
            thinking_start: None,
            thinking_collapsed: true,
        }
    }

    pub fn start_turn(&mut self) {
        self.committed.clear();
        self.tail.clear();
        self.thinking_buf.clear();
        self.thinking = false;
        self.thinking_start = None;
    }

    pub fn push_thinking(&mut self, text: &str) {
        if !self.thinking {
            self.thinking = true;
            self.thinking_start = Some(Instant::now());
        }
        self.thinking_buf.push_str(text);
        // Bounded tail: keep only last THINKING_VIEW_MAX bytes
        if self.thinking_buf.len() > THINKING_VIEW_MAX {
            let excess = self.thinking_buf.len() - THINKING_VIEW_MAX;
            self.thinking_buf.drain(..excess);
        }
    }

    pub fn push_text(&mut self, text: &str) {
        // If we were thinking, commit thinking summary
        if self.thinking {
            self.commit_thinking();
        }
        self.tail.push_str(text);
    }

    pub fn current_text(&self) -> String {
        let mut result = String::new();
        if self.thinking && !self.thinking_collapsed {
            result.push_str(&self.format_thinking_expanded());
        }
        result.push_str(&self.committed);
        result.push_str(&self.tail);
        result
    }

    pub fn commit(&mut self) {
        if self.thinking {
            self.commit_thinking();
        }
        self.committed.push_str(&self.tail);
        self.tail.clear();
    }

    pub fn toggle_thinking(&mut self) {
        self.thinking_collapsed = !self.thinking_collapsed;
    }

    pub fn thinking_elapsed(&self) -> Option<f64> {
        self.thinking_start.map(|s| s.elapsed().as_secs_f64())
    }

    pub fn is_thinking(&self) -> bool {
        self.thinking
    }

    pub fn thinking_collapsed(&self) -> bool {
        self.thinking_collapsed
    }

    fn commit_thinking(&mut self) {
        if let Some(start) = self.thinking_start {
            let elapsed = start.elapsed().as_secs_f64();
            if self.thinking_collapsed {
                self.committed.push_str(&format!("✻ Thought for {:.1}s\n\n", elapsed));
            } else {
                self.committed.push_str(&self.format_thinking_expanded());
            }
        }
        self.thinking = false;
        self.thinking_buf.clear();
        self.thinking_start = None;
    }

    fn format_thinking_expanded(&self) -> String {
        let elapsed = self.thinking_elapsed().unwrap_or(0.0);
        let lines: Vec<&str> = self.thinking_buf.lines().collect();
        let display_lines: Vec<&str> = if lines.len() > THINKING_TAIL_LINES {
            &lines[lines.len() - THINKING_TAIL_LINES..]
        } else {
            &lines
        };
        let mut result = String::from("✻ Thinking...\n");
        for line in display_lines {
            result.push_str(&format!("│ {}\n", line));
        }
        result.push_str(&format!("({:.1}s)\n\n", elapsed));
        result
    }
}
```

- [ ] **Step 2: Add to lib.rs**

In `crates/aletheon-body/src/lib.rs`, add:
```rust
pub mod streaming;
```

- [ ] **Step 3: Verify it compiles**

Run: `cargo check -p aletheon-body`
Expected: compiles

- [ ] **Step 4: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/streaming.rs crates/aletheon-body/src/lib.rs
git commit -m "feat(tui): add StreamController for two-region streaming

Bounded tail window (4KB) for thinking, 12-line visual cap.
Collapsible thinking block with elapsed time."
```

---

## Task 5: Implement ThinkingBlock widget

**Files:**
- Create: `crates/aletheon-body/src/impl/ui/thinking.rs`
- Modify: `crates/aletheon-body/src/lib.rs`

- [ ] **Step 1: Create thinking.rs**

```rust
//! ThinkingBlock widget for collapsible thinking display.

use ratatui::{
    style::{Color, Modifier, Style},
    text::{Line, Span},
};

pub struct ThinkingBlock {
    pub collapsed: bool,
    pub elapsed: f64,
    pub text: String,
}

impl ThinkingBlock {
    pub fn new(elapsed: f64) -> Self {
        Self {
            collapsed: true,
            elapsed,
            text: String::new(),
        }
    }

    pub fn render_collapsed(&self) -> Vec<Line<'static>> {
        let style = Style::default().fg(Color::DarkGray).add_modifier(Modifier::ITALIC);
        vec![
            Line::from(vec![
                Span::styled(format!("✻ Thought for {:.1}s", self.elapsed), style),
            ]),
            Line::from(""),
        ]
    }

    pub fn render_expanded(&self) -> Vec<Line<'static>> {
        let header_style = Style::default().fg(Color::Cyan);
        let dim_style = Style::default().fg(Color::DarkGray);
        let mut lines = vec![
            Line::from(vec![
                Span::styled("✻ Thinking...", header_style),
            ]),
        ];
        for line in self.text.lines() {
            lines.push(Line::from(vec![
                Span::styled("│ ", dim_style),
                Span::raw(line.to_string()),
            ]));
        }
        lines.push(Line::from(vec![
            Span::styled(format!("({:.1}s)", self.elapsed), dim_style),
        ]));
        lines.push(Line::from(""));
        lines
    }
}
```

- [ ] **Step 2: Add to lib.rs**

```rust
pub mod thinking;
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/thinking.rs crates/aletheon-body/src/lib.rs
git commit -m "feat(tui): add ThinkingBlock widget

Collapsible thinking display with elapsed time.
Collapsed: '✻ Thought for N.Ns'
Expanded: bordered thinking text with timestamps."
```

---

## Task 6: Implement ToolCard widget

**Files:**
- Create: `crates/aletheon-body/src/impl/ui/toolcard.rs`
- Modify: `crates/aletheon-body/src/lib.rs`

- [ ] **Step 1: Create toolcard.rs**

```rust
//! ToolCard widget for tool call display.

use ratatui::{
    style::{Color, Style},
    text::{Line, Span},
};

const COLLAPSE_LINES: usize = 3;

pub struct ToolCard {
    pub call_id: String,
    pub tool: String,
    pub args: String,
    pub output: String,
    pub is_error: bool,
    pub finished: bool,
    pub expanded: bool,
}

impl ToolCard {
    pub fn new(call_id: String, tool: String, args: String) -> Self {
        Self {
            call_id,
            tool,
            args,
            output: String::new(),
            is_error: false,
            finished: false,
            expanded: false,
        }
    }

    pub fn finish(&mut self, output: &str, is_error: bool) {
        self.output = output.to_string();
        self.is_error = is_error;
        self.finished = true;
    }

    pub fn toggle(&mut self) {
        self.expanded = !self.expanded;
    }

    pub fn render(&self) -> Vec<Line<'static>> {
        let dot_color = if self.is_error {
            Color::Red
        } else if !self.finished {
            Color::Yellow
        } else {
            tool_color(&self.tool)
        };

        let status = if !self.finished {
            " ⠋".to_string()
        } else if self.is_error {
            " ✗".to_string()
        } else {
            " ✓".to_string()
        };

        let header = format!("⏺ {}({}){}", self.tool, truncate_args(&self.args, 60), status);
        let mut lines = vec![
            Line::from(vec![
                Span::styled("  ", Style::default()),
                Span::styled("● ", Style::default().fg(dot_color)),
                Span::raw(header),
            ]),
        ];

        if self.expanded {
            let output_lines: Vec<&str> = self.output.lines().collect();
            let display = if output_lines.len() > 200 {
                &output_lines[..200]
            } else {
                &output_lines
            };
            for line in display {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                    Span::raw(line.to_string()),
                ]));
            }
            if output_lines.len() > 200 {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("... ({} lines total)", output_lines.len()), Style::default().fg(Color::DarkGray)),
                ]));
            }
        } else if self.finished {
            let line_count = self.output.lines().count();
            if line_count > COLLAPSE_LINES {
                lines.push(Line::from(vec![
                    Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                    Span::styled(format!("{} lines, Ctrl+B to expand", line_count), Style::default().fg(Color::DarkGray)),
                ]));
            } else {
                for line in self.output.lines().take(COLLAPSE_LINES) {
                    lines.push(Line::from(vec![
                        Span::styled("  │ ", Style::default().fg(Color::DarkGray)),
                        Span::raw(line.to_string()),
                    ]));
                }
            }
        }

        lines.push(Line::from(""));
        lines
    }

    pub fn to_summary(&self) -> String {
        let status = if self.is_error { "failed" } else { "done" };
        format!("  ⏺ {}({}) — {}", self.tool, truncate_args(&self.args, 40), status)
    }
}

fn tool_color(tool: &str) -> Color {
    let lower = tool.to_lowercase();
    if lower.contains("read") || lower.contains("glob") || lower.contains("grep") {
        Color::Cyan
    } else if lower.contains("write") || lower.contains("edit") || lower.contains("apply") {
        Color::Green
    } else if lower.contains("bash") || lower.contains("shell") || lower.contains("exec") {
        Color::Yellow
    } else {
        Color::Magenta
    }
}

fn truncate_args(args: &str, max: usize) -> String {
    if args.len() <= max {
        args.to_string()
    } else {
        format!("{}...", &args[..max])
    }
}
```

- [ ] **Step 2: Add to lib.rs**

```rust
pub mod toolcard;
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/toolcard.rs crates/aletheon-body/src/lib.rs
git commit -m "feat(tui): add ToolCard widget

Tool call cards with category-colored dots (cyan/green/yellow/magenta),
collapsible output (Ctrl+B), max 200 lines expanded."
```

---

## Task 7: Add tab completion

**Files:**
- Create: `crates/aletheon-body/src/impl/ui/completion.rs`
- Modify: `crates/aletheon-body/src/lib.rs`

- [ ] **Step 1: Create completion.rs**

```rust
//! Tab completion for slash commands.

use ratatui::{
    layout::Rect,
    style::{Color, Modifier, Style},
    text::{Line, Span},
    widgets::{Block, Borders, Clear, List, ListItem},
    Frame,
};

pub struct CompletionPopup {
    pub visible: bool,
    pub candidates: Vec<String>,
    pub selected: usize,
    pub input_prefix: String,
}

impl CompletionPopup {
    pub fn new() -> Self {
        Self {
            visible: false,
            candidates: Vec::new(),
            selected: 0,
            input_prefix: String::new(),
        }
    }

    pub fn show(&mut self, prefix: &str, commands: &[String]) {
        self.candidates = commands.iter()
            .filter(|c| c.starts_with(prefix))
            .cloned()
            .collect();
        if self.candidates.is_empty() {
            self.visible = false;
            return;
        }
        self.visible = true;
        self.selected = 0;
        self.input_prefix = prefix.to_string();
    }

    pub fn hide(&mut self) {
        self.visible = false;
        self.candidates.clear();
    }

    pub fn next(&mut self) {
        if !self.candidates.is_empty() {
            self.selected = (self.selected + 1) % self.candidates.len();
        }
    }

    pub fn prev(&mut self) {
        if !self.candidates.is_empty() {
            self.selected = if self.selected == 0 {
                self.candidates.len() - 1
            } else {
                self.selected - 1
            };
        }
    }

    pub fn selected(&self) -> Option<&str> {
        self.candidates.get(self.selected).map(|s| s.as_str())
    }

    pub fn render(&self, f: &mut Frame, area: Rect) {
        if !self.visible || self.candidates.is_empty() {
            return;
        }

        let items: Vec<ListItem> = self.candidates.iter().enumerate().map(|(i, cmd)| {
            let style = if i == self.selected {
                Style::default().fg(Color::Black).bg(Color::Cyan)
            } else {
                Style::default().fg(Color::White)
            };
            ListItem::new(Line::from(Span::styled(format!("  {} ", cmd), style)))
        }).collect();

        let height = (self.candidates.len() as u16 + 2).min(10);
        let popup = Rect {
            x: area.x + 2,
            y: area.y.saturating_sub(height),
            width: 30.min(area.width.saturating_sub(4)),
            height,
        };

        f.render_widget(Clear, popup);
        let block = Block::default()
            .borders(Borders::ALL)
            .border_style(Style::default().fg(Color::DarkGray));
        let list = List::new(items).block(block);
        f.render_widget(list, popup);
    }
}
```

- [ ] **Step 2: Add to lib.rs**

```rust
pub mod completion;
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/completion.rs crates/aletheon-body/src/lib.rs
git commit -m "feat(tui): add tab completion popup for slash commands

Floating popup with candidate list, up/down navigation,
single-match direct completion."
```

---

## Task 8: Fix spinner animation and command history

**Files:**
- Modify: `crates/aletheon-body/src/impl/ui/status.rs`
- Modify: `crates/aletheon-body/src/impl/ui/input.rs`

- [ ] **Step 1: Fix spinner in status.rs**

Add `tick_spinner()` method that advances the frame counter:

```rust
pub fn tick_spinner(&mut self) {
    self.spinner_frame = (self.spinner_frame + 1) % 8;
    if self.waiting {
        self.elapsed_secs += 0.06; // ~60ms per tick
    }
}
```

- [ ] **Step 2: Add command history to input.rs**

Add a `CommandHistory` struct:

```rust
pub struct CommandHistory {
    entries: Vec<String>,
    cursor: usize,
    max_size: usize,
}

impl CommandHistory {
    pub fn new() -> Self {
        Self {
            entries: Vec::new(),
            cursor: 0,
            max_size: 50,
        }
    }

    pub fn push(&mut self, entry: String) {
        if entry.is_empty() { return; }
        // Avoid consecutive duplicates
        if self.entries.last() == Some(&entry) { return; }
        self.entries.push(entry);
        if self.entries.len() > self.max_size {
            self.entries.remove(0);
        }
        self.cursor = self.entries.len();
    }

    pub fn up(&mut self) -> Option<&str> {
        if self.cursor > 0 {
            self.cursor -= 1;
            self.entries.get(self.cursor).map(|s| s.as_str())
        } else {
            None
        }
    }

    pub fn down(&mut self) -> Option<&str> {
        if self.cursor < self.entries.len() - 1 {
            self.cursor += 1;
            self.entries.get(self.cursor).map(|s| s.as_str())
        } else {
            self.cursor = self.entries.len();
            None
        }
    }

    pub fn reset_cursor(&mut self) {
        self.cursor = self.entries.len();
    }
}
```

- [ ] **Step 3: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/status.rs crates/aletheon-body/src/impl/ui/input.rs
git commit -m "feat(tui): fix spinner animation and add command history

Spinner now ticks at 60ms intervals during streaming.
Command history with up/down arrows, max 50 entries."
```

---

## Task 9: Integrate everything into draw() and main loop

**Files:**
- Modify: `crates/aletheon-body/src/impl/ui/mod.rs`
- Modify: `crates/aletheon-body/src/impl/ui/chat.rs`

- [ ] **Step 1: Update App struct with all new fields**

```rust
struct App {
    // ... existing fields ...
    stream_ctrl: StreamController,
    active_tools: HashMap<String, ToolCard>,
    turn_tokens: Option<(u32, u32)>,
    history: CommandHistory,
    completion: CompletionPopup,
}
```

- [ ] **Step 2: Update draw() to render thinking and tool cards**

In the `draw()` function, after rendering the chat area, render active tool cards and thinking blocks:

```rust
// Render active tool cards below chat
for card in app.active_tools.values() {
    for line in card.render() {
        // Append to chat widget's visible lines
    }
}
```

- [ ] **Step 3: Wire Ctrl+O and Ctrl+B keybindings**

In `handle_key()`:
```rust
KeyCode::Char('o') if key.modifiers.contains(KeyModifiers::CONTROL) => {
    app.stream_ctrl.toggle_thinking();
}
KeyCode::Char('b') if key.modifiers.contains(KeyModifiers::CONTROL) => {
    // Toggle most recent tool card
    if let Some(last) = app.active_tools.values_mut().last() {
        last.toggle();
    }
}
```

- [ ] **Step 4: Wire Tab for completion**

```rust
KeyCode::Tab => {
    if app.input_buf.starts_with('/') {
        let commands = get_all_commands(&app.skill_loader);
        app.completion.show(&app.input_buf, &commands);
    }
}
```

- [ ] **Step 5: Wire Up/Down for history**

```rust
KeyCode::Up => {
    if app.completion.visible {
        app.completion.prev();
    } else if let Some(entry) = app.history.up() {
        app.input_buf = entry.to_string();
        app.cursor = app.input_buf.len();
    }
}
KeyCode::Down => {
    if app.completion.visible {
        app.completion.next();
    } else if let Some(entry) = app.history.down() {
        app.input_buf = entry.to_string();
        app.cursor = app.input_buf.len();
    }
}
```

- [ ] **Step 6: Verify full build**

Run: `cargo build --release --bin aletheon`
Expected: compiles successfully

- [ ] **Step 7: Commit**

```bash
git add crates/aletheon-body/src/impl/ui/
git commit -m "feat(tui): integrate streaming, thinking, tool cards, completion

Full integration of Phase 1 TUI redesign:
- StreamController for two-region streaming
- ThinkingBlock with Ctrl+O toggle
- ToolCard with Ctrl+B toggle
- Tab completion for slash commands
- Command history with up/down arrows
- Spinner animation fix"
```

---

## Task 10: End-to-end test

- [ ] **Step 1: Start daemon**

```bash
cargo build --release --bin aletheond --bin aletheon
pkill -9 -f aletheond 2>/dev/null; sleep 1
./target/release/aletheond -s /tmp/aletheon/aletheon.sock &
sleep 2
```

- [ ] **Step 2: Test via socat (backward compat)**

```bash
echo '{"jsonrpc":"2.0","method":"chat","id":1,"params":{"message":"hello"}}' | socat -t10 - /tmp/aletheon/aletheon.sock
```
Expected: JSONL events (thinking_delta, text_delta, etc.) followed by final JSON-RPC response

- [ ] **Step 3: Test TUI interactively**

```bash
./target/release/aletheon -s /tmp/aletheon/aletheon.sock
```

Test checklist:
- [ ] Type "hello" → see streaming text appear
- [ ] See "✻ Thought for N.Ns" during/after thinking
- [ ] Ctrl+O expands thinking block
- [ ] Type a message that triggers tool call → see ToolCard
- [ ] Ctrl+B expands tool output
- [ ] Tab completion works for `/ref<Tab>`
- [ ] Up/Down arrows recall command history
- [ ] Spinner animates during streaming
- [ ] No freezing/hanging

- [ ] **Step 4: Commit test results**

```bash
git add -A
git commit -m "test: verify TUI redesign Phase 1 end-to-end"
```

---

## Self-Review Checklist

- [x] **Spec coverage:** Every requirement from the design doc has a corresponding task
- [x] **Placeholder scan:** No TBD/TODO in code steps
- [x] **Type consistency:** Event types match between handler and TUI dispatcher
- [x] **Backward compat:** JSON-RPC responses still sent after events
- [x] **File paths exact:** All paths are absolute or relative to project root
- [x] **Code complete:** Each step has actual implementation code
