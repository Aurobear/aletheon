> New document — code paths reflect aletheon-* crate structure

# UI Subsystem (TUI)

> Terminal user interface — chat, commands, computer view, event handling, input, markdown rendering, skills, status.

**Crate:** `aletheon-body`
**Module:** `crates/aletheon-body/src/impl/ui/`
**Last updated:** 2026-06-14

---

## Implementation Status

| Component | Status | Code Location | Notes |
|-----------|--------|---------------|-------|
| TUI entry point | ✅ Implemented | `ui/mod.rs` | `run()` — full TUI with raw mode, alternate screen |
| ChatWidget | ✅ Implemented | `ui/chat.rs` | Chat message display and scrolling |
| Command parser | ✅ Implemented | `ui/command.rs` | `/help`, `/clear`, `/quit`, `/status`, skill commands |
| Computer view | ✅ Implemented | `ui/computer.rs` | Screenshot + accessibility tree view (feature-gated) |
| Event handling | ✅ Implemented | `ui/event.rs` | Keyboard, mouse, paste, resize events |
| Input handling | ✅ Implemented | `ui/input.rs` | Text input with cursor, CJK support |
| Markdown rendering | ✅ Implemented | `ui/markdown.rs` | Terminal markdown rendering |
| Skill system | ✅ Implemented | `ui/skill.rs` | SkillLoader — load and invoke skills |
| Status bar | ✅ Implemented | `ui/status.rs` | Connection status, model name, streaming indicator |
| Terminal compat | ✅ Implemented | `ui/term_compat.rs` | TermCaps — terminal capability detection |

---

## 1. Architecture

The TUI is a ratatui-based terminal application connecting to the aletheond daemon via Unix socket (JSON-RPC 2.0).

```
aletheon-cli (TUI)
    ├── ratatui + crossterm
    ├── UnixStream → aletheond
    └── Modules:
        ├── chat.rs      — ChatWidget (message history, scrolling)
        ├── command.rs    — /command parsing
        ├── computer.rs   — Screenshot + a11y view (feature-gated)
        ├── event.rs      — Event dispatch
        ├── input.rs      — Text input with cursor
        ├── markdown.rs   — Terminal markdown
        ├── skill.rs      — Skill loading and invocation
        ├── status.rs     — Status bar
        └── term_compat.rs — Terminal capability detection
```

## 2. Chat Module

**ChatWidget** — displays conversation history with role-based styling:
- `Role::User` — user messages
- `Role::Assistant` — agent responses (streaming support)
- `Role::System` — system messages (errors, status)
- Scrolling via PageUp/PageDown
- Width-aware text wrapping

## 3. Command System

Slash commands parsed by `command.rs`:

| Command | Action |
|---------|--------|
| `/help` | Show help text |
| `/clear` | Clear chat history |
| `/quit` | Exit TUI |
| `/status` | Query daemon status via JSON-RPC |
| `/<skill> [args]` | Invoke a loaded skill |

Commands are dispatched before regular chat messages. Unknown commands show an error.

## 4. Computer View

**Feature gate:** `input + display + a11y` (all three required)

Provides a visual view of the desktop:
- Screenshot capture via display driver
- Accessibility tree overlay via a11y driver
- Element highlighting and selection

## 5. Event Handling

The TUI uses crossterm's event system:

| Event | Handling |
|-------|----------|
| Key press | Dispatched to `handle_key()` |
| Paste | Insert text at cursor position |
| Resize | Update chat widget width |
| Focus change | Track terminal focus state |

### 5.1 Key Bindings

| Key | Action |
|-----|--------|
| Enter | Submit message (with CJK delay) |
| Shift+Enter / Alt+Enter | Insert newline |
| `\` + Enter | Insert newline (continuation) |
| Ctrl+C | Clear input (double-press to quit) |
| Ctrl+D | Quit (when input empty) |
| Ctrl+L | Clear screen |
| Esc | Clear input |
| Left/Right | Move cursor |
| Home/End | Jump to start/end |
| PageUp/PageDown | Scroll chat |
| Backspace/Delete | Delete character |

## 6. Input Handling

**CJK-aware input:**
- Detects CJK characters (Unicode ranges: CJK Unified, Extension A, Symbols, Fullwidth, Hangul, Hiragana, Katakana)
- When CJK detected, Enter submission is delayed by 100ms to allow IME composition to finish
- "double-defer" pattern adapted from OpenCode

**Cursor management:**
- Byte-index cursor position
- Character-aware movement (multi-byte UTF-8 support)
- Visual cursor display with reverse video

## 7. Markdown Rendering

Terminal markdown rendering with ratatui:
- Headers, bold, italic, code blocks
- Syntax-aware formatting
- Terminal capability detection (unicode vs ASCII fallback)

## 8. Skill System

**SkillLoader** — loads skill definitions from a directory:
- Skills are markdown files with metadata
- `/<skill-name> [args]` invocation
- Skill content is sent to the daemon as a chat message

## 9. Status Bar

**StatusBar** — single-line status display:
- Connection status (connected/disconnected)
- Model name
- Streaming indicator (spinner)
- Terminal capability-aware (unicode vs ASCII)

## 10. Terminal Compatibility

**TermCaps** — detects terminal capabilities:
- Unicode support (box-drawing characters)
- Color depth (16-color, 256-color, truecolor)
- ASCII fallback for limited terminals

## 11. Non-TTY Mode

When stdin is not a TTY (piped input), the TUI falls back to simple line mode:
- Plain text input/output
- No raw mode, no alternate screen
- Same JSON-RPC protocol to daemon

## 12. Implementation Notes

**Code location:** `crates/aletheon-body/src/impl/ui/` (11 files)

**Key design decisions:**
- ratatui + crossterm for cross-platform terminal rendering
- Unix socket JSON-RPC for daemon communication
- CJK-aware input with IME composition delay
- Feature-gated computer view (requires all three: input, display, a11y)
- Simple line mode fallback for non-TTY environments
