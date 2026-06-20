# TUI Integration Test Framework + Model Naming Fix

**Date**: 2026-06-19
**Status**: Draft
**Scope**: Real daemon + real TUI testing with log capture and automated verification

## 1. Problem

1. TUI streaming features (thinking blocks, tool cards, Ctrl+O/B) have no automated tests
2. Model naming in `config.toml` uses `deepseek/deepseek-v4-flash` but should be `deepseek-v4-flash`
3. No way to reproduce TUI bugs — testing is manual and ad-hoc

## 2. Model Naming Fix

**Change**: `config.toml` — remove `provider/` prefix from model_routing and aliases.

Before:
```toml
[model_routing]
default = "deepseek/deepseek-v4-flash"
reasoning = "deepseek/deepseek-v4-pro[1m]"
```

After:
```toml
[model_routing]
default = "deepseek-v4-flash"
reasoning = "deepseek-v4-pro[1m]"
```

**Why this works**: `ProviderRegistry::resolve()` at `provider_registry.rs:122-128` already supports bare model names — it falls back to `default_provider`.

**Files changed**: `~/.aletheon/config.toml` only.

## 3. TUI Test Framework

### 3.1 Architecture

```
scripts/test_tui_integration.sh (orchestrator)
  │
  ├── Phase 1: Start daemon (real config, deepseek-v4-flash)
  │     └── daemon → socket + daemon.log
  │
  ├── Phase 2: Launch TUI with test flags
  │     └── TUI --test-input scenarios.txt --record-frames frames.jsonl --record-events events.jsonl
  │
  ├── Phase 3: Wait for TUI exit
  │
  ├── Phase 4: Collect artifacts
  │     ├── daemon.log
  │     ├── frames.jsonl
  │     └── events.jsonl
  │
  └── Phase 5: Verify + report
        ├── Check frames.jsonl for expected widgets
        ├── Check events.jsonl for correct event sequence
        ├── Check daemon.log for ERROR/WARN
        └── Check exit code (non-zero = panic/crash)
```

### 3.2 TUI New CLI Flags

| Flag | Type | Description |
|------|------|-------------|
| `--test-input <file>` | path | Read inputs from file, one per line. Auto-submit each line after previous response completes. |
| `--record-frames <file>` | path | After each render, write a JSON snapshot to this file. |
| `--record-events <file>` | path | Write every daemon→TUI event as JSONL to this file. |
| `--auto-submit` | flag | Auto-submit each line from `--test-input` (no Enter key needed). |
| `--test-timeout <secs>` | int | Exit after N seconds if test hasn't completed. Default: 120. |

**Implementation location**: `crates/binaries/aletheon-cli/src/main.rs` — add CLI args, wire into App.

### 3.3 Frame Recording

After each `app.tick()` / render cycle, serialize the visible buffer:

```rust
// In ui/mod.rs main loop, after render:
if let Some(recorder) = &mut frame_recorder {
    let buffer = f.buffer_mut();  // ratatui Buffer
    let snapshot = FrameSnapshot {
        ts: now_ms(),
        cols: buffer.area.width,
        rows: buffer.area.height,
        content: buffer_to_text(buffer),  // extract visible text
        thinking_visible: app.stream_ctrl.thinking_active(),
        tool_count: app.active_tools.len(),
        cursor_pos: (cursor.col, cursor.row),
    };
    recorder.write(snapshot);
}
```

### 3.4 Event Recording

Wrap the event handler to log every event:

```rust
fn handle_event_with_recording(app: &mut App, params: &Value, recorder: &mut EventRecorder) {
    let event_type = params.get("type").and_then(|v| v.as_str()).unwrap_or("");
    recorder.write(json!({
        "ts": now_ms(),
        "type": event_type,
        "params": params,
    }));
    handle_event(app, params);  // existing handler
}
```

### 3.5 Test Scenarios

Each scenario is a file with inputs and expected outcomes:

```
# scenarios/basic_response.txt
hello
---
expect: text_delta present
expect: turn_done present
expect: no panic
expect: frame contains "hello" or response text
```

**Scenarios**:

