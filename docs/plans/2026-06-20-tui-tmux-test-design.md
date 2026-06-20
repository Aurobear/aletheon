# TUI tmux-based Test Framework

## Problem

Current test mode embeds TestBackend/FrameRecorder/EventRecorder directly into `ui/mod.rs` (2000+ lines). This approach:
- Pollutes production code with `if is_test_mode` branches
- Cannot verify real terminal rendering
- Event recording through daemon's `notify_tx` is unreliable (shared channels, lost events)
- `turn_active` / `streaming` dual-state is bug-prone
- Each fix adds more complexity to an already overloaded file

## Solution

**tmux-based integration testing** — drive the real TUI in a real terminal via tmux. Zero changes to production code.

## Architecture

```
scripts/test_tui_tmux.sh              ← Main test runner
tests/tui_tmux/
├── lib.sh                             ← Test library (tui_start/send/wait/assert)
├── test_basic_response.sh
├── test_tool_call.sh
├── test_rapid_fire.sh
├── test_thinking_block.sh
├── test_long_output.sh
├── test_error_recovery.sh
└── test_ctrl_shortcuts.sh
```

## Core API (lib.sh)

### Setup/Teardown
- `tui_start` — Start daemon, create tmux session, launch TUI
- `tui_stop` — Kill TUI, daemon, destroy tmux session

### Input
- `tui_send "text"` — Type text into TUI
- `tui_key <key>` — Send special key (Enter, Esc, Tab, Ctrl+C, etc.)
- `tui_submit "text"` — Send text + Enter in one call

### Verification
- `tui_wait "pattern" [timeout_secs]` — Poll `tmux capture-pane` until pattern appears
- `tui_capture` — Dump current screen to stdout
- `tui_assert "pattern"` — Fail if pattern not on screen
- `tui_refute "pattern"` — Fail if pattern IS on screen

### Implementation Details
- tmux session name: `aletheon-test-$$` (PID-based, unique per run)
- tmux size: 120x40 (matches current test config)
- Polling interval: 1s (for `tui_wait`)
- Default timeout: 60s
- Cleanup via trap on EXIT

## Test Scenarios

| Scenario | Input | Verify |
|---|---|---|
| basic_response | "hello" + Enter | Response text appears |
| tool_call | "列出当前目录的文件" + Enter | Tool card appears, file list shown |
| thinking_block | "用一句话解释递归" + Enter | Response appears |
| rapid_fire | 5 messages, auto-submit | All 5 responses appear |
| long_output | "写一个HTTP server" + Enter | Code block appears |
| error_recovery | "读取 /tmp/nonexistent" + Enter | Error handled gracefully |
| ctrl_shortcuts | Ctrl+C, Ctrl+L, etc. | UI responds to shortcuts |

## What Gets Removed

From `ui/mod.rs`:
- `TestConfig` struct
- `FrameRecorder` / `EventRecorder` / `TestInputReader`
- `test_input` / `record_frames` / `record_events` / `auto_submit` / `test_timeout` flags
- `is_test_mode` branches in `run_app()`
- `turn_active` field (revert to `streaming` only)
- `draw_with_recorder()` wrapper

From CLI:
- `--test-input`, `--record-frames`, `--record-events`, `--auto-submit`, `--test-timeout` flags

From scripts:
- `scripts/test_tui_integration.sh` (replaced by tmux version)
- `scripts/verify_tui_test.sh`

## What Gets Kept

- `tests/tui_scenarios/*.txt` — input files (used by tmux tests)
- daemon JSON-RPC protocol — unchanged