| # | File | Input | Verification |
|---|------|-------|-------------|
| 1 | `basic_response.txt` | `hello` | text_delta events, non-empty response, clean exit |
| 2 | `tool_call.txt` | `列出当前目录文件` | tool_call_start + tool_call_result events, tool card in frames |
| 3 | `thinking_block.txt` | `分析 Rust vs Go 的并发模型` | thinking_delta events, thinking block in frames |
| 4 | `rapid_fire.txt` | 5x `test N` quickly | No panic, all responses received |
| 5 | `long_output.txt` | `写一个完整的 HTTP server in Python` | Truncation/scrolling works, no crash |
| 6 | `error_recovery.txt` | `读取 /tmp/nonexistent_xyz_999` | Error event, error display, no hang |
| 7 | `ctrl_shortcuts.txt` | `hello` + simulated Ctrl+O, Ctrl+B | State toggles visible in frames |

### 3.6 Verification Script

```bash
# scripts/verify_tui_test.sh
# Usage: ./scripts/verify_tui_test.sh frames.jsonl events.jsonl daemon.log

verify_frame_contains() {
    local pattern="$1"
    if jq -r '.content' "$FRAMES" | grep -q "$pattern"; then
        pass "Frame contains: $pattern"
    else
        fail "Frame missing: $pattern"
    fi
}

verify_event_sequence() {
    local events=("turn_start" "text_delta" "turn_done")
    local last_idx=-1
    for ev in "${events[@]}"; do
        local idx=$(grep -n "\"type\":\"$ev\"" "$EVENTS" | head -1 | cut -d: -f1)
        if [[ -z "$idx" ]]; then
            fail "Event missing: $ev"
        elif [[ $idx -le $last_idx ]]; then
            fail "Event out of order: $ev"
        else
            pass "Event found: $ev"
            last_idx=$idx
        fi
    done
}

verify_no_panic() {
    if grep -qi "panic\|thread.*panicked" "$DAEMON_LOG"; then
        fail "Daemon panic detected"
    else
        pass "No daemon panic"
    fi
}
```

### 3.7 Output Artifacts

All test artifacts go to `/tmp/aletheon-tui-test-<timestamp>/`:

```
/tmp/aletheon-tui-test-20260619-1530/
├── daemon.log              # daemon stderr/stdout
├── frames.jsonl            # rendered frame snapshots
├── events.jsonl            # daemon→TUI events
├── scenario_results.txt    # pass/fail per scenario
└── summary.txt             # final test summary
```

## 4. Implementation Plan

### Task 1: Fix model naming in config.toml
- File: `~/.aletheon/config.toml`
- Change: Remove `deepseek/` prefix from model_routing entries

### Task 2: Add CLI flags to aletheon-cli
- File: `crates/binaries/aletheon-cli/src/main.rs`
- Add: `--test-input`, `--record-frames`, `--record-events`, `--auto-submit`, `--test-timeout`

### Task 3: Add frame recording to TUI
- File: `crates/aletheon-body/src/impl/ui/mod.rs`
- Add: `FrameRecorder` struct, write after each render

### Task 4: Add event recording to TUI
- File: `crates/aletheon-body/src/impl/ui/mod.rs`
- Add: `EventRecorder` struct, wrap `handle_event()`

### Task 5: Implement test input reader
- File: `crates/aletheon-body/src/impl/ui/input.rs`
- Add: `TestInputReader` that reads from file, auto-submits

### Task 6: Create test scenarios
- Files: `tests/tui_scenarios/*.txt`
- 7 scenarios as described in 3.5

### Task 7: Create orchestrator script
- File: `scripts/test_tui_integration.sh`
- Starts daemon, runs TUI with test flags, collects artifacts

### Task 8: Create verification script
- File: `scripts/verify_tui_test.sh`
- Reads artifacts, runs assertions, generates report

## 5. File Changes Summary

| File | Change |
|------|--------|
| `~/.aletheon/config.toml` | Fix model_routing naming |
| `crates/binaries/aletheon-cli/src/main.rs` | Add test CLI flags |
| `crates/aletheon-body/src/impl/ui/mod.rs` | FrameRecorder + EventRecorder |
| `crates/aletheon-body/src/impl/ui/input.rs` | TestInputReader |
| `tests/tui_scenarios/*.txt` | Test scenario files (7) |
| `scripts/test_tui_integration.sh` | Orchestrator script |
| `scripts/verify_tui_test.sh` | Verification script |

## 6. Success Criteria

1. `./scripts/test_tui_integration.sh` runs all 7 scenarios
2. Each scenario produces frames.jsonl + events.jsonl
3. `./scripts/verify_tui_test.sh` validates all artifacts
4. All scenarios pass (no panic, correct event sequence, expected widgets visible)
5. Model naming uses bare `deepseek-v4-flash` in routing config
